//! Harness catalog and discovery.
//!
//! A *harness* is anything an annotation batch can be sent to: console agents
//! (Pi, Claude Code, Codex, OpenCode, Gemini CLI…), desktop chat apps (ChatGPT,
//! Claude, OpenCode, Open WebUI…), browser apps, or a plain folder. Live sessions
//! of harnesses with an Annotate integration register themselves (see bridge.rs)
//! and receive batches directly; everything else is reached by starting a new
//! terminal session or by pasting into the desktop app.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Mode {
    pub id: String,
    pub label: String,
}

pub fn mode(id: &str, label: &str) -> Mode {
    Mode {
        id: id.into(),
        label: label.into(),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Terminal,
    Desktop,
    Web,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Harness {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// Terminal: argv template. `{prompt}` is replaced by the one-line request.
    #[serde(default)]
    pub command: Vec<String>,
    /// Desktop: application names per platform (macOS bundle name, Windows Start
    /// menu name, Linux .desktop Name=).
    #[serde(default)]
    pub app: Vec<String>,
    /// Web: address to open.
    #[serde(default)]
    pub url: String,
    /// Optional key chord pressed after activation to focus the chat input (e.g. "cmd+l").
    #[serde(default)]
    pub keys: String,
    /// Integration id that gives this harness live sessions.
    #[serde(default)]
    pub integration: String,
}

fn term(id: &str, name: &str, command: &[&str], integration: &str) -> Harness {
    Harness {
        id: id.into(),
        name: name.into(),
        kind: Kind::Terminal,
        command: command.iter().map(|s| s.to_string()).collect(),
        app: vec![],
        url: String::new(),
        keys: String::new(),
        integration: integration.into(),
    }
}

fn desk(id: &str, name: &str, app: &[&str], keys: &str, integration: &str) -> Harness {
    Harness {
        id: id.into(),
        name: name.into(),
        kind: Kind::Desktop,
        command: vec![],
        app: app.iter().map(|s| s.to_string()).collect(),
        url: String::new(),
        keys: keys.into(),
        integration: integration.into(),
    }
}

/// Built-in catalog. Users can add more in `~/.annotate/harnesses.json`.
pub fn builtin() -> Vec<Harness> {
    vec![
        term("pi", "Pi", &["pi", "{prompt}"], "pi"),
        term(
            "claude-code",
            "Claude Code",
            &["claude", "{prompt}"],
            "claude-code",
        ),
        term("codex", "Codex CLI", &["codex", "{prompt}"], "codex"),
        term(
            "opencode",
            "OpenCode",
            &["opencode", "--prompt", "{prompt}"],
            "opencode",
        ),
        term(
            "gemini",
            "Gemini CLI",
            &["gemini", "-i", "{prompt}"],
            "gemini",
        ),
        term(
            "copilot",
            "GitHub Copilot CLI",
            &["copilot", "-i", "{prompt}"],
            "",
        ),
        term(
            "cursor-agent",
            "Cursor Agent CLI",
            &["cursor-agent", "{prompt}"],
            "cursor",
        ),
        term("qwen", "Qwen Code", &["qwen", "-i", "{prompt}"], ""),
        term(
            "goose",
            "Goose",
            &["goose", "run", "--interactive", "-t", "{prompt}"],
            "",
        ),
        desk("chatgpt", "ChatGPT", &["ChatGPT"], "", ""),
        desk(
            "claude-desktop",
            "Claude",
            &["Claude"],
            "",
            "claude-desktop",
        ),
        desk(
            "opencode-desktop",
            "OpenCode Desktop",
            &["OpenCode"],
            "",
            "",
        ),
        desk("open-webui-desktop", "Open WebUI", &["Open WebUI"], "", ""),
        desk("codex-desktop", "Codex", &["Codex"], "", ""),
        desk("lm-studio", "LM Studio", &["LM Studio"], "", ""),
        desk("ollama", "Ollama", &["Ollama"], "", ""),
        desk("jan", "Jan", &["Jan"], "", ""),
        desk("msty", "Msty", &["Msty"], "", ""),
        desk("perplexity", "Perplexity", &["Perplexity"], "", ""),
        desk("cherry-studio", "Cherry Studio", &["Cherry Studio"], "", ""),
        desk("anythingllm", "AnythingLLM", &["AnythingLLM"], "", ""),
        desk("goose-desktop", "Goose Desktop", &["Goose"], "", ""),
        desk("cursor", "Cursor (chat)", &["Cursor"], "mod+l", "cursor"),
        desk("windsurf", "Windsurf (Cascade)", &["Windsurf"], "mod+l", ""),
        desk(
            "vscode",
            "VS Code (Copilot Chat)",
            &["Visual Studio Code", "Code"],
            "mod+ctrl+i",
            "",
        ),
        Harness {
            id: "open-webui".into(),
            name: "Open WebUI (browser)".into(),
            kind: Kind::Web,
            command: vec![],
            app: vec![],
            url: String::new(),
            keys: String::new(),
            integration: String::new(),
        },
    ]
}

pub fn custom() -> Vec<Harness> {
    let Ok(text) = std::fs::read_to_string(crate::paths::custom_harnesses()) else {
        return vec![];
    };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&text) else {
        return vec![];
    };
    items
        .into_iter()
        .filter_map(|v| serde_json::from_value::<Harness>(v).ok())
        .filter(|h| {
            !h.id.is_empty()
                && h.id.len() <= 60
                && h.id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .filter(|h| match h.kind {
            Kind::Terminal => {
                h.command.iter().any(|a| a.contains("{prompt}")) && !h.command.is_empty()
            }
            Kind::Desktop => !h.app.is_empty(),
            Kind::Web => h.url.starts_with("http://") || h.url.starts_with("https://"),
        })
        .map(|h| Harness {
            id: format!("custom-{}", h.id),
            ..h
        })
        .collect()
}

pub fn catalog() -> Vec<Harness> {
    let mut all = builtin();
    all.extend(custom());
    all
}

pub fn find(id: &str) -> Option<Harness> {
    catalog().into_iter().find(|h| h.id == id)
}

/// Friendly name for an MCP client / integration harness id.
pub fn harness_name(id: &str) -> String {
    if let Some(h) = find(id) {
        return h.name;
    }
    match id {
        "mcp" => "MCP client".into(),
        "cli" => "Terminal".into(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------- discovery

/// Where an installed desktop app lives / how to launch it.
#[derive(Clone, Debug, Serialize)]
pub struct Installed {
    pub harness: String,
    /// macOS .app path, Windows AppID, Linux .desktop id, or terminal binary path.
    pub location: String,
}

/// Extra binary folders that GUI-launched apps often miss from PATH.
fn extra_bin_dirs() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut dirs: Vec<PathBuf> = [
        ".local/bin",
        ".opencode/bin",
        ".volta/bin",
        ".bun/bin",
        ".npm-global/bin",
        ".cargo/bin",
        ".deno/bin",
        "bin",
        ".claude/local",
    ]
    .iter()
    .map(|d| home.join(d))
    .collect();
    if cfg!(target_os = "macos") {
        dirs.extend(
            ["/opt/homebrew/bin", "/usr/local/bin"]
                .iter()
                .map(PathBuf::from),
        );
    }
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
    }
    dirs
}

pub fn which_bin(bin: &str) -> Option<PathBuf> {
    if let Ok(path) = which::which(bin) {
        return Some(path);
    }
    let paths = std::env::join_paths(extra_bin_dirs()).ok()?;
    which::which_in(
        bin,
        Some(paths),
        std::env::current_dir().unwrap_or_default(),
    )
    .ok()
}

/// macOS/Linux apps launched from the Dock get a minimal PATH. Adopt the user's
/// login-shell PATH once so terminal harnesses are found and can run.
pub fn adopt_login_path() {
    #[cfg(unix)]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let child = std::process::Command::new(&shell)
            .args(["-l", "-c", "printf '__ANNOTATE_PATH__%s' \"$PATH\""])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .spawn();
        let Ok(child) = child else { return };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(child.wait_with_output());
        });
        if let Ok(Ok(output)) = rx.recv_timeout(std::time::Duration::from_secs(4)) {
            let text = String::from_utf8_lossy(&output.stdout);
            if let Some(path) = text
                .rsplit("__ANNOTATE_PATH__")
                .next()
                .filter(|p| !p.trim().is_empty())
            {
                let mut merged: Vec<PathBuf> = std::env::split_paths(path.trim()).collect();
                if let Some(existing) = std::env::var_os("PATH") {
                    for p in std::env::split_paths(&existing) {
                        if !merged.contains(&p) {
                            merged.push(p);
                        }
                    }
                }
                if let Ok(joined) = std::env::join_paths(merged) {
                    std::env::set_var("PATH", joined);
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn find_app(names: &[String]) -> Option<String> {
    let home = dirs::home_dir().unwrap_or_default();
    let roots = [
        PathBuf::from("/Applications"),
        home.join("Applications"),
        PathBuf::from("/Applications/Setapp"),
        PathBuf::from("/System/Applications"),
    ];
    names.iter().find_map(|name| {
        roots
            .iter()
            .map(|r| r.join(format!("{name}.app")))
            .find(|p| p.exists())
            .map(|p| p.display().to_string())
    })
}

#[cfg(windows)]
fn start_apps() -> &'static Vec<(String, String)> {
    static APPS: OnceLock<Vec<(String, String)>> = OnceLock::new();
    APPS.get_or_init(|| {
        let output = crate::platform::hidden_command("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); Get-StartApps | Select-Object Name,AppID | ConvertTo-Json -Compress"])
            .output();
        let Ok(output) = output else { return vec![] };
        let value: Value = serde_json::from_slice(String::from_utf8_lossy(&output.stdout).trim_start_matches('\u{feff}').as_bytes()).unwrap_or(Value::Null);
        let rows = match value {
            Value::Array(rows) => rows,
            Value::Object(_) => vec![value],
            _ => vec![],
        };
        rows.iter().filter_map(|r| Some((r["Name"].as_str()?.to_string(), r["AppID"].as_str()?.to_string()))).collect()
    })
}

#[cfg(windows)]
fn find_app(names: &[String]) -> Option<String> {
    let apps = start_apps();
    names.iter().find_map(|name| {
        apps.iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .or_else(|| {
                apps.iter()
                    .find(|(n, _)| n.to_lowercase().starts_with(&name.to_lowercase()))
            })
            .map(|(_, id)| id.clone())
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn desktop_entries() -> &'static Vec<(String, String)> {
    static ENTRIES: OnceLock<Vec<(String, String)>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        let home = dirs::home_dir().unwrap_or_default();
        let mut roots = vec![
            home.join(".local/share/applications"),
            home.join(".local/share/flatpak/exports/share/applications"),
        ];
        let data_dirs =
            std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        roots.extend(
            data_dirs
                .split(':')
                .map(|d| PathBuf::from(d).join("applications")),
        );
        roots.extend(
            [
                "/var/lib/flatpak/exports/share/applications",
                "/var/lib/snapd/desktop/applications",
            ]
            .iter()
            .map(PathBuf::from),
        );
        let mut out = vec![];
        for root in roots {
            let Ok(entries) = std::fs::read_dir(&root) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if let Some(name) = text.lines().find_map(|l| l.strip_prefix("Name=")) {
                    out.push((name.trim().to_string(), path.display().to_string()));
                }
            }
        }
        out
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn find_app(names: &[String]) -> Option<String> {
    let entries = desktop_entries();
    names.iter().find_map(|name| {
        entries
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, p)| p.clone())
    })
}

/// Detects which catalog harnesses are installed. Cached; call `rescan` to refresh.
fn cache() -> &'static Mutex<Option<HashMap<String, Installed>>> {
    static CACHE: OnceLock<Mutex<Option<HashMap<String, Installed>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

pub fn rescan() {
    let found = scan();
    *cache().lock().unwrap() = Some(found);
}

pub fn installed() -> HashMap<String, Installed> {
    let cache = cache();
    if let Some(found) = cache.lock().unwrap().clone() {
        return found;
    }
    let found = scan();
    *cache.lock().unwrap() = Some(found.clone());
    found
}

fn scan() -> HashMap<String, Installed> {
    let mut found = HashMap::new();
    for h in catalog() {
        let location = match h.kind {
            Kind::Terminal => h
                .command
                .first()
                .and_then(|bin| which_bin(bin))
                .map(|p| p.display().to_string()),
            Kind::Desktop => find_app(&h.app),
            Kind::Web => Some(h.url.clone()),
        };
        if let Some(location) = location {
            found.insert(
                h.id.clone(),
                Installed {
                    harness: h.id.clone(),
                    location,
                },
            );
        }
    }
    found
}

// ---------------------------------------------------------------- destinations

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Destination {
    pub id: String,
    /// "session" | "terminal" | "desktop" | "web" | "folder"
    pub group: String,
    pub harness: String,
    pub harness_name: String,
    pub label: String,
    pub project: String,
    pub session: String,
    pub name: String,
    pub modes: Vec<Mode>,
    pub needs_folder: bool,
    pub detail: String,
}

pub fn paste_modes() -> Vec<Mode> {
    vec![
        mode("paste", "Paste (review, then send)"),
        mode("pasteSend", "Paste and send"),
    ]
}

/// Static destinations: installed harnesses (new session / paste) and the folder.
pub fn static_destinations() -> Vec<Destination> {
    let installed = installed();
    let mut out = vec![];
    for h in catalog() {
        let Some(found) = installed.get(&h.id) else {
            continue;
        };
        let (group, label, modes, needs_folder, detail) = match h.kind {
            Kind::Terminal => (
                "terminal",
                format!("{} — new session", h.name),
                vec![mode("newSession", "New terminal session")],
                true,
                format!(
                    "Starts {} in a new terminal in the chosen folder: {}",
                    h.name, found.location
                ),
            ),
            Kind::Desktop => (
                "desktop",
                format!("{} app", h.name),
                paste_modes(),
                false,
                format!("Activates {} and pastes the screenshots and notes.", h.name),
            ),
            Kind::Web => (
                "web",
                h.name.clone(),
                paste_modes(),
                false,
                "Opens Open WebUI in your browser and pastes the screenshots and notes.".into(),
            ),
        };
        out.push(Destination {
            id: format!("{group}:{}", h.id),
            group: group.into(),
            harness: h.id.clone(),
            harness_name: h.name.clone(),
            label,
            project: String::new(),
            session: String::new(),
            name: String::new(),
            modes,
            needs_folder,
            detail,
        });
    }
    out.push(Destination {
        id: "folder:".into(),
        group: "folder".into(),
        harness: "folder".into(),
        harness_name: "Folder".into(),
        label: "Save to folder only".into(),
        project: String::new(),
        session: String::new(),
        name: String::new(),
        modes: vec![mode("save", "Save files")],
        needs_folder: true,
        detail: "Saves the screenshots, annotations.json and ANNOTATIONS.md into <folder>/.annotate/ and opens it.".into(),
    });
    out
}

/// Live sessions from the registry that answer an authenticated liveness check.
pub fn live_sessions() -> Vec<(crate::bridge::Record, Destination)> {
    let mut records = crate::bridge::records(&crate::paths::sessions());
    records.extend(
        crate::bridge::records(&crate::paths::legacy_sessions())
            .into_iter()
            .map(|r| crate::bridge::Record { legacy: true, ..r }),
    );
    let handles: Vec<_> = records
        .into_iter()
        .map(|record| {
            std::thread::spawn(move || {
                let status = crate::bridge::request(
                    &record.url,
                    &record.token,
                    "/session",
                    None,
                    std::time::Duration::from_millis(1500),
                )
                .ok()?;
                if status["targetId"].as_str() != Some(record.id.as_str())
                    || status["project"].as_str().unwrap_or("") != record.project
                    || status["session"].as_str().unwrap_or("") != record.session
                {
                    return None;
                }
                let harness = status["harness"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(if record.harness.is_empty() {
                        "pi"
                    } else {
                        &record.harness
                    })
                    .to_string();
                let harness_name = status["harnessName"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .unwrap_or_else(|| {
                        if record.harness_name.is_empty() {
                            harness_name(&harness)
                        } else {
                            record.harness_name.clone()
                        }
                    });
                let modes = parse_modes(&status["deliveryModes"]);
                let name: String = status["name"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(120)
                    .collect();
                let destination = Destination {
                    id: format!("session:{}", record.id),
                    group: "session".into(),
                    harness: harness.clone(),
                    harness_name: harness_name.clone(),
                    label: harness_name.clone(),
                    project: record.project.clone(),
                    session: record.session.clone(),
                    name,
                    modes,
                    needs_folder: false,
                    detail: format!("Live {harness_name} session in {}", record.project),
                };
                Some((record, destination))
            })
        })
        .collect();
    let mut live: Vec<_> = handles
        .into_iter()
        .filter_map(|h| h.join().ok().flatten())
        .collect();
    live.sort_by(|a, b| {
        a.1.harness_name
            .cmp(&b.1.harness_name)
            .then(a.1.project.cmp(&b.1.project))
            .then(a.1.id.cmp(&b.1.id))
    });
    live
}

/// Accepts `["followUp","steer"]` (original Pi extension) or `[{id,label}]`.
pub fn parse_modes(value: &Value) -> Vec<Mode> {
    let known = |id: &str| match id {
        "followUp" => "Follow-up",
        "steer" => "Queued (steering)",
        "prompt" => "Prompt now",
        "queue" => "Queue for the agent",
        "return" => "Return to waiting agent",
        _ => "",
    };
    let modes: Vec<Mode> = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|m| match m {
                    Value::String(id) => {
                        Some(mode(id, if known(id).is_empty() { id } else { known(id) }))
                    }
                    Value::Object(o) => {
                        let id = o.get("id")?.as_str()?;
                        let label = o.get("label").and_then(|l| l.as_str()).unwrap_or(known(id));
                        Some(mode(id, if label.is_empty() { id } else { label }))
                    }
                    _ => None,
                })
                .filter(|m| m.id.len() <= 40 && m.label.len() <= 80)
                .take(8)
                .collect()
        })
        .unwrap_or_default();
    if modes.is_empty() {
        vec![mode("followUp", "Follow-up")]
    } else {
        modes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_and_object_modes() {
        let legacy = parse_modes(&serde_json::json!(["followUp", "steer"]));
        assert_eq!(legacy[1].label, "Queued (steering)");
        let objects = parse_modes(&serde_json::json!([{ "id": "queue", "label": "Queue" }]));
        assert_eq!(objects, vec![mode("queue", "Queue")]);
        assert_eq!(parse_modes(&Value::Null)[0].id, "followUp");
    }

    #[test]
    fn catalog_ids_are_unique() {
        let all = builtin();
        let mut ids: Vec<_> = all.iter().map(|h| h.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        assert!(all
            .iter()
            .filter(|h| h.kind == Kind::Terminal)
            .all(|h| h.command.iter().any(|a| a == "{prompt}")));
    }
}
