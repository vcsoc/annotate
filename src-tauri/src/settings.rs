//! Persistent Console preferences (atomic JSON file in the app config dir).
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub countdown_seconds: u32,
    pub capture_mode: String,
    pub max_annotations: usize,
    pub default_deliver_as: String,
    pub capture_shortcut: String,
    pub global_shortcut: bool,
    pub console_layout: String,
    pub mark_tool: String,
    pub mark_overflow: String,
    /// Keep the Console running after a successful Send (otherwise it closes, as before).
    pub keep_open_after_send: bool,
    /// What the Copy buttons put on the clipboard.
    pub copy_format: String,
    /// Terminal used to start console harnesses ("" = automatic).
    pub terminal_app: String,
    /// Open WebUI address used when no desktop app is installed.
    pub open_webui_url: String,
    /// Working folder for harnesses that start a new session.
    pub project_dir: String,
    /// Last chosen destination id.
    pub destination: String,
    /// Delay before pasting into a desktop app, in milliseconds.
    pub paste_delay_ms: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            countdown_seconds: 1,
            capture_mode: "live".into(),
            max_annotations: 8,
            default_deliver_as: "followUp".into(),
            capture_shortcut: String::new(),
            global_shortcut: true,
            console_layout: "timeline".into(),
            mark_tool: "freehand".into(),
            mark_overflow: "clip".into(),
            keep_open_after_send: false,
            copy_format: "rich".into(),
            terminal_app: String::new(),
            open_webui_url: "http://localhost:3000".into(),
            project_dir: String::new(),
            destination: String::new(),
            paste_delay_ms: 900,
        }
    }
}

pub fn validate_shortcut(value: &str) -> Result<String, String> {
    if value.len() <= 128 {
        if value.trim().is_empty() {
            return Ok(String::new());
        }
        let mut parts: Vec<String> = value.split('+').map(|p| p.trim().to_string()).collect();
        let key = parts.pop().unwrap_or_default();
        if !parts.is_empty()
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphabetic()))
            && !key.is_empty()
            && key.chars().all(|c| ('!'..='~').contains(&c))
        {
            parts.push(key);
            return Ok(parts.join("+"));
        }
    }
    Err("Enter a shortcut with a modifier and key (e.g. CommandOrControl+Alt+A), or leave it blank for the launch default.".into())
}

fn one_of(value: &str, allowed: &[&str], message: &str) -> Result<String, String> {
    if allowed.contains(&value) {
        Ok(value.to_string())
    } else {
        Err(message.into())
    }
}

/// Validates a complete settings object. Mirrors the original Console rules.
pub fn validate(value: &Settings) -> Result<Settings, String> {
    if value.countdown_seconds > 30 {
        return Err("Countdown must be a whole number from 0 to 30 seconds.".into());
    }
    if !(1..=12).contains(&value.max_annotations) {
        return Err("Maximum annotations must be a whole number from 1 to 12.".into());
    }
    if value.paste_delay_ms > 10_000 {
        return Err("Paste delay must be at most 10 seconds.".into());
    }
    let url = value.open_webui_url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) || url.len() > 300 {
        return Err("Open WebUI address must start with http:// or https://".into());
    }
    if value.terminal_app.len() > 120
        || value.project_dir.len() > 4096
        || value.destination.len() > 200
    {
        return Err("Invalid settings value".into());
    }
    Ok(Settings {
        countdown_seconds: value.countdown_seconds,
        capture_mode: one_of(
            &value.capture_mode,
            &["live", "preserved"],
            "Choose Live marking or Preserved frame.",
        )?,
        max_annotations: value.max_annotations,
        default_deliver_as: one_of(
            &value.default_deliver_as,
            &["followUp", "steer"],
            "Choose Follow-up or Queued (steering) as the default.",
        )?,
        capture_shortcut: validate_shortcut(&value.capture_shortcut)?,
        global_shortcut: value.global_shortcut,
        console_layout: one_of(
            &value.console_layout,
            &["timeline", "pill"],
            "Choose the Timeline feed or Command pill design.",
        )?,
        mark_tool: one_of(
            &value.mark_tool,
            &["freehand", "rectangle"],
            "Choose Freehand or Area as the default mark tool.",
        )?,
        mark_overflow: one_of(
            &value.mark_overflow,
            &["clip", "extend"],
            "Choose Trim or Extend for marks that cross the app edge.",
        )?,
        keep_open_after_send: value.keep_open_after_send,
        copy_format: one_of(
            &value.copy_format,
            &["rich", "image", "caption", "text"],
            "Choose a clipboard format.",
        )?,
        terminal_app: value.terminal_app.trim().to_string(),
        open_webui_url: url.trim_end_matches('/').to_string(),
        project_dir: value.project_dir.clone(),
        destination: value.destination.clone(),
        paste_delay_ms: value.paste_delay_ms,
    })
}

/// Merges a partial JSON object over `current` and validates the result.
pub fn merge(current: &Settings, patch: &Value) -> Result<Settings, String> {
    let mut base = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let Some(patch) = patch.as_object() else {
        return Err("Invalid settings".into());
    };
    for (key, value) in patch {
        if base.get(key).is_none() {
            return Err(format!("Unknown setting: {key}"));
        }
        base[key] = value.clone();
    }
    let next: Settings =
        serde_json::from_value(base).map_err(|_| "Invalid settings value".to_string())?;
    validate(&next)
}

/// Loads settings; unknown/corrupt fields fall back to defaults individually so a
/// bad manual edit never erases the other preferences.
pub fn load(file: &Path) -> Settings {
    let defaults = Settings::default();
    let Ok(text) = fs::read_to_string(file) else {
        return defaults;
    };
    let Ok(Value::Object(stored)) = serde_json::from_str::<Value>(&text) else {
        return defaults;
    };
    let mut current = defaults.clone();
    for (key, value) in stored {
        let patch = serde_json::json!({ key: value });
        if let Ok(next) = merge(&current, &patch) {
            current = next;
        }
    }
    current
}

pub fn save(file: &Path, value: &Settings) -> Result<Settings, String> {
    let settings = validate(value)?;
    write_atomic(
        file,
        (serde_json::to_string_pretty(&settings).unwrap() + "\n").as_bytes(),
    )?;
    Ok(settings)
}

pub fn write_atomic(file: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = file.parent().ok_or("Invalid settings path")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let temp = dir.join(format!(
        ".{}.{}.tmp",
        file.file_name().unwrap().to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let mut f = fs::File::create(&temp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&temp, file)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        assert_eq!(validate(&Settings::default()).unwrap(), Settings::default());
    }

    #[test]
    fn rejects_out_of_range() {
        let s = Settings {
            countdown_seconds: 31,
            ..Default::default()
        };
        assert!(validate(&s).is_err());
        let s = Settings {
            max_annotations: 13,
            ..Default::default()
        };
        assert!(validate(&s).is_err());
        let s = Settings {
            console_layout: "grid".into(),
            ..Default::default()
        };
        assert!(validate(&s).is_err());
    }

    #[test]
    fn shortcut_validation() {
        assert_eq!(
            validate_shortcut("CommandOrControl + Shift + A").unwrap(),
            "CommandOrControl+Shift+A"
        );
        assert_eq!(validate_shortcut("  ").unwrap(), "");
        assert!(validate_shortcut("A").is_err());
        assert!(validate_shortcut("Ctrl1+A").is_err());
    }

    #[test]
    fn merge_rejects_unknown_keys() {
        assert!(merge(&Settings::default(), &serde_json::json!({ "bogus": 1 })).is_err());
        let next = merge(
            &Settings::default(),
            &serde_json::json!({ "maxAnnotations": 4 }),
        )
        .unwrap();
        assert_eq!(next.max_annotations, 4);
    }

    #[test]
    fn load_keeps_valid_fields_when_others_are_corrupt() {
        let dir = std::env::temp_dir().join(format!("annotate-settings-{}", uuid::Uuid::new_v4()));
        let file = dir.join("settings.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &file,
            r#"{"maxAnnotations":5,"captureShortcut":"nonsense","consoleLayout":"pill"}"#,
        )
        .unwrap();
        let s = load(&file);
        assert_eq!(s.max_annotations, 5);
        assert_eq!(s.console_layout, "pill");
        assert_eq!(s.capture_shortcut, "");
        save(&file, &s).unwrap();
        assert_eq!(load(&file), s);
        fs::remove_dir_all(dir).unwrap();
    }
}
