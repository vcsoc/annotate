//! Shared locations. Everything cross-harness lives under `~/.annotate`
//! (override with ANNOTATE_HOME), so integrations in any language can find it.
use std::path::PathBuf;

pub fn home() -> PathBuf {
    if let Some(dir) = std::env::var_os("ANNOTATE_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".annotate")
}

/// Live session records written by integrations (Pi extension, OpenCode plugin, MCP server…).
pub fn sessions() -> PathBuf {
    std::env::var_os("ANNOTATE_REGISTRY_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join("sessions"))
}

/// Registry of the original pi-annotate extension, still supported as a destination.
pub fn legacy_sessions() -> PathBuf {
    if let Some(dir) = std::env::var_os("PI_ANNOTATE_REGISTRY_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    let agent = std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_default()
                .join(".pi")
                .join("agent")
        });
    agent.join("annotate").join("sessions")
}

/// Control record of the running Console app.
pub fn app_record() -> PathBuf {
    home().join("app.json")
}

/// Optional user-defined harnesses.
pub fn custom_harnesses() -> PathBuf {
    home().join("harnesses.json")
}

pub fn ensure_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Absolute path of this executable (used when installing integrations).
pub fn current_exe() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "annotate".into())
}
