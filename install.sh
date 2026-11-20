#!/bin/sh
# Annotate installer for macOS and Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/vcsoc/annotate/HEAD/install.sh | sh
#
# Options (append after `sh -s --`, or set the matching environment variable):
#   --version vX.Y.Z        ANNOTATE_VERSION       release tag to install (default: latest)
#   --source                ANNOTATE_FROM_SOURCE=1 build from source with Rust instead of a release
#   --integrations a,b      ANNOTATE_INTEGRATIONS  also install integrations (pi, claude-code, codex,
#                                                  opencode, gemini, claude-desktop, cursor, agent-skills)
#   --bin-dir DIR           ANNOTATE_BIN_DIR       where the `annotate` command goes (default ~/.local/bin)
#   --uninstall                                    remove Annotate (keeps ~/.annotate and saved batches)
#   --yes                   ANNOTATE_YES=1         do not ask before installing system packages
# Example:
#   curl -fsSL https://raw.githubusercontent.com/vcsoc/annotate/HEAD/install.sh | sh -s -- --integrations pi
set -eu

REPO="${ANNOTATE_REPO:-vcsoc/annotate}"
VERSION="${ANNOTATE_VERSION:-latest}"
FROM_SOURCE="${ANNOTATE_FROM_SOURCE:-0}"
INTEGRATIONS="${ANNOTATE_INTEGRATIONS:-}"
BIN_DIR="${ANNOTATE_BIN_DIR:-$HOME/.local/bin}"
YES="${ANNOTATE_YES:-0}"
UNINSTALL=0
# Testing aids: use an existing checkout / install the app elsewhere.
SRC_DIR="${ANNOTATE_SRC:-}"
APP_DIR="${ANNOTATE_APP_DIR:-}"

while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --source) FROM_SOURCE=1; shift ;;
    --integrations) INTEGRATIONS="$2"; shift 2 ;;
    --bin-dir) BIN_DIR="$2"; shift 2 ;;
    --uninstall) UNINSTALL=1; shift ;;
    --yes|-y) YES=1; shift ;;
    -h|--help) echo "See https://github.com/$REPO#install for the options."; exit 0 ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

if [ -t 1 ]; then B="$(printf '\033[1m')"; G="$(printf '\033[32m')"; Y="$(printf '\033[33m')"; R="$(printf '\033[31m')"; N="$(printf '\033[0m')"; else B=''; G=''; Y=''; R=''; N=''; fi
say()  { printf '%s==>%s %s\n' "$B" "$N" "$*"; }
ok()   { printf '%s✓%s %s\n' "$G" "$N" "$*"; }
warn() { printf '%s!%s %s\n' "$Y" "$N" "$*" >&2; }
die()  { printf '%s✗ %s%s\n' "$R" "$*" "$N" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

OS="$(uname -s)"; ARCH="$(uname -m)"
case "$OS" in
  Darwin) PLATFORM=macos ;;
  Linux) PLATFORM=linux ;;
  *) die "Unsupported OS: $OS. On Windows run: powershell -c \"irm https://raw.githubusercontent.com/$REPO/HEAD/install.ps1 | iex\"" ;;
esac
case "$ARCH" in x86_64|amd64) ARCH=x86_64 ;; arm64|aarch64) ARCH=aarch64 ;; *) die "Unsupported CPU: $ARCH" ;; esac

if [ "$PLATFORM" = macos ]; then
  if [ -z "$APP_DIR" ]; then
    if [ -w /Applications ]; then APP_DIR=/Applications; else APP_DIR="$HOME/Applications"; fi
  fi
  APP="$APP_DIR/Annotate.app"
  EXE="$APP/Contents/MacOS/annotate"
else
  APP_DIR="${APP_DIR:-$HOME/.local/share/annotate}"
  EXE="$APP_DIR/annotate"
fi

TMP="$(mktemp -d 2>/dev/null || mktemp -d -t annotate)"
cleanup() {
  if [ -n "${MOUNT:-}" ]; then hdiutil detach -quiet "$MOUNT" >/dev/null 2>&1 || true; fi
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

download() { # url file
  if have curl; then curl -fsSL --retry 2 -o "$2" "$1"; elif have wget; then wget -qO "$2" "$1"; else die "curl or wget is required"; fi
}
fetch() { if have curl; then curl -fsSL "$1"; else wget -qO- "$1"; fi; }

sudo_run() {
  if [ "$(id -u)" = 0 ]; then "$@"; elif have sudo; then sudo "$@"; else die "Run as root or install sudo to continue: $*"; fi
}
confirm() { # question
  [ "$YES" = 1 ] && return 0
  if [ -r /dev/tty ]; then printf '%s [Y/n] ' "$1" >/dev/tty; read -r answer </dev/tty || answer=; case "$answer" in n*|N*) return 1 ;; esac; return 0; fi
  return 0
}

# ------------------------------------------------------------------ uninstall
if [ "$UNINSTALL" = 1 ]; then
  have annotate && annotate quit >/dev/null 2>&1 || true
  if [ "$PLATFORM" = macos ]; then rm -rf "$APP"; else rm -rf "$APP_DIR" "$HOME/.local/share/applications/annotate.desktop" "$HOME/.local/share/icons/hicolor/128x128/apps/annotate.png"; fi
  rm -f "$BIN_DIR/annotate"
  ok "Annotate removed. Your settings, ~/.annotate and saved .annotate/ batches were kept."
  echo "  Remove integrations first next time with: annotate integrations uninstall <id>"
  exit 0
fi

# ------------------------------------------------------------------ prebuilt release
release_asset() { # pattern -> download URL of the first matching asset
  if [ "$VERSION" = latest ]; then api="https://api.github.com/repos/$REPO/releases/latest"; else api="https://api.github.com/repos/$REPO/releases/tags/$VERSION"; fi
  fetch "$api" 2>/dev/null | grep -o '"browser_download_url": *"[^"]*"' | sed 's/.*"\(http[^"]*\)"/\1/' | grep -E "$1" | head -n 1
}

install_release_macos() {
  url="$(release_asset '_(universal|aarch64|x64)\.dmg$' || true)"
  [ -n "$url" ] || return 1
  say "Downloading $(basename "$url")"
  download "$url" "$TMP/Annotate.dmg"
  MOUNT="$TMP/mnt"; mkdir -p "$MOUNT"
  hdiutil attach -quiet -nobrowse -readonly -mountpoint "$MOUNT" "$TMP/Annotate.dmg"
  mkdir -p "$APP_DIR"; rm -rf "$APP"
  cp -R "$MOUNT/Annotate.app" "$APP"
  hdiutil detach -quiet "$MOUNT"; MOUNT=
  xattr -dr com.apple.quarantine "$APP" 2>/dev/null || true
}

install_release_linux() {
  [ "$ARCH" = x86_64 ] || return 1
  url="$(release_asset '_amd64\.AppImage$' || true)"
  [ -n "$url" ] || return 1
  say "Downloading $(basename "$url")"
  mkdir -p "$APP_DIR"
  download "$url" "$TMP/Annotate.AppImage"
  install -m 755 "$TMP/Annotate.AppImage" "$EXE"
}

# ------------------------------------------------------------------ build from source
linux_deps() {
  if have pkg-config && pkg-config --exists webkit2gtk-4.1 gtk+-3.0 libpipewire-0.3 dbus-1 xcb xrandr gbm 2>/dev/null; then return 0; fi
  if have apt-get; then
    set -- apt-get install -y build-essential pkg-config curl git libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libxdo-dev libssl-dev libxcb1-dev libxrandr-dev libdbus-1-dev libpipewire-0.3-dev libwayland-dev libegl-dev libgbm-dev libclang-dev
    pre=1
  elif have dnf; then
    set -- dnf install -y gcc gcc-c++ make pkgconf-pkg-config curl git webkit2gtk4.1-devel gtk3-devel librsvg2-devel libxdo-devel openssl-devel libxcb-devel libXrandr-devel dbus-devel pipewire-devel wayland-devel mesa-libEGL-devel mesa-libgbm-devel clang-devel
    pre=
  elif have pacman; then
    set -- pacman -S --needed --noconfirm base-devel pkgconf curl git webkit2gtk-4.1 gtk3 librsvg xdotool openssl libxcb libxrandr dbus pipewire wayland libglvnd mesa clang
    pre=
  elif have zypper; then
    set -- zypper install -y gcc gcc-c++ make pkg-config curl git webkit2gtk3-devel gtk3-devel librsvg-devel xdotool-devel libopenssl-devel libxcb-devel libXrandr-devel dbus-1-devel pipewire-devel wayland-devel Mesa-libEGL-devel libgbm-devel clang-devel
    pre=
  else
    die "Install the Tauri system libraries for your distribution (see https://tauri.app/start/prerequisites/), then run this again."
  fi
  say "Annotate needs these system packages to build: $*"
  confirm "Install them now with sudo?" || die "Cancelled. Install the packages above, then run this again."
  if [ -n "$pre" ]; then sudo_run apt-get update; fi
  sudo_run "$@"
}

ensure_rust() {
  # shellcheck disable=SC1091
  [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
  if have cargo; then return 0; fi
  say "Installing Rust (rustup, for your user only)"
  fetch https://sh.rustup.rs | sh -s -- -y --profile minimal
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
}

install_source() {
  if [ "$PLATFORM" = macos ]; then
    xcode-select -p >/dev/null 2>&1 || { xcode-select --install >/dev/null 2>&1 || true; die "Install the Xcode Command Line Tools (a dialog just opened), then run this again."; }
  else
    linux_deps
  fi
  have git || die "git is required"
  ensure_rust
  if [ -n "$SRC_DIR" ]; then
    src="$SRC_DIR"
  else
    src="$HOME/.annotate/src"
    ref=""; [ "$VERSION" != latest ] && ref="$VERSION"
    if [ -d "$src/.git" ]; then
      say "Updating the source in $src"
      git -C "$src" fetch --quiet --tags origin
      if [ -n "$ref" ]; then git -C "$src" checkout --quiet "$ref"; else git -C "$src" checkout --quiet FETCH_HEAD 2>/dev/null || git -C "$src" pull --quiet --ff-only; fi
    else
      say "Downloading the source to $src"
      mkdir -p "$(dirname "$src")"
      if [ -n "$ref" ]; then git clone --quiet --depth 1 --branch "$ref" "https://github.com/$REPO.git" "$src"; else git clone --quiet --depth 1 "https://github.com/$REPO.git" "$src"; fi
    fi
  fi
  say "Building Annotate (Rust + Tauri) — the first build takes a few minutes"
  if [ "$PLATFORM" = macos ]; then
    if ! cargo tauri --version >/dev/null 2>&1; then
      say "Installing the Tauri bundler (cargo install tauri-cli)"
      cargo install tauri-cli --version "^2" --locked --quiet
    fi
    (cd "$src/src-tauri" && cargo tauri build --bundles app) || die "Build failed"
    mkdir -p "$APP_DIR"; rm -rf "$APP"
    cp -R "$src/src-tauri/target/release/bundle/macos/Annotate.app" "$APP"
  else
    (cd "$src/src-tauri" && cargo build --release --locked) || die "Build failed"
    mkdir -p "$APP_DIR"
    install -m 755 "$src/src-tauri/target/release/annotate" "$EXE"
    mkdir -p "$HOME/.local/share/icons/hicolor/128x128/apps"
    cp "$src/src-tauri/icons/128x128.png" "$HOME/.local/share/icons/hicolor/128x128/apps/annotate.png"
  fi
}

# ------------------------------------------------------------------ install
say "Installing Annotate for $PLATFORM ($ARCH)"
have annotate && annotate quit >/dev/null 2>&1 || true
installed=0
if [ "$FROM_SOURCE" != 1 ]; then
  if [ "$PLATFORM" = macos ]; then install_release_macos && installed=1 || true; else install_release_linux && installed=1 || true; fi
  [ "$installed" = 1 ] || warn "No prebuilt release found for $PLATFORM/$ARCH ($VERSION) — building from source instead."
fi
[ "$installed" = 1 ] || install_source

[ -x "$EXE" ] || die "Installation failed: $EXE is missing"

# Command-line entry point.
mkdir -p "$BIN_DIR"
ln -sf "$EXE" "$BIN_DIR/annotate"

# Linux menu entry.
if [ "$PLATFORM" = linux ]; then
  mkdir -p "$HOME/.local/share/applications"
  cat > "$HOME/.local/share/applications/annotate.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Annotate
Comment=Annotated screenshots for any AI harness
Exec=$EXE
Icon=annotate
Categories=Development;Utility;
Terminal=false
EOF
fi

ok "Installed $("$EXE" --version 2>/dev/null || echo Annotate) → $([ "$PLATFORM" = macos ] && echo "$APP" || echo "$EXE")"

# Optional integrations.
if [ -n "$INTEGRATIONS" ]; then
  for id in $(echo "$INTEGRATIONS" | tr ',' ' '); do
    if "$EXE" integrations install "$id"; then ok "Integration: $id"; else warn "Integration $id could not be installed"; fi
  done
fi

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) warn "$BIN_DIR is not on your PATH. Add this line to your shell profile (~/.zshrc, ~/.bashrc…):"
     echo "    export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac

echo
echo "${B}Next steps${N}"
echo "  • Open Annotate:            annotate   (or from $([ "$PLATFORM" = macos ] && echo Launchpad/Applications || echo "your app menu"))"
echo "  • Connect a harness:        annotate integrations install pi     (or claude-code, codex, opencode, …)"
echo "  • List destinations:        annotate harnesses"
[ "$PLATFORM" = macos ] && echo "  • First capture: allow Annotate in System Settings → Privacy & Security → Screen Recording."
echo "  • Tutorial:                 https://github.com/$REPO/blob/HEAD/docs/tutorial.html"
