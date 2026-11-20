//! Built-in MCP server (`annotate mcp`, stdio JSON-RPC).
//!
//! Any MCP-capable harness (Claude Code, Codex, Gemini CLI, OpenCode, Cursor,
//! Claude Desktop, Goose, VS Code…) gets:
//! * a live destination in the Console for this session (registered on initialize);
//! * `request_annotations` — opens the Console aimed at this session and waits
//!   until the user sends a batch, returning screenshots + notes;
//! * `get_annotations` — returns batches the user already sent to this session.
use crate::batch::Package;
use crate::receiver::Receiver;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

const VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

fn client_harness(name: &str) -> (String, String) {
    let n = name.to_lowercase();
    let pick = |id: &str| (id.to_string(), crate::harness::harness_name(id));
    if n.contains("claude-code") || n.contains("claude code") {
        pick("claude-code")
    } else if n.contains("claude") {
        pick("claude-desktop")
    } else if n.contains("codex") {
        pick("codex")
    } else if n.contains("gemini") {
        pick("gemini")
    } else if n.contains("opencode") {
        pick("opencode")
    } else if n.contains("cursor") {
        pick("cursor")
    } else if n.contains("windsurf") {
        pick("windsurf")
    } else if n.contains("goose") {
        pick("goose")
    } else if n.contains("copilot") || n.contains("visual studio code") || n.contains("vscode") {
        pick("vscode")
    } else if n.contains("qwen") {
        pick("qwen")
    } else {
        let id: String = n
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .take(40)
            .collect();
        (
            if id.is_empty() { "mcp".into() } else { id },
            if name.trim().is_empty() {
                "MCP client".into()
            } else {
                name.chars().take(60).collect()
            },
        )
    }
}

fn tools() -> Value {
    json!([
        {
            "name": "request_annotations",
            "title": "Ask the user for UI annotations",
            "description": "Opens the Annotate Console aimed at this session so the user can capture screenshots of any application, highlight areas and write notes. Blocks until the user presses Send (or the wait expires) and returns every annotated screenshot with its notes. Use this when you need the user to point at something on screen.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message": { "type": "string", "description": "Short hint shown in the Console, e.g. what to annotate." },
                    "wait_seconds": { "type": "integer", "minimum": 5, "maximum": 3600, "description": "How long to wait for the user (default 900)." }
                }
            }
        },
        {
            "name": "get_annotations",
            "title": "Get pending UI annotations",
            "description": "Returns annotation batches the user has already sent to this session from the Annotate Console (screenshots of the whole app with highlighted areas, plus notes). Returns immediately unless wait_seconds is set.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "wait_seconds": { "type": "integer", "minimum": 0, "maximum": 3600, "description": "Wait up to this long for a batch (default 0)." }
                }
            }
        }
    ])
}

pub fn package_content(packages: &[Package]) -> Vec<Value> {
    let mut content = vec![];
    for package in packages {
        content.push(json!({ "type": "text", "text": package.text }));
        for item in &package.items {
            content.push(json!({ "type": "text", "text": item.text }));
            content.push(json!({ "type": "image", "data": item.image, "mimeType": "image/png" }));
        }
    }
    content
}

fn out() -> &'static Mutex<std::io::Stdout> {
    static OUT: OnceLock<Mutex<std::io::Stdout>> = OnceLock::new();
    OUT.get_or_init(|| Mutex::new(std::io::stdout()))
}

fn write(message: Value) {
    let mut stdout = out().lock().unwrap();
    let _ = writeln!(stdout, "{message}");
    let _ = stdout.flush();
}

fn reply(id: &Value, result: Value) {
    write(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
}

fn error(id: &Value, code: i64, message: &str) {
    write(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }));
}

fn tool_result(text: &str, content: Vec<Value>, is_error: bool) -> Value {
    let mut all = vec![json!({ "type": "text", "text": text })];
    all.extend(content);
    json!({ "content": all, "isError": is_error })
}

pub fn run() -> Result<(), String> {
    let receiver: Arc<Mutex<Option<Arc<Receiver>>>> = Arc::new(Mutex::new(None));
    let cwd = std::env::current_dir().unwrap_or_default();
    let project = crate::receiver::project_for(&cwd);
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            error(&Value::Null, -32700, "Parse error");
            continue;
        };
        let id = message.get("id").cloned();
        let method = message["method"].as_str().unwrap_or("").to_string();
        let params = message.get("params").cloned().unwrap_or(json!({}));
        match (method.as_str(), id) {
            ("initialize", Some(id)) => {
                let requested = params["protocolVersion"].as_str().unwrap_or("");
                let version = VERSIONS
                    .iter()
                    .find(|v| **v == requested)
                    .copied()
                    .unwrap_or(VERSIONS[1]);
                let (harness, name) =
                    client_harness(params["clientInfo"]["name"].as_str().unwrap_or(""));
                let mut guard = receiver.lock().unwrap();
                if guard.is_none() {
                    match Receiver::start(
                        &harness,
                        &name,
                        &project,
                        "MCP",
                        json!([{ "id": "queue", "label": "Queue for the agent" }]),
                    ) {
                        Ok(r) => *guard = Some(Arc::new(r)),
                        Err(e) => eprintln!("annotate mcp: could not register destination: {e}"),
                    }
                }
                reply(
                    &id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": { "tools": { "listChanged": false } },
                        "serverInfo": { "name": "annotate", "title": "Annotate", "version": env!("CARGO_PKG_VERSION") },
                        "instructions": "Annotate lets the user send annotated screenshots (whole application window with highlighted areas and notes) to this session. Call request_annotations when you need the user to show you something on screen; call get_annotations when the user says they sent annotations."
                    }),
                );
            }
            ("ping", Some(id)) => reply(&id, json!({})),
            ("tools/list", Some(id)) => reply(&id, json!({ "tools": tools() })),
            ("tools/call", Some(id)) => {
                let Some(r) = receiver.lock().unwrap().clone() else {
                    reply(
                        &id,
                        tool_result(
                            "Annotate could not register this session as a destination.",
                            vec![],
                            true,
                        ),
                    );
                    continue;
                };
                // Long waits run on their own thread so pings and other calls are answered.
                std::thread::spawn(move || call_tool(id, &params, &r));
            }
            ("resources/list", Some(id)) => reply(&id, json!({ "resources": [] })),
            ("prompts/list", Some(id)) => reply(&id, json!({ "prompts": [] })),
            (_, Some(id)) if !method.starts_with("notifications/") => {
                error(&id, -32601, &format!("Method not found: {method}"))
            }
            _ => {}
        }
    }
    Ok(())
}

fn call_tool(id: Value, params: &Value, receiver: &Receiver) {
    let args = &params["arguments"];
    let name = params["name"].as_str().unwrap_or("");
    let wait = |default: u64| {
        Duration::from_secs(args["wait_seconds"].as_u64().unwrap_or(default).min(3600))
    };
    match name {
        "get_annotations" => {
            let packages = receiver.take(wait(0));
            if packages.is_empty() {
                reply(&id, tool_result("No annotations have been sent to this session yet. Ask the user to open the Annotate Console, choose this session as the destination and press Send — or call request_annotations.", vec![], false));
            } else {
                reply(
                    &id,
                    tool_result(
                        &format!("{} annotation batch(es) received.", packages.len()),
                        package_content(&packages),
                        false,
                    ),
                );
            }
        }
        "request_annotations" => {
            let mut body = json!({ "target": receiver.record.id });
            if let Some(message) = args["message"].as_str() {
                body["message"] = json!(message.chars().take(300).collect::<String>());
            }
            if let Err(e) = crate::control::send_or_launch("/open", &body) {
                reply(
                    &id,
                    tool_result(
                        &format!("Could not open the Annotate Console: {e}"),
                        vec![],
                        true,
                    ),
                );
                return;
            }
            let packages = receiver.take(wait(900));
            if packages.is_empty() {
                reply(&id, tool_result("The user has not sent annotations yet. The Annotate Console stays open; call get_annotations later to collect them.", vec![], false));
            } else {
                reply(
                    &id,
                    tool_result(
                        &format!("The user sent {} annotation batch(es).", packages.len()),
                        package_content(&packages),
                        false,
                    ),
                );
            }
        }
        other => reply(
            &id,
            tool_result(&format!("Unknown tool: {other}"), vec![], true),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_clients() {
        assert_eq!(client_harness("claude-code").0, "claude-code");
        assert_eq!(client_harness("codex-mcp-client").0, "codex");
        assert_eq!(client_harness("gemini-cli-mcp-client").0, "gemini");
        assert_eq!(client_harness("My Tool!").0, "mytool");
        assert_eq!(client_harness("").0, "mcp");
    }
}
