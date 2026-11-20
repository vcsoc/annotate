//! Delivery adapters. Every destination receives the same saved package.
use crate::batch::{self, Package};
use crate::bridge::{self, Record};
use crate::harness::{self, Destination, Kind};
use crate::model::Draft;
use crate::settings::Settings;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

pub struct Outcome {
    pub message: String,
    pub saved_to: String,
}

/// Delivers drafts to a destination. `record` is set for live sessions.
pub fn deliver(
    drafts: &[Draft],
    batch_id: &str,
    destination: &Destination,
    record: Option<&Record>,
    deliver_as: &str,
    settings: &Settings,
) -> Result<Outcome, String> {
    if !destination.modes.iter().any(|m| m.id == deliver_as) {
        return Err("Invalid delivery mode for this destination".into());
    }
    let folder_project = || -> Result<String, String> {
        let dir = settings.project_dir.trim();
        if dir.is_empty() {
            return Err(
                "Choose a project folder for this destination first (📁 under the destination)."
                    .into(),
            );
        }
        if !Path::new(dir).is_dir() {
            return Err(format!("The project folder no longer exists: {dir}"));
        }
        Ok(dir.to_string())
    };
    match destination.group.as_str() {
        "session" => {
            let record =
                record.ok_or("Selected session is offline. Choose an active destination.")?;
            if record.legacy {
                return deliver_legacy(drafts, batch_id, destination, record, deliver_as);
            }
            let package = batch::write_package(
                drafts,
                batch_id,
                deliver_as,
                &record.project,
                &destination.harness,
            )?;
            let value = serde_json::to_value(&package).unwrap();
            let result = bridge::request(
                &record.url,
                &record.token,
                "/deliver",
                Some(&value),
                Duration::from_secs(60),
            )?;
            if result["ok"] != json!(true) {
                return Err(result["error"]
                    .as_str()
                    .unwrap_or("The destination did not accept the batch")
                    .to_string());
            }
            Ok(Outcome {
                message: format!(
                    "Sent {} annotation{} to {}.",
                    package.items.len(),
                    plural(package.items.len()),
                    destination.harness_name
                ),
                saved_to: package.saved_to,
            })
        }
        "terminal" => {
            let project = folder_project()?;
            let package =
                batch::write_package(drafts, batch_id, deliver_as, &project, &destination.harness)?;
            let h = harness::find(&destination.harness).ok_or("Unknown harness")?;
            launch_terminal(&h, &package, &project, &settings.terminal_app)?;
            Ok(Outcome {
                message: format!(
                    "Started {} in a new terminal with {} annotation{}.",
                    h.name,
                    package.items.len(),
                    plural(package.items.len())
                ),
                saved_to: package.saved_to,
            })
        }
        "desktop" | "web" => {
            let h = harness::find(&destination.harness).ok_or("Unknown harness")?;
            let package =
                batch::write_package(drafts, batch_id, deliver_as, "", &destination.harness)?;
            if h.kind == Kind::Web {
                let url = if h.url.is_empty() {
                    settings.open_webui_url.clone()
                } else {
                    h.url.clone()
                };
                open_path(&url)?;
                std::thread::sleep(Duration::from_millis(2500));
            } else {
                let installed = harness::installed();
                let location = installed
                    .get(&h.id)
                    .map(|i| i.location.clone())
                    .ok_or_else(|| format!("{} is not installed", h.name))?;
                crate::paste::activate(&location, &h.app)?;
            }
            crate::paste::paste_package(
                &package,
                &h.keys,
                settings.paste_delay_ms,
                deliver_as == "pasteSend",
            )?;
            let verb = if deliver_as == "pasteSend" {
                "Sent"
            } else {
                "Pasted"
            };
            Ok(Outcome {
                message: format!(
                    "{verb} {} annotation{} into {}. Files: {}",
                    package.items.len(),
                    plural(package.items.len()),
                    h.name,
                    package.saved_to
                ),
                saved_to: package.saved_to,
            })
        }
        "folder" => {
            let project = folder_project()?;
            let package = batch::write_package(drafts, batch_id, deliver_as, &project, "folder")?;
            let _ = open_path(&package.saved_to);
            Ok(Outcome {
                message: format!(
                    "Saved {} annotation{} to {}.",
                    package.items.len(),
                    plural(package.items.len()),
                    package.saved_to
                ),
                saved_to: package.saved_to,
            })
        }
        _ => Err("Unknown destination".into()),
    }
}

/// Original pi-annotate sessions accept the earlier batch format and save it themselves.
fn deliver_legacy(
    drafts: &[Draft],
    batch_id: &str,
    destination: &Destination,
    record: &Record,
    deliver_as: &str,
) -> Result<Outcome, String> {
    let items: Vec<Value> = drafts
        .iter()
        .map(|d| {
            let mut item = json!({ "image": d.image, "comment": d.comment.trim(), "kind": d.kind, "points": d.points, "source": d.source });
            if d.marks.len() > 1 {
                item["marks"] = json!(d.marks.iter().map(|m| json!({ "kind": m.kind, "points": m.points, "comment": m.comment.trim() })).collect::<Vec<_>>());
            }
            item
        })
        .collect();
    let body = json!({ "id": batch_id, "items": items, "deliverAs": deliver_as });
    let result = bridge::request(
        &record.url,
        &record.token,
        "/deliver",
        Some(&body),
        Duration::from_secs(60),
    )?;
    let saved_to = result["savedTo"].as_str().unwrap_or("").to_string();
    Ok(Outcome {
        message: format!(
            "Sent {} annotation{} to {}.",
            drafts.len(),
            plural(drafts.len()),
            destination.harness_name
        ),
        saved_to,
    })
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

pub fn open_path(target: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(target).spawn();
    #[cfg(windows)]
    let result = crate::platform::hidden_command("explorer.exe")
        .arg(target)
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(target).spawn();
    result
        .map(|_| ())
        .map_err(|e| format!("Cannot open {target}: {e}"))
}

/// One-line request handed to a console harness. Points at ANNOTATIONS.md so it
/// survives every shell and works for agents that read images with a file tool.
pub fn terminal_prompt(package: &Package) -> String {
    format!(
        "Please act on the {} UI annotation{} saved in {}: read ANNOTATIONS.md there and view each referenced screenshot (the whole app with the requested area highlighted), then make the requested changes in this project.",
        package.items.len(),
        plural(package.items.len()),
        package.saved_to
    )
}

#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(windows)]
fn cmd_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\"").replace('%', "%%"))
}

pub fn launch_terminal(
    h: &harness::Harness,
    package: &Package,
    project: &str,
    terminal_app: &str,
) -> Result<(), String> {
    let prompt = terminal_prompt(package);
    let argv: Vec<String> = h
        .command
        .iter()
        .map(|a| a.replace("{prompt}", &prompt))
        .collect();
    // Resolve the binary so the new terminal does not depend on its PATH.
    let mut argv = argv;
    if let Some(bin) = harness::which_bin(&argv[0]) {
        argv[0] = bin.display().to_string();
    }
    let folder = Path::new(&package.saved_to);
    #[cfg(unix)]
    {
        let script_path = folder.join(if cfg!(target_os = "macos") {
            "run.command"
        } else {
            "run.sh"
        });
        let command = argv
            .iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" ");
        let script = format!(
            "#!/bin/sh\n# Started by Annotate for {}\ncd {} || exit 1\nexec {}\n",
            h.name,
            shell_quote(project),
            command
        );
        std::fs::write(&script_path, script).map_err(|e| e.to_string())?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        let script = script_path.display().to_string();
        #[cfg(target_os = "macos")]
        {
            let app = if terminal_app.is_empty() {
                "Terminal"
            } else {
                terminal_app
            };
            let status = match app.to_lowercase().as_str() {
                "ghostty" => std::process::Command::new("open")
                    .args(["-na", "Ghostty", "--args", "-e", &script])
                    .status(),
                "kitty" => std::process::Command::new("open")
                    .args(["-na", "kitty", "--args", &script])
                    .status(),
                "wezterm" => std::process::Command::new("open")
                    .args(["-na", "WezTerm", "--args", "start", "--", &script])
                    .status(),
                "alacritty" => std::process::Command::new("open")
                    .args(["-na", "Alacritty", "--args", "-e", &script])
                    .status(),
                _ => std::process::Command::new("open")
                    .args(["-a", app, &script])
                    .status(),
            };
            return match status {
                Ok(s) if s.success() => Ok(()),
                _ => Err(format!(
                    "Could not open {app}. Choose another terminal in Settings."
                )),
            };
        }
        #[cfg(not(target_os = "macos"))]
        {
            let mut candidates: Vec<String> = vec![];
            if !terminal_app.is_empty() {
                candidates.push(terminal_app.to_string());
            }
            if let Ok(t) = std::env::var("TERMINAL") {
                candidates.push(t);
            }
            candidates.extend(
                [
                    "x-terminal-emulator",
                    "kitty",
                    "alacritty",
                    "foot",
                    "wezterm",
                    "ghostty",
                    "gnome-terminal",
                    "konsole",
                    "xfce4-terminal",
                    "xterm",
                ]
                .iter()
                .map(|s| s.to_string()),
            );
            for term in candidates {
                let Some(path) = harness::which_bin(&term) else {
                    continue;
                };
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let args: Vec<&str> = match name.as_str() {
                    "gnome-terminal" | "kgx" => vec!["--", "sh", &script],
                    "wezterm" => vec!["start", "--", "sh", &script],
                    "kitty" | "foot" => vec!["sh", &script],
                    _ => vec!["-e", "sh", &script],
                };
                if std::process::Command::new(&path)
                    .args(&args)
                    .current_dir(project)
                    .spawn()
                    .is_ok()
                {
                    return Ok(());
                }
            }
            Err("No terminal emulator found. Set one in Settings → Harnesses → Terminal.".into())
        }
    }
    #[cfg(windows)]
    {
        let script_path = folder.join("run.cmd");
        let command = argv
            .iter()
            .map(|a| cmd_quote(a))
            .collect::<Vec<_>>()
            .join(" ");
        let script = format!(
            "@echo off\r\nREM Started by Annotate for {}\r\ncd /d {}\r\n{}\r\n",
            h.name,
            cmd_quote(project),
            command
        );
        std::fs::write(&script_path, script).map_err(|e| e.to_string())?;
        let script = script_path.display().to_string();
        let wt = if terminal_app.is_empty() || terminal_app.eq_ignore_ascii_case("wt") {
            harness::which_bin("wt")
        } else {
            None
        };
        let result = match wt {
            Some(wt) => std::process::Command::new(wt)
                .args(["-d", project, "cmd", "/k", &script])
                .spawn(),
            None => std::process::Command::new("cmd")
                .args(["/c", "start", "Annotate", "cmd", "/k", &script])
                .current_dir(project)
                .spawn(),
        };
        result
            .map(|_| ())
            .map_err(|e| format!("Could not open a terminal: {e}"))
    }
}

/// Status payload a Rust-side receiver (MCP / wait) returns from GET /session.
pub fn session_status(record: &Record, name: &str, modes: Value) -> Value {
    json!({
        "targetId": record.id,
        "harness": record.harness,
        "harnessName": record.harness_name,
        "project": record.project,
        "session": record.session,
        "name": name,
        "deliveryModes": modes,
        "protocol": 1
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run with a legacy pi-annotate bridge advertised in PI_ANNOTATE_REGISTRY_DIR:
    /// `cargo test legacy -- --ignored`
    #[test]
    #[ignore]
    fn legacy_pi_annotate_delivery() {
        let (record, destination) = harness::live_sessions()
            .into_iter()
            .find(|(r, _)| r.legacy)
            .expect("legacy session");
        let png = crate::batch::encode_png(&image::RgbaImage::from_pixel(
            8,
            6,
            image::Rgba([255, 0, 0, 255]),
        ));
        use base64::Engine;
        let mark = |c: &str| crate::model::Mark {
            kind: "rectangle".into(),
            points: vec![[0.1, 0.1], [0.5, 0.5]],
            comment: c.into(),
        };
        let draft = Draft {
            id: "x".into(),
            image: base64::engine::general_purpose::STANDARD.encode(png),
            base: None,
            preview: String::new(),
            comment: "1. one\n2. two".into(),
            kind: "rectangle".into(),
            points: vec![[0.1, 0.1], [0.5, 0.5]],
            marks: vec![mark("one"), mark("two")],
            width: 8,
            height: 6,
            source: "Test".into(),
        };
        let outcome = deliver(
            &[draft],
            &uuid::Uuid::new_v4().to_string(),
            &destination,
            Some(&record),
            "steer",
            &Settings::default(),
        )
        .unwrap();
        println!("{} {}", outcome.message, outcome.saved_to);
    }
}
