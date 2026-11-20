# Annotate installer for Windows.
#
#   powershell -c "irm https://raw.githubusercontent.com/vcsoc/annotate/HEAD/install.ps1 | iex"
#
# Options (environment variables, or parameters when run as a script block):
#   -Version vX.Y.Z      $env:ANNOTATE_VERSION        release tag (default: latest)
#   -Source              $env:ANNOTATE_FROM_SOURCE=1  build from source with Rust instead of a release
#   -Integrations a,b    $env:ANNOTATE_INTEGRATIONS   also install integrations (pi, claude-code, codex,
#                                                     opencode, gemini, claude-desktop, cursor, agent-skills)
#   -Uninstall                                        remove Annotate (keeps ~\.annotate and saved batches)
# Example with options:
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/vcsoc/annotate/HEAD/install.ps1))) -Integrations pi
param(
  [string]$Version = $(if ($env:ANNOTATE_VERSION) { $env:ANNOTATE_VERSION } else { 'latest' }),
  [switch]$Source = ($env:ANNOTATE_FROM_SOURCE -eq '1'),
  [string]$Integrations = $env:ANNOTATE_INTEGRATIONS,
  [switch]$Uninstall
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repo = if ($env:ANNOTATE_REPO) { $env:ANNOTATE_REPO } else { 'vcsoc/annotate' }
$InstallDir = Join-Path $env:LOCALAPPDATA 'Annotate'
$Exe = Join-Path $InstallDir 'annotate.exe'
$Tmp = Join-Path ([IO.Path]::GetTempPath()) ("annotate-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force -Path $Tmp | Out-Null

function Say($m) { Write-Host "==> $m" -ForegroundColor Cyan }
function Ok($m) { Write-Host "✓ $m" -ForegroundColor Green }
function Warn($m) { Write-Host "! $m" -ForegroundColor Yellow }
function Have($c) { [bool](Get-Command $c -ErrorAction SilentlyContinue) }
function Refresh-Path { $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User') + ';' + (Join-Path $env:USERPROFILE '.cargo\bin') }
function Add-UserPath($dir) {
  $user = [Environment]::GetEnvironmentVariable('Path', 'User')
  if (-not (($user -split ';') -contains $dir)) { [Environment]::SetEnvironmentVariable('Path', ($user.TrimEnd(';') + ';' + $dir).TrimStart(';'), 'User') }
  if (-not (($env:Path -split ';') -contains $dir)) { $env:Path += ";$dir" }
}
function Stop-Annotate { if (Test-Path $Exe) { & $Exe quit 2>$null | Out-Null }; Get-Process annotate -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue }

try {
  if ($Uninstall) {
    Stop-Annotate
    $uninstaller = Join-Path $InstallDir 'uninstall.exe'
    if (Test-Path $uninstaller) { Start-Process $uninstaller -ArgumentList '/S' -Wait } else { Remove-Item -Recurse -Force $InstallDir -ErrorAction SilentlyContinue }
    Remove-Item -Force (Join-Path ([Environment]::GetFolderPath('Programs')) 'Annotate.lnk') -ErrorAction SilentlyContinue
    Ok 'Annotate removed. Your settings, ~\.annotate and saved .annotate\ batches were kept.'
    return
  }

  Say "Installing Annotate for Windows ($env:PROCESSOR_ARCHITECTURE)"
  Stop-Annotate
  $installed = $false

  if (-not $Source) {
    try {
      $api = if ($Version -eq 'latest') { "https://api.github.com/repos/$Repo/releases/latest" } else { "https://api.github.com/repos/$Repo/releases/tags/$Version" }
      $release = Invoke-RestMethod -Uri $api -Headers @{ 'User-Agent' = 'annotate-installer' }
      $asset = $release.assets | Where-Object { $_.name -match '_x64-setup\.exe$' } | Select-Object -First 1
      if ($asset) {
        Say "Downloading $($asset.name)"
        $setup = Join-Path $Tmp $asset.name
        Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $setup -UseBasicParsing
        # Per-user NSIS installer: no admin rights, installs to %LOCALAPPDATA%\Annotate.
        Start-Process $setup -ArgumentList '/S', "/D=$InstallDir" -Wait
        $installed = Test-Path $Exe
      }
    } catch { }
    if (-not $installed) { Warn "No prebuilt release found ($Version) — building from source instead." }
  }

  if (-not $installed) {
    # Build tools: MSVC C++ build tools and WebView2 are Tauri's prerequisites.
    $programs = if (${env:ProgramFiles(x86)}) { ${env:ProgramFiles(x86)} } else { 'C:\Program Files (x86)' }
    $vswhere = Join-Path $programs 'Microsoft Visual Studio\Installer\vswhere.exe'
    $hasMsvc = (Test-Path $vswhere) -and (& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
    if (-not $hasMsvc) {
      if (-not (Have winget)) { throw 'Install "Visual Studio Build Tools" with the "Desktop development with C++" workload, then run this again.' }
      Say 'Installing Visual Studio Build Tools (C++) — this can take a while'
      winget install --id Microsoft.VisualStudio.2022.BuildTools -e --silent --accept-package-agreements --accept-source-agreements --override '--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
    }
    if (-not (Have git)) {
      if (-not (Have winget)) { throw 'Install Git from https://git-scm.com, then run this again.' }
      Say 'Installing Git'
      winget install --id Git.Git -e --silent --accept-package-agreements --accept-source-agreements
      Refresh-Path
    }
    Refresh-Path
    if (-not (Have cargo)) {
      Say 'Installing Rust (rustup, for your user only)'
      $rustup = Join-Path $Tmp 'rustup-init.exe'
      Invoke-WebRequest -Uri 'https://win.rustup.rs/x86_64' -OutFile $rustup -UseBasicParsing
      & $rustup -y --profile minimal | Out-Host
      Refresh-Path
    }
    $src = if ($env:ANNOTATE_SRC) { $env:ANNOTATE_SRC } else { Join-Path $env:USERPROFILE '.annotate\src' }
    if (-not $env:ANNOTATE_SRC) {
      if (Test-Path (Join-Path $src '.git')) {
        Say "Updating the source in $src"
        git -C $src fetch --quiet --tags origin
        if ($Version -ne 'latest') { git -C $src checkout --quiet $Version } else { git -C $src checkout --quiet FETCH_HEAD }
      } else {
        Say "Downloading the source to $src"
        New-Item -ItemType Directory -Force -Path (Split-Path $src) | Out-Null
        if ($Version -ne 'latest') { git clone --quiet --depth 1 --branch $Version "https://github.com/$Repo.git" $src } else { git clone --quiet --depth 1 "https://github.com/$Repo.git" $src }
      }
    }
    Say 'Building Annotate (Rust + Tauri) — the first build takes a few minutes'
    Push-Location (Join-Path $src 'src-tauri')
    try { cargo build --release; if ($LASTEXITCODE) { throw 'Build failed' } } finally { Pop-Location }
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Copy-Item -Force (Join-Path $src 'src-tauri\target\release\annotate.exe') $Exe
    # Start menu shortcut.
    $shell = New-Object -ComObject WScript.Shell
    $link = $shell.CreateShortcut((Join-Path ([Environment]::GetFolderPath('Programs')) 'Annotate.lnk'))
    $link.TargetPath = $Exe; $link.WorkingDirectory = $InstallDir; $link.Description = 'Annotated screenshots for any AI harness'; $link.Save()
  }

  if (-not (Test-Path $Exe)) { throw "Installation failed: $Exe is missing" }
  Add-UserPath $InstallDir
  Ok "Installed $(& $Exe --version) → $Exe"

  if ($Integrations) {
    foreach ($id in ($Integrations -split '[,\s]+' | Where-Object { $_ })) {
      & $Exe integrations install $id
      if ($LASTEXITCODE -eq 0) { Ok "Integration: $id" } else { Warn "Integration $id could not be installed" }
    }
  }

  Write-Host ''
  Write-Host 'Next steps' -ForegroundColor White
  Write-Host '  • Open Annotate from the Start menu, or run:  annotate   (open a new terminal first)'
  Write-Host '  • Connect a harness:   annotate integrations install pi     (or claude-code, codex, opencode, …)'
  Write-Host '  • List destinations:   annotate harnesses'
  Write-Host "  • Tutorial:            https://github.com/$Repo/blob/HEAD/docs/tutorial.html"
} catch {
  # No `exit`: under `irm | iex` it would close the user's PowerShell window.
  Write-Host "✗ $($_.Exception.Message)" -ForegroundColor Red
  $global:LASTEXITCODE = 1
} finally {
  Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
