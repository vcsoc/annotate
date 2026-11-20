//! Installs Annotate into harnesses: a Pi extension, an OpenCode plugin, the
//! built-in MCP server (Claude Code, Codex, Gemini CLI, Claude Desktop, Cursor)
//! and an Agent Skill. Existing config files are merged, never replaced, and a
//! one-time `.annotate-backup` copy is kept before the first edit.
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const PI_EXTENSION: &str = include_str!("../../integrations/pi/index.ts");
const OPENCODE_PLUGIN: &str = include_str!("../../integrations/opencode/annotate.js");
const SKILL: &str = include_str!("../../integrations/skill/SKILL.md");

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub id: String,
    pub name: String,
    pub description: String,
    pub installed: bool,
    pub available: bool,
}

struct Spec {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    /// Binary that signals the harness is installed ("" = always available).
    bin: &'static str,
}

const SPECS: &[Spec] = &[
    Spec {
        id: "pi",
        name: "Pi",
        description:
            "Extension: live Pi sessions, /annotate, Follow-up or Queued (steering) delivery.",
        bin: "pi",
    },
    Spec {
        id: "claude-code",
        name: "Claude Code",
        description: "MCP server (request_annotations / get_annotations) and the annotate skill.",
        bin: "claude",
    },
    Spec {
        id: "codex",
        name: "Codex CLI",
        description: "MCP server in ~/.codex/config.toml and the annotate skill.",
        bin: "codex",
    },
    Spec {
        id: "opencode",
        name: "OpenCode",
        description: "Plugin: batches are prompted into the active OpenCode session with images.",
        bin: "opencode",
    },
    Spec {
        id: "gemini",
        name: "Gemini CLI",
        description: "MCP server in ~/.gemini/settings.json.",
        bin: "gemini",
    },
    Spec {
        id: "claude-desktop",
        name: "Claude Desktop",
        description: "MCP server in claude_desktop_config.json.",
        bin: "",
    },
    Spec {
        id: "cursor",
        name: "Cursor",
        description: "MCP server in ~/.cursor/mcp.json (Cursor app and cursor-agent).",
        bin: "",
    },
    Spec {
        id: "agent-skills",
        name: "Agent Skills (any harness)",
        description: "Skill in ~/.agents/skills: any agent with a shell can run `annotate wait`.",
        bin: "",
    },
];

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

fn bin() -> String {
    crate::paths::current_exe()
}

fn claude_desktop_config() -> PathBuf {
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support/Claude/claude_desktop_config.json")
    } else if cfg!(windows) {
        dirs::config_dir()
            .unwrap_or_default()
            .join("Claude/claude_desktop_config.json")
    } else {
        home().join(".config/Claude/claude_desktop_config.json")
    }
}

fn pi_extension() -> PathBuf {
    let agent = std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".pi/agent"));
    agent.join("extensions/annotate/index.ts")
}

fn opencode_plugin() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"));
    base.join("opencode/plugins/annotate.js")
}

fn skill_path(root: &str) -> PathBuf {
    home().join(root).join("skills/annotate/SKILL.md")
}

fn write_file(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    crate::settings::write_atomic(path, text.as_bytes())
}

fn backup(path: &Path) {
    let copy = PathBuf::from(format!("{}.annotate-backup", path.display()));
    if path.exists() && !copy.exists() {
        let _ = std::fs::copy(path, copy);
    }
}

fn skill_text() -> String {
    SKILL.replace("{{BIN}}", &bin())
}

fn mcp_entry() -> Value {
    json!({ "command": bin(), "args": ["mcp"] })
}

/// Reads a JSON config, refusing to overwrite a file that does not parse.
fn read_json(path: &Path) -> Result<Value, String> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(json!({})),
        Ok(text) => serde_json::from_str(&text).map_err(|e| {
            format!(
                "{} is not plain JSON ({e}); add the annotate MCP server manually.",
                path.display()
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e.to_string()),
    }
}

fn set_json_server(path: &Path, key: &str, install: bool) -> Result<(), String> {
    let mut value = read_json(path)?;
    if !value.is_object() {
        return Err(format!("{} is not a JSON object", path.display()));
    }
    let servers = value
        .as_object_mut()
        .unwrap()
        .entry(key)
        .or_insert_with(|| json!({}));
    let Some(servers) = servers.as_object_mut() else {
        return Err(format!("{key} in {} is not an object", path.display()));
    };
    if install {
        servers.insert("annotate".into(), mcp_entry());
    } else if servers.remove("annotate").is_none() {
        return Ok(());
    }
    backup(path);
    write_file(
        path,
        &(serde_json::to_string_pretty(&value).unwrap() + "\n"),
    )
}

fn json_has_server(path: &Path, key: &str) -> bool {
    read_json(path)
        .ok()
        .is_some_and(|v| v[key]["annotate"].is_object())
}

fn codex_config() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".codex"))
        .join("config.toml")
}

fn codex_set(install: bool) -> Result<(), String> {
    let path = codex_config();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| format!("{} is not valid TOML: {e}", path.display()))?;
    if install {
        let servers = doc.entry("mcp_servers").or_insert(toml_edit::table());
        let Some(servers) = servers.as_table_mut() else {
            return Err("mcp_servers in config.toml is not a table".into());
        };
        servers.set_implicit(true);
        let mut table = toml_edit::Table::new();
        table["command"] = toml_edit::value(bin());
        let mut args = toml_edit::Array::new();
        args.push("mcp");
        table["args"] = toml_edit::value(args);
        servers.insert("annotate", toml_edit::Item::Table(table));
    } else if let Some(servers) = doc.get_mut("mcp_servers").and_then(|s| s.as_table_mut()) {
        if servers.remove("annotate").is_none() {
            return Ok(());
        }
    } else {
        return Ok(());
    }
    backup(&path);
    write_file(&path, &doc.to_string())
}

fn codex_has() -> bool {
    std::fs::read_to_string(codex_config())
        .ok()
        .and_then(|t| t.parse::<toml_edit::DocumentMut>().ok())
        .is_some_and(|d| {
            d.get("mcp_servers")
                .and_then(|s| s.get("annotate"))
                .is_some()
        })
}

fn claude_code_has_mcp() -> bool {
    json_has_server(&home().join(".claude.json"), "mcpServers")
}

fn claude_code_mcp(install: bool) -> Result<(), String> {
    if let Some(claude) = crate::harness::which_bin("claude") {
        let claude = claude.display().to_string();
        let _ = crate::platform::run(
            &claude,
            &["mcp", "remove", "--scope", "user", "annotate"],
            20_000,
        );
        if install {
            let exe = bin();
            crate::platform::run(
                &claude,
                &[
                    "mcp", "add", "--scope", "user", "annotate", "--", &exe, "mcp",
                ],
                20_000,
            )?;
        }
        Ok(())
    } else {
        set_json_server(&home().join(".claude.json"), "mcpServers", install)
    }
}

fn remove_file(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => {
            if let Some(dir) = path.parent() {
                let _ = std::fs::remove_dir(dir);
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

fn installed(id: &str) -> bool {
    match id {
        "pi" => pi_extension().exists(),
        "claude-code" => claude_code_has_mcp() && skill_path(".claude").exists(),
        "codex" => codex_has(),
        "opencode" => opencode_plugin().exists(),
        "gemini" => json_has_server(&home().join(".gemini/settings.json"), "mcpServers"),
        "claude-desktop" => json_has_server(&claude_desktop_config(), "mcpServers"),
        "cursor" => json_has_server(&home().join(".cursor/mcp.json"), "mcpServers"),
        "agent-skills" => skill_path(".agents").exists(),
        _ => false,
    }
}

fn available(spec: &Spec) -> bool {
    match spec.id {
        "claude-desktop" => {
            claude_desktop_config().parent().is_some_and(Path::exists)
                || crate::harness::installed().contains_key("claude-desktop")
        }
        "cursor" => {
            home().join(".cursor").exists() || crate::harness::installed().contains_key("cursor")
        }
        _ => spec.bin.is_empty() || crate::harness::which_bin(spec.bin).is_some(),
    }
}

pub fn list() -> Vec<Status> {
    SPECS
        .iter()
        .map(|s| Status {
            id: s.id.into(),
            name: s.name.into(),
            description: s.description.into(),
            installed: installed(s.id),
            available: available(s),
        })
        .collect()
}

pub fn install(id: &str) -> Result<String, String> {
    let spec = SPECS
        .iter()
        .find(|s| s.id == id)
        .ok_or("Unknown integration")?;
    match id {
        "pi" => write_file(
            &pi_extension(),
            &PI_EXTENSION.replace(
                "__ANNOTATE_BIN__",
                &bin().replace('\\', "\\\\").replace('\'', "\\'"),
            ),
        )?,
        "claude-code" => {
            write_file(&skill_path(".claude"), &skill_text())?;
            claude_code_mcp(true)?;
        }
        "codex" => {
            codex_set(true)?;
            write_file(&skill_path(".codex"), &skill_text())?;
        }
        "opencode" => write_file(&opencode_plugin(), OPENCODE_PLUGIN)?,
        "gemini" => set_json_server(&home().join(".gemini/settings.json"), "mcpServers", true)?,
        "claude-desktop" => set_json_server(&claude_desktop_config(), "mcpServers", true)?,
        "cursor" => set_json_server(&home().join(".cursor/mcp.json"), "mcpServers", true)?,
        "agent-skills" => write_file(&skill_path(".agents"), &skill_text())?,
        _ => return Err("Unknown integration".into()),
    }
    let restart = match id {
        "pi" => " Run /reload in open Pi sessions.",
        "agent-skills" => "",
        _ => " Restart open sessions of it to load the change.",
    };
    Ok(format!("Installed the {} integration.{restart}", spec.name))
}

pub fn uninstall(id: &str) -> Result<String, String> {
    let spec = SPECS
        .iter()
        .find(|s| s.id == id)
        .ok_or("Unknown integration")?;
    match id {
        "pi" => remove_file(&pi_extension())?,
        "claude-code" => {
            remove_file(&skill_path(".claude"))?;
            claude_code_mcp(false)?;
        }
        "codex" => {
            codex_set(false)?;
            remove_file(&skill_path(".codex"))?;
        }
        "opencode" => remove_file(&opencode_plugin())?,
        "gemini" => set_json_server(&home().join(".gemini/settings.json"), "mcpServers", false)?,
        "claude-desktop" => set_json_server(&claude_desktop_config(), "mcpServers", false)?,
        "cursor" => set_json_server(&home().join(".cursor/mcp.json"), "mcpServers", false)?,
        "agent-skills" => remove_file(&skill_path(".agents"))?,
        _ => return Err("Unknown integration".into()),
    }
    Ok(format!("Removed the {} integration.", spec.name))
}

/// Generic MCP configuration for any other client.
pub fn mcp_snippet() -> String {
    serde_json::to_string_pretty(&json!({ "mcpServers": { "annotate": mcp_entry() } })).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_merge_keeps_other_servers() {
        let dir = std::env::temp_dir().join(format!("annotate-int-{}", uuid::Uuid::new_v4()));
        let file = dir.join("mcp.json");
        write_file(
            &file,
            r#"{"mcpServers":{"other":{"command":"x"}},"theme":"dark"}"#,
        )
        .unwrap();
        set_json_server(&file, "mcpServers", true).unwrap();
        let v = read_json(&file).unwrap();
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["mcpServers"]["annotate"]["args"][0], "mcp");
        assert_eq!(v["theme"], "dark");
        set_json_server(&file, "mcpServers", false).unwrap();
        assert!(read_json(&file).unwrap()["mcpServers"]["annotate"].is_null());
        write_file(&file, "{ // comment\n}").unwrap();
        assert!(set_json_server(&file, "mcpServers", true).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn templates_have_placeholders() {
        assert!(PI_EXTENSION.contains("__ANNOTATE_BIN__"));
        assert!(SKILL.contains("{{BIN}}"));
        assert!(OPENCODE_PLUGIN.contains("export const AnnotatePlugin"));
    }
}
