//! Authenticated loopback bridges.
//!
//! * Session records: every live harness session that loaded an Annotate
//!   integration writes `~/.annotate/sessions/<id>.json` with a loopback URL and
//!   a random bearer token. The Console lists them and POSTs batches to `/deliver`.
//! * The same tiny HTTP server is used by Rust-side receivers (the MCP server and
//!   `annotate wait`) and by the Console's own control endpoint.
//!
//! Requests must carry the bearer token, the exact `127.0.0.1:<port>` Host and no
//! Origin header, so web pages cannot reach the bridges.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub url: String,
    pub token: String,
    #[serde(default)]
    pub harness: String,
    #[serde(default)]
    pub harness_name: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub session: String,
    #[serde(default)]
    pub pid: u32,
    #[serde(default = "one")]
    pub protocol: u32,
    /// Registered by the original pi-annotate extension (legacy batch format).
    #[serde(skip)]
    pub legacy: bool,
}

fn one() -> u32 {
    1
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

pub fn valid_record(r: &Record) -> bool {
    valid_id(&r.id)
        && r.url
            .strip_prefix("http://127.0.0.1:")
            .is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        && r.token.len() == 64
        && r.token
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("OS randomness");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes a session record atomically (0600) and returns a guard that removes it.
pub fn advertise(record: &Record, dir: &Path) -> Result<Advertised, String> {
    if !valid_record(record) {
        return Err("Invalid Annotate destination".into());
    }
    crate::paths::ensure_private_dir(dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("{}.json", record.id));
    crate::settings::write_atomic(&file, serde_json::to_string(record).unwrap().as_bytes())?;
    Ok(Advertised { file })
}

pub struct Advertised {
    file: std::path::PathBuf,
}

impl Drop for Advertised {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
    }
}

pub fn records(dir: &Path) -> Vec<Record> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    entries
        .flatten()
        .take(256)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let id = name.strip_suffix(".json")?;
            if !valid_id(id) {
                return None;
            }
            let meta = std::fs::symlink_metadata(entry.path()).ok()?;
            if !meta.is_file() || meta.len() > 16_384 {
                return None;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if meta.uid() != unsafe { libc_getuid() } || meta.mode() & 0o077 != 0 {
                    return None;
                }
            }
            let record: Record =
                serde_json::from_str(&std::fs::read_to_string(entry.path()).ok()?).ok()?;
            (valid_record(&record) && record.id == id).then_some(record)
        })
        .collect()
}

#[cfg(unix)]
extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().redirects(0).build()
}

/// Authenticated request to a bridge. `body = None` issues a GET.
pub fn request(
    url: &str,
    token: &str,
    path: &str,
    body: Option<&Value>,
    timeout: Duration,
) -> Result<Value, String> {
    let req = |method: &str| {
        agent()
            .request(method, &format!("{url}{path}"))
            .timeout(timeout)
            .set("Authorization", &format!("Bearer {token}"))
    };
    let result = match body {
        None => req("GET").call(),
        Some(b) => req("POST")
            .set("Content-Type", "application/json")
            .send_string(&serde_json::to_string(b).unwrap()),
    };
    match result {
        Ok(response) => response.into_json::<Value>().map_err(|e| e.to_string()),
        Err(ureq::Error::Status(_, response)) => {
            let value: Value = response.into_json().unwrap_or(json!({}));
            Err(value["error"]
                .as_str()
                .unwrap_or("Destination request failed")
                .to_string())
        }
        Err(e) => Err(format!("Destination unreachable: {e}")),
    }
}

// ---------------------------------------------------------------- server

pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Value,
}

pub type Handler = Arc<dyn Fn(Request) -> Result<Value, (u16, String)> + Send + Sync>;

pub struct Server {
    pub url: String,
    pub token: String,
    server: Arc<tiny_http::Server>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.server.unblock();
    }
}

/// Starts a loopback server on a random port with a fresh token.
pub fn serve(max_body: usize, handler: Handler) -> Result<Server, String> {
    let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").map_err(|e| e.to_string())?);
    let port = server
        .server_addr()
        .to_ip()
        .map(|a| a.port())
        .ok_or("No loopback port")?;
    let token = random_token();
    let expected_auth = format!("Bearer {token}");
    let expected_host = format!("127.0.0.1:{port}");
    let listener = server.clone();
    std::thread::spawn(move || {
        for mut req in listener.incoming_requests() {
            let header = |name: &'static str| {
                req.headers()
                    .iter()
                    .find(|h| h.field.equiv(name))
                    .map(|h| h.value.as_str().to_string())
            };
            let authorized = header("Origin").is_none()
                && header("Host").as_deref() == Some(expected_host.as_str())
                && header("Authorization")
                    .is_some_and(|a| constant_eq(a.as_bytes(), expected_auth.as_bytes()));
            let reply = |req: tiny_http::Request, status: u16, body: Value| {
                let response = tiny_http::Response::from_string(body.to_string())
                    .with_status_code(status)
                    .with_header(
                        "Content-Type: application/json"
                            .parse::<tiny_http::Header>()
                            .unwrap(),
                    )
                    .with_header(
                        "Cache-Control: no-store"
                            .parse::<tiny_http::Header>()
                            .unwrap(),
                    );
                let _ = req.respond(response);
            };
            if !authorized {
                reply(req, 401, json!({ "error": "Unauthorized companion" }));
                continue;
            }
            let method = req.method().as_str().to_string();
            let path = req.url().to_string();
            if req.body_length().is_some_and(|n| n > max_body) {
                reply(req, 413, json!({ "error": "Batch too large" }));
                continue;
            }
            let mut body = Vec::new();
            if method == "POST" {
                if !header("Content-Type").is_some_and(|c| c.starts_with("application/json")) {
                    reply(req, 415, json!({ "error": "Expected JSON" }));
                    continue;
                }
                if req
                    .as_reader()
                    .take(max_body as u64 + 1)
                    .read_to_end(&mut body)
                    .is_err()
                    || body.len() > max_body
                {
                    reply(req, 413, json!({ "error": "Batch too large" }));
                    continue;
                }
            }
            let body = if body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&body).unwrap_or(Value::Null)
            };
            let handler = handler.clone();
            // Handlers may block (e.g. waiting for the user); serve each on its own thread.
            std::thread::spawn(move || match handler(Request { method, path, body }) {
                Ok(value) => reply(req, 200, value),
                Err((status, message)) => reply(req, status, json!({ "error": message })),
            });
        }
    });
    Ok(Server {
        url: format!("http://127.0.0.1:{port}"),
        token,
        server,
    })
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_requires_token() {
        let server = serve(
            1024,
            Arc::new(|req: Request| Ok(json!({ "path": req.path, "body": req.body }))),
        )
        .unwrap();
        let ok = request(
            &server.url,
            &server.token,
            "/x",
            Some(&json!({ "a": 1 })),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(ok["path"], "/x");
        assert_eq!(ok["body"]["a"], 1);
        assert!(request(
            &server.url,
            &"0".repeat(64),
            "/x",
            None,
            Duration::from_secs(2)
        )
        .is_err());
    }

    #[test]
    fn records_roundtrip() {
        let dir = std::env::temp_dir().join(format!("annotate-reg-{}", uuid::Uuid::new_v4()));
        let record = Record {
            id: uuid::Uuid::new_v4().to_string(),
            url: "http://127.0.0.1:1234".into(),
            token: random_token(),
            harness: "pi".into(),
            harness_name: "Pi".into(),
            project: "/tmp".into(),
            session: "s".into(),
            pid: 1,
            protocol: 1,
            legacy: false,
        };
        {
            let _guard = advertise(&record, &dir).unwrap();
            assert_eq!(records(&dir).len(), 1);
        }
        assert!(records(&dir).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
