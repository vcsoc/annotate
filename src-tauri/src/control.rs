//! Control channel of the running Console (`~/.annotate/app.json`). The CLI,
//! the MCP server and integrations use it to open/retarget the Console, start a
//! capture from a compositor keybinding, or quit.
use crate::bridge;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize)]
pub struct AppRecord {
    pub url: String,
    pub token: String,
    pub pid: u32,
}

pub fn read() -> Option<AppRecord> {
    let text = std::fs::read_to_string(crate::paths::app_record()).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write(record: &AppRecord) -> Result<(), String> {
    crate::paths::ensure_private_dir(&crate::paths::home()).map_err(|e| e.to_string())?;
    crate::settings::write_atomic(
        &crate::paths::app_record(),
        serde_json::to_string(record).unwrap().as_bytes(),
    )
}

pub fn remove(token: &str) {
    if read().is_some_and(|r| r.token == token) {
        let _ = std::fs::remove_file(crate::paths::app_record());
    }
}

/// Sends a command to the running Console, if any.
pub fn send(path: &str, body: &Value) -> Result<Value, String> {
    let record = read().ok_or("Annotate Console is not running")?;
    bridge::request(
        &record.url,
        &record.token,
        path,
        Some(body),
        Duration::from_secs(5),
    )
}

pub fn running() -> bool {
    read().is_some_and(|r| {
        bridge::request(
            &r.url,
            &r.token,
            "/status",
            None,
            Duration::from_millis(800),
        )
        .is_ok()
    })
}

/// Sends a command, launching the Console first when it is not running.
pub fn send_or_launch(path: &str, body: &Value) -> Result<Value, String> {
    if let Ok(value) = send(path, body) {
        return Ok(value);
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    #[allow(unused_mut)]
    let mut command = std::process::Command::new(exe);
    command
        .arg("gui")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0008 | 0x0000_0200); // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .spawn()
        .map_err(|e| format!("Cannot start Annotate Console: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
        if let Ok(value) = send(path, body) {
            return Ok(value);
        }
    }
    Err("Annotate Console did not start".into())
}
