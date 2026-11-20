# Changelog

## 1.0.0

Annotate is now a standalone, harness-agnostic **Rust + Tauri** app for macOS, Windows and Linux. It replaces the Electron companion of the pi-annotate extension.

- **Same Console, same features.** The Timeline feed and Command pill designs, live desktop marking (Freehand/Area), the note beside the mark, numbered marks in the annotation popup, Retake/Reuse/Remove, edge docking, every Setting and batch Send are carried over.
- **Send to any harness.** The destination dropdown lists:
  - live sessions (Pi, OpenCode, any MCP client, `annotate wait`, and sessions of the original pi-annotate extension);
  - installed console harnesses, started in a new terminal in the chosen project folder;
  - installed desktop chat apps (pasted automatically, optionally sent);
  - Open WebUI in the browser;
  - a plain folder.
- **Built-in integrations**, installed from Settings or `annotate integrations install`: a Pi extension, an OpenCode plugin, an MCP server for Claude Code, Codex, Gemini CLI, Claude Desktop and Cursor, and an Agent Skill. Any other MCP client works with the copied config.
- **MCP tools** `request_annotations` and `get_annotations`, so agents can ask you to show them something on screen.
- **Copy to clipboard.** Copy one annotation or all of them as image and notes (rich), a screenshot with the note printed under it, the screenshot only, or the notes only.
- **One-line install:** `install.sh` (macOS/Linux, `curl … | sh`) and `install.ps1` (Windows, `irm … | iex`). They install the latest release, or build from source with Rust when none exists, and can add integrations, pin a version or uninstall.
- **CLI:** `annotate open | capture | quit | wait | mcp | harnesses | integrations`.
- **GNOME/KDE Wayland** now works through display capture and the in-image editor (outline the whole app, then mark).
- Every batch is saved as `ANNOTATIONS.md`, `annotations.json` and the PNGs under `<project>/.annotate/<batch-id>/`.
- New settings: **Keep the Console open after Send**, **Copy format**, **Terminal for new sessions**, **Open WebUI address**, **Desktop app paste delay**.
