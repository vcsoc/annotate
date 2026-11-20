//! Capture shortcut: OS global shortcut, a temporary compositor binding on
//! Hyprland/sway (which run `annotate capture`), or Console-only.
use crate::app::{self, lock};
use crate::platform::{self, Desktop};
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

fn trigger(app: &AppHandle) {
    let ready = {
        let inner = lock(app);
        inner.shortcut.ready && !inner.shortcut.recording && !inner.busy()
    };
    if ready {
        crate::flow::spawn_capture(app, crate::flow::CaptureOptions::default());
    }
}

/// Hyprland/sway binding spec: ("SUPER SHIFT", "A") / "Mod4+Shift+a".
fn compositor_chord(value: &str) -> Result<(String, String), String> {
    let mut parts: Vec<&str> = value.split('+').map(str::trim).collect();
    let key = parts.pop().unwrap_or("");
    let mut mods = vec![];
    for part in parts {
        let m = match part.to_lowercase().as_str() {
            "ctrl" | "control" | "commandorcontrol" | "cmdorctrl" => "CTRL",
            "shift" => "SHIFT",
            "alt" | "option" => "ALT",
            "super" | "meta" | "cmd" | "command" => "SUPER",
            other => return Err(format!("Unsupported shortcut modifier: {other}")),
        };
        if !mods.contains(&m) {
            mods.push(m);
        }
    }
    let valid = key.len() == 1 && key.chars().all(|c| c.is_ascii_alphanumeric())
        || (key.len() <= 3 && key.starts_with('F') && key[1..].parse::<u8>().is_ok());
    if mods.is_empty() || !valid {
        return Err("Use a modified letter, number or function key for the Annotate shortcut on this compositor".into());
    }
    mods.sort();
    Ok((mods.join(" "), key.to_uppercase()))
}

fn bind_compositor(value: &str) -> Result<String, String> {
    let (mods, key) = compositor_chord(value)?;
    let exe = crate::paths::current_exe();
    match platform::desktop() {
        Desktop::Hyprland => {
            let binds: serde_json::Value =
                serde_json::from_slice(&platform::run("hyprctl", &["-j", "binds"], 3000)?)
                    .unwrap_or_default();
            let mask = mods
                .split(' ')
                .map(|m| match m {
                    "SHIFT" => 1,
                    "CTRL" => 4,
                    "ALT" => 8,
                    "SUPER" => 64,
                    _ => 0,
                })
                .sum::<u64>();
            if binds.as_array().into_iter().flatten().any(|b| {
                b["modmask"].as_u64() == Some(mask)
                    && b["key"]
                        .as_str()
                        .is_some_and(|k| k.eq_ignore_ascii_case(&key))
            }) {
                return Err(format!(
                    "{value} is already bound in Hyprland. Choose another capture shortcut."
                ));
            }
            let spec = format!(
                "{mods}, {key}, exec, '{}' capture",
                exe.replace('\'', "'\\''")
            );
            let out = String::from_utf8_lossy(&platform::run(
                "hyprctl",
                &["keyword", "bind", &spec],
                3000,
            )?)
            .to_string();
            if out.to_lowercase().contains("error") || out.to_lowercase().contains("invalid") {
                return Err(format!("Hyprland refused the shortcut: {}", out.trim()));
            }
            Ok(format!("{mods}, {key}"))
        }
        Desktop::Sway => {
            let chord = format!(
                "{}+{}",
                mods.replace("SUPER", "Mod4")
                    .replace("CTRL", "Ctrl")
                    .replace("SHIFT", "Shift")
                    .replace("ALT", "Mod1")
                    .replace(' ', "+"),
                key.to_lowercase()
            );
            platform::run(
                "swaymsg",
                &[
                    "bindsym",
                    "--no-warn",
                    &chord,
                    &format!("exec '{}' capture", exe.replace('\'', "'\\''")),
                ],
                3000,
            )?;
            Ok(chord)
        }
        _ => Err("No compositor binding on this desktop".into()),
    }
}

fn unbind_compositor(binding: &str) {
    match platform::desktop() {
        Desktop::Hyprland => {
            let _ = platform::run("hyprctl", &["keyword", "unbind", binding], 3000);
        }
        Desktop::Sway => {
            let _ = platform::run("swaymsg", &["unbindsym", binding], 3000);
        }
        _ => {}
    }
}

fn unregister_current(app: &AppHandle) {
    let (current, global, compositor) = {
        let mut inner = lock(app);
        let s = &mut inner.shortcut;
        let out = (s.current.clone(), s.global, s.compositor.take());
        s.ready = false;
        out
    };
    if let Some(binding) = compositor {
        unbind_compositor(&binding);
    } else if global && !current.is_empty() {
        let _ = app.global_shortcut().unregister(current.as_str());
    }
}

fn register_os(app: &AppHandle, value: &str) -> Result<(), String> {
    app.global_shortcut()
        .on_shortcut(value, |app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                trigger(app);
            }
        })
        .map_err(|e| format!("Cannot register {value}. It may be invalid, reserved or already in use. Choose another capture shortcut. ({e})"))
}

/// Registers `value` (global or Console-only). On failure the previous shortcut stays active.
pub fn apply(app: &AppHandle, value: &str, global: bool) -> Result<(), String> {
    let (previous, previous_global) = {
        let inner = lock(app);
        (inner.shortcut.current.clone(), inner.shortcut.global)
    };
    let same = previous == value && previous_global == global && lock(app).shortcut.ready;
    if same {
        return Ok(());
    }
    unregister_current(app);
    let attempt = |value: &str, global: bool| -> Result<Option<String>, String> {
        if !global {
            return Ok(None);
        }
        if platform::wlroots() {
            return bind_compositor(value).map(Some);
        }
        register_os(app, value).map(|_| None)
    };
    match attempt(value, global) {
        Ok(compositor) => {
            let mut inner = lock(app);
            inner.shortcut = app::ShortcutState {
                current: value.into(),
                ready: true,
                global,
                error: None,
                recording: inner.shortcut.recording,
                updating: inner.shortcut.updating,
                compositor,
            };
            Ok(())
        }
        Err(error) => {
            // Restore the previous binding so the shortcut keeps working.
            if !previous.is_empty() {
                if let Ok(compositor) = attempt(&previous, previous_global) {
                    let mut inner = lock(app);
                    inner.shortcut.current = previous;
                    inner.shortcut.global = previous_global;
                    inner.shortcut.ready = true;
                    inner.shortcut.compositor = compositor;
                }
            }
            Err(error)
        }
    }
}

/// Startup registration; failures are reported but leave Capture usable.
pub fn init(app: &AppHandle) {
    let (value, global) = {
        let inner = lock(app);
        (inner.capture_shortcut(), inner.settings.global_shortcut)
    };
    if let Err(error) = apply(app, &value, global) {
        let mut inner = lock(app);
        inner.shortcut.current = value;
        inner.shortcut.global = global;
        inner.shortcut.error = Some(error.clone());
        drop(inner);
        app::status(app, error);
    }
}

/// While the Settings field records keys, release the OS grab so the keys reach it.
pub fn set_recording(app: &AppHandle, active: bool) {
    let (current, global, ready, compositor) = {
        let mut inner = lock(app);
        inner.shortcut.recording = active;
        (
            inner.shortcut.current.clone(),
            inner.shortcut.global,
            inner.shortcut.ready,
            inner.shortcut.compositor.is_some(),
        )
    };
    if !global || !ready || current.is_empty() {
        return;
    }
    if compositor {
        return; // compositor bindings run `annotate capture`, which checks `recording`.
    }
    if active {
        let _ = app.global_shortcut().unregister(current.as_str());
    } else if register_os(app, &current).is_err() {
        let mut inner = lock(app);
        inner.shortcut.ready = false;
        inner.shortcut.error = Some(format!(
            "Cannot re-register {current}. Choose another capture shortcut."
        ));
    }
}

pub fn console_trigger(app: &AppHandle) -> bool {
    let (ok, global) = {
        let inner = lock(app);
        (inner.shortcut.ready && !inner.busy(), inner.shortcut.global)
    };
    if !ok || global {
        return false;
    }
    trigger(app);
    true
}

/// External trigger (`annotate capture` from a compositor binding).
pub fn external_trigger(app: &AppHandle) {
    trigger(app);
}

/// Escape cancels desktop selection even when the selector is not key (macOS panels).
pub fn grab_escape(app: &AppHandle) -> bool {
    if platform::desktop() != Desktop::Native {
        return false;
    }
    app.global_shortcut()
        .on_shortcut("Escape", |app, _s, event| {
            if event.state() == ShortcutState::Pressed {
                crate::flow::cancel_selection(app);
            }
        })
        .is_ok()
}

pub fn release_escape(app: &AppHandle, grabbed: bool) {
    if grabbed {
        let _ = app.global_shortcut().unregister("Escape");
    }
}

pub fn release_all(app: &AppHandle) {
    unregister_current(app);
    let _ = app.global_shortcut().unregister_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compositor_chords() {
        assert_eq!(
            compositor_chord("CommandOrControl+Shift+A").unwrap(),
            ("CTRL SHIFT".into(), "A".into())
        );
        assert_eq!(
            compositor_chord("Super+F5").unwrap(),
            ("SUPER".into(), "F5".into())
        );
        assert!(compositor_chord("A").is_err());
        assert!(compositor_chord("Hyper+A").is_err());
    }
}
