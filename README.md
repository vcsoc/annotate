# Annotate — v1.0.0

A floating Console for collecting application screenshots, highlighted areas and notes, then sending the whole batch to **any AI harness** — console agents (Pi, Claude Code, Codex, OpenCode, Gemini CLI, Copilot CLI…), desktop apps (ChatGPT, Claude, OpenCode, Open WebUI, LM Studio…), a browser app, or a plain folder. Any annotation can also be **copied to the clipboard** and pasted into documents, slides, PDF editors or chat apps.

Annotate is a native **Rust + Tauri** app for macOS, Windows and Linux. It is the harness-agnostic successor of the [pi-annotate](https://github.com/vcsoc/pi-annotate) Pi extension and keeps its design and features: the Timeline feed and Command pill Consoles, live desktop marking, notes beside the mark, numbered marks, retake/replace, batch Send and every Setting.

**Every saved image contains the whole selected application AND your highlighted mark together — not a crop of just the marked area.** The note is kept with that image and sent as text.

**New here?** Open the illustrated step-by-step guide: [`docs/tutorial.html`](docs/tutorial.html).

## Install — one line

**macOS / Linux**

```sh
curl -fsSL https://raw.githubusercontent.com/vcsoc/annotate/HEAD/install.sh | sh
```

**Windows** (PowerShell)

```powershell
powershell -c "irm https://raw.githubusercontent.com/vcsoc/annotate/HEAD/install.ps1 | iex"
```

**With Cargo** (any OS with Rust; installs the `annotate` binary only)

```sh
cargo install --git https://github.com/vcsoc/annotate annotate --locked
```

The scripts install the latest GitHub release (`.dmg` on macOS, `.AppImage` on Linux x64, the per-user installer on Windows). When no release is available they build from source automatically: they install Rust and the system libraries if needed. No Node.js or npm is used. Options:

| Option (`sh -s -- …`) | Windows | Effect |
|---|---|---|
| `--integrations pi,claude-code` | `$env:ANNOTATE_INTEGRATIONS="pi,claude-code"` | Also install integrations |
| `--version v1.0.0` | `$env:ANNOTATE_VERSION="v1.0.0"` | A specific release |
| `--source` | `$env:ANNOTATE_FROM_SOURCE="1"` | Always build from source |
| `--uninstall` | `& ([scriptblock]::Create((irm …/install.ps1))) -Uninstall` | Remove Annotate (keeps settings and batches) |
| `--yes` | — | Do not ask before installing system packages (Linux) |

Example — install and connect Pi in one go:

```sh
curl -fsSL https://raw.githubusercontent.com/vcsoc/annotate/HEAD/install.sh | sh -s -- --integrations pi
```

## Build from source manually

Annotate is pure **Rust + Tauri**. The UI is static HTML/CSS/JS embedded in the binary, so no Node.js or npm is needed to build or run it.

```sh
cargo install tauri-cli --version "^2" --locked   # once: the Tauri bundler
git clone https://github.com/vcsoc/annotate.git
cd annotate/src-tauri
cargo tauri build        # bundles the app into src-tauri/target/release/bundle/
```

Run it without bundling with `cargo run` (or `cargo tauri dev`).

Requirements: Rust (stable) plus the [Tauri system prerequisites](https://tauri.app/start/prerequisites/) for your OS:

- macOS: Xcode Command Line Tools.
- Windows: Visual Studio Build Tools (C++) and WebView2.
- Linux: Ubuntu 24.04, Debian 12, Fedora 38 or newer, because PipeWire 0.3.65+ is needed for screen capture. On Debian/Ubuntu install `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `librsvg2-dev`, `libxdo-dev`, `libpipewire-0.3-dev`, `libdbus-1-dev`, `libxcb1-dev`, `libxrandr-dev`, `libgbm-dev`, `libclang-dev`.

The install scripts put the `annotate` command on your `PATH`. After a manual build you can do it yourself (it is the same binary as the app):

```sh
# macOS
ln -s /Applications/Annotate.app/Contents/MacOS/annotate /usr/local/bin/annotate
```

Then open **Annotate**, press **＋ Capture** (or **⌘/Ctrl+Shift+A**), mark an area in any app, write a note, press **✓**, choose a destination and **Send**.

## Workflow

1. **Capture.** Press Capture (the default 1-second countdown lets you reopen a menu) or the global shortcut (no countdown). The Console hides and a transparent selection surface covers each display.
2. **Mark** an area in an application, using **Freehand** (the default) or **Area**. Switch tools in the hint bar at the top, or press **A**/**F**. **Esc** cancels.
3. Annotate finds the application under the mark and captures **that whole window** with the mark painted in. If a mark crosses the app edge, it is either **Trimmed** to the app (the default) or the capture is **Extended**.
4. A **note** opens beside the mark. Type, then press **✓** (or Ctrl/⌘+Enter). The entry is saved in the Console. **Nothing is sent yet.**
5. In the Console's timeline:
   - Edit the text in place.
   - **↻ Retake** (replace) or **⊕ Reuse** (capture again as a new entry with this note).
   - **＋ Mark** (or double-click the screenshot) opens the annotation beside the Console, where you can draw more **numbered marks**, each with its own comment.
   - **⧉ Copy** puts it on the clipboard.
   - **Remove** deletes it.
6. Choose **Send to harness / session** and **Send as**, then press **Send**. All annotations are delivered together. On failure the batch stays in the Console for a retry.

## Destinations

The destination dropdown is grouped:

| Group | What it is | How a batch arrives |
|---|---|---|
| **Live sessions** | Running harness sessions that loaded an Annotate integration: Pi, OpenCode, any MCP client (Claude Code, Codex, Gemini CLI, Cursor, Claude Desktop, VS Code…), or an agent waiting in `annotate wait`. Sessions of the original **pi-annotate** extension are listed too. | Delivered straight into that session. Pi: **Follow-up** or **Queued (steering)**. OpenCode: prompted into the active session with image parts. MCP / `wait`: returned to the agent's tool call. |
| **Start in a terminal** | Installed console harnesses: Pi, Claude Code, Codex, OpenCode, Gemini CLI, Copilot CLI, Cursor Agent, Qwen Code, Goose. | Starts a new session in your terminal, in the chosen **📁 project folder**, with a request to read `ANNOTATIONS.md` and view each screenshot. |
| **Desktop apps** | Installed chat apps: ChatGPT, Claude, OpenCode Desktop, Open WebUI, Codex, LM Studio, Ollama, Jan, Msty, Perplexity, Cherry Studio, AnythingLLM, Goose, plus Cursor, Windsurf and VS Code chat. | Brings the app to the front and pastes the notes and each screenshot. **Paste** leaves the message for you to review; **Paste and send** also presses Enter. |
| **Browser** | Open WebUI at the configured address. | Opens it and pastes as above. |
| **Other** | Save to folder only. | Writes the files and opens the folder. |

Every batch is saved as `<project>/.annotate/<batch-id>/`. Batches without a project folder go to `~/.annotate/batches/<batch-id>/`. Each folder contains `1.png…`, `annotations.json` (notes plus normalized mark coordinates) and `ANNOTATIONS.md`. Add `.annotate/` to your `.gitignore`.

Pasting into another app requires **Accessibility** permission on macOS (System Settings → Privacy & Security → Accessibility). On Wayland it requires [`wtype`](https://github.com/atx/wtype) (wlroots compositors only). Annotate restores your previous text clipboard afterwards.

### Custom harnesses

Add your own destinations in `~/.annotate/harnesses.json`:

```json
[
  { "id": "aider", "name": "Aider", "kind": "terminal", "command": ["aider", "--message", "{prompt}"] },
  { "id": "myapp", "name": "My Chat App", "kind": "desktop", "app": ["My Chat App"], "keys": "mod+l" },
  { "id": "webchat", "name": "Team chat", "kind": "web", "url": "https://chat.example.com" }
]
```

`{prompt}` is replaced by a one-line request that points at the saved `ANNOTATIONS.md`. In `keys`, `mod` means ⌘ on macOS and Ctrl elsewhere. The chord is pressed after activation to focus the chat input.

## Integrations

Go to **Settings → Harnesses → Integrations**, or run `annotate integrations install <id>`. Each integration is built into the app and installed with one click. Existing config files are merged, not replaced, and a one-time `*.annotate-backup` copy is kept.

| Id | Harness | What is installed |
|---|---|---|
| `pi` | Pi | Extension `~/.pi/agent/extensions/annotate/index.ts`. Adds live sessions, `/annotate` (opens the Console aimed at the session) and `/annotate stop`, and supports Follow-up and Queued (steering) delivery. **Remove the older `pi-annotate` package first.** Both register `/annotate`. Its sessions still work as destinations without this extension. |
| `claude-code` | Claude Code | MCP server (`claude mcp add --scope user annotate`) plus skill `~/.claude/skills/annotate/SKILL.md` |
| `codex` | Codex CLI | `[mcp_servers.annotate]` in `~/.codex/config.toml` plus skill `~/.codex/skills/annotate/SKILL.md` |
| `opencode` | OpenCode | Plugin `~/.config/opencode/plugins/annotate.js`. Batches are prompted into the active session with images. |
| `gemini` | Gemini CLI | `mcpServers.annotate` in `~/.gemini/settings.json` |
| `claude-desktop` | Claude Desktop | `mcpServers.annotate` in `claude_desktop_config.json` |
| `cursor` | Cursor | `mcpServers.annotate` in `~/.cursor/mcp.json` |
| `agent-skills` | Any Agent-Skills harness | `~/.agents/skills/annotate/SKILL.md`. Any agent with a shell can run `annotate wait`. |

For any other MCP client, **⧉ MCP config** (or `annotate integrations snippet`) copies:

```json
{ "mcpServers": { "annotate": { "command": "/path/to/annotate", "args": ["mcp"] } } }
```

### MCP tools

- `request_annotations { message?, wait_seconds? }`: opens the Console aimed at this session, waits for **Send** and returns every screenshot (as image content) with its notes. Agents use this to ask you to show them something.
- `get_annotations { wait_seconds? }`: returns batches you already sent to this session.

While an MCP client is connected, its session appears under **Live sessions** (for example *Claude Code · myproject*).

### `annotate wait`

For harnesses with a shell but no MCP:

```sh
annotate wait --project "$PWD" --timeout 900
```

This registers the terminal as a live destination, opens the Console aimed at it, and waits for Send. It then prints the request and screenshot paths. It exits with 0 when a batch arrives and 2 on timeout.

### Writing your own integration (protocol 1)

A live session is a loopback HTTP server plus a record file `~/.annotate/sessions/<uuid>.json` (mode 0600):

```json
{ "id": "<uuid>", "url": "http://127.0.0.1:<port>", "token": "<64 hex>", "harness": "myharness", "harnessName": "My Harness", "project": "/abs/project", "session": "<id>", "pid": 123, "protocol": 1 }
```

Requests carry `Authorization: Bearer <token>`, `Host: 127.0.0.1:<port>` and no `Origin`. Reject anything else.

- `GET /session` returns `{ targetId, harness, harnessName, project, session, name, deliveryModes: [{ id, label }], protocol: 1 }`. `targetId`, `project` and `session` must match the record.
- `POST /deliver` receives `{ protocol: 1, id, deliverAs, project, savedTo, text, items: [{ index, source, comment, kind, points, marks, width, height, file, text, image }] }`. `image` is a base64 PNG. Forward `text`, then each item's `text` and `image`. Reply `{ ok: true }`, or `{ error }` with a 4xx status. Treat repeated batch `id`s as already delivered.

See `integrations/pi/index.ts` and `integrations/opencode/annotate.js` for complete examples.

## Copy to clipboard

**⧉ Copy** (on an entry, in the annotation popup, or Ctrl/⌘+C in the Console) copies one annotation. The **⧉** icon next to 🗑 copies them all. **Settings → Clipboard → Copy format** chooses what is copied:

- **Image and notes** (the default): PNG, HTML (screenshot plus note) and plain text in one clipboard item. Word, PowerPoint, Pages, Google Docs and similar editors get the image with its note. Image-only targets get the screenshot. Text fields get the note.
- **Screenshot with the note printed under it**: a single image, ideal for PDF editors and whiteboards.
- **Screenshot only** / **Notes only**.

Several annotations are stacked into one image where a single image is needed.

## Settings

All the original Console settings are kept: capture shortcut (recorded by pressing it, applied immediately), Global shortcut switch, default mark tool, edge overflow (Trim/Extend), capture countdown (0–30 s), capture mode on Hyprland/sway (Live / Preserved frame), maximum annotations (1–12), default Send as, and Console design (**Timeline feed** docks to a screen edge at about 60% of the screen height; **Command pill** is a compact bar with a tray). New settings:

- **Keep the Console open after Send.** Off by default: as before, the Console closes after a successful Send.
- **Copy format** (see above).
- **Terminal for new sessions**: Automatic, Terminal, iTerm, Ghostty, WezTerm, kitty, Alacritty, Windows Terminal or `cmd`, or any detected Linux terminal.
- **Open WebUI address** and **Desktop app paste delay**.
- **Integrations**: install or remove, copy the MCP config, re-detect harnesses.

Preferences live in the app config directory (`settings.json`, `console-size.json`). The launch-default shortcut can be overridden with `ANNOTATE_HOTKEY` (`PI_ANNOTATE_HOTKEY` is honoured too).

## CLI

```text
annotate                         Open the Annotate Console
annotate open [--target ID] [--message TEXT]
annotate capture                 Start a capture (bind this in your compositor)
annotate quit                    Close the Console, discarding unsent drafts
annotate wait [--project DIR] [--timeout SECS] [--harness ID] [--no-open]
annotate mcp                     MCP server over stdio
annotate harnesses [--json]      List detected destinations
annotate integrations [list | install ID | uninstall ID | snippet]
```

## Desktop support

| Desktop | Capture path | Notes |
|---|---|---|
| macOS 11+ | Transparent selectors on every display (including fullscreen Spaces). Window geometry comes from CoreGraphics (JXA), and the exact window is captured by id with `screencapture -l`. | Allow **Screen Recording** for Annotate. Allow **Accessibility** only if you paste into desktop apps. |
| Windows 10/11 | Transparent selectors. Win32/DWM window geometry in physical pixels; exact-window capture. | WebView2 is required (preinstalled on Windows 11). Secure desktop/UAC/DRM content cannot be captured. |
| Linux X11 | Transparent selectors, EWMH window list and exact-window capture (xcap). | Synthetic paste uses X11. |
| Hyprland / sway | Live `slurp` Area / transparent Freehand (default), or Preserved frame. Whole-app evidence via `grim`. Windows are placed through compositor IPC. | Needs `grim` and `slurp`. The capture shortcut is a temporary compositor binding that runs `annotate capture`. |
| GNOME / KDE Wayland | Display capture through the screenshot portal. Outline the whole app, then mark inside the in-image editor. | Window geometry is not exposed by these compositors. |

Minimized, protected or OS-restricted windows may be unavailable. A window that moves or closes during selection is rejected rather than attached to the wrong app.

## Privacy and limits

- No background recording, no startup screenshot, no audio, no automatic Send.
- **The entire selected app is attached**, including content outside the highlighted area. Review it before Send.
- Drafts live in memory until Send. Closing or quitting asks before discarding them.
- Bridges bind only to `127.0.0.1`, use per-launch random tokens, and reject browser `Origin` and unexpected `Host` headers. Session records are private files (0600).
- Limits: 1–12 entries per batch (8 by default), 20 marks per annotation, 8,000 characters per note, a 24 MiB batch. Images are downscaled to at most 1920px.

## Development and tests

```sh
cd src-tauri
cargo run                       # run the app
cargo test                      # all Rust tests
cargo fmt --all --check
```

Optional: `node --test test/*.test.mjs` exercises the Pi extension and the OpenCode plugin. Those two files run inside the Pi and OpenCode harnesses, which are themselves Node-based. Building or running Annotate never needs Node.

- Rust unit tests cover geometry (targeting, overflow, docking, note placement), settings, batch packaging, bridges and auth, harness parsing, the integration config merges, the MCP client mapping and an end-to-end Console → live-session delivery.
- The optional Node tests run the Pi extension and the OpenCode plugin against fake harness APIs over the real bridge protocol, plus the original workflow tests.
- `cargo test legacy -- --ignored` delivers to an original pi-annotate bridge advertised in `PI_ANNOTATE_REGISTRY_DIR`.
- `ANNOTATE_HOME` isolates `~/.annotate`. `ANNOTATE_DISPLAYS=0` limits the selector to the primary display, which is handy while screen-sharing.

## Layout

```text
src-tauri/      Rust backend (capture, harnesses, delivery, MCP, integrations, CLI)
ui/             Console UI (HTML/CSS/JS; window.annotate API in tauri-api.js)
integrations/   Pi extension, OpenCode plugin and the agent skill (embedded in the binary)
designs/        Console design concepts carried over from pi-annotate
test/           Node tests
```

Developed by Chris Visser — <https://github.com/vcsoc>
