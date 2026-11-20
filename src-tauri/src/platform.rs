//! Small OS-specific helpers.
use std::process::Command;

/// A command that never flashes a console window on Windows.
pub fn hidden_command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

/// Runs a command with a timeout, returning stdout bytes.
pub fn run(program: &str, args: &[&str], timeout_ms: u64) -> Result<Vec<u8>, String> {
    let child = hidden_command(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("{program} is not installed")
            } else {
                format!("{program}: {e}")
            }
        })?;
    let id = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(std::time::Duration::from_millis(timeout_ms)) {
        Ok(Ok(output)) if output.status.success() => Ok(output.stdout),
        Ok(Ok(output)) => {
            let err = String::from_utf8_lossy(&output.stderr);
            Err(format!(
                "{program} failed: {}",
                err.trim().chars().take(400).collect::<String>()
            ))
        }
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => {
            kill(id);
            Err(format!("{program} timed out"))
        }
    }
}

fn kill(pid: u32) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
    #[cfg(windows)]
    {
        let _ = hidden_command("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status();
    }
}

#[cfg(target_os = "macos")]
mod mac {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        pub fn CGPreflightScreenCaptureAccess() -> bool;
        pub fn CGRequestScreenCaptureAccess() -> bool;
    }
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        pub fn AXIsProcessTrusted() -> bool;
    }
}

/// macOS Screen Recording permission (always true elsewhere).
pub fn screen_capture_allowed(request: bool) -> bool {
    #[cfg(target_os = "macos")]
    unsafe {
        if mac::CGPreflightScreenCaptureAccess() {
            return true;
        }
        if request {
            return mac::CGRequestScreenCaptureAccess();
        }
        false
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = request;
        true
    }
}

/// macOS Accessibility permission, needed to press ⌘V in another app.
pub fn accessibility_allowed() -> bool {
    #[cfg(target_os = "macos")]
    unsafe {
        mac::AXIsProcessTrusted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

pub fn open_accessibility_settings() {
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
    }
}

/// Raises a Tauri window above fullscreen apps and onto every Space (macOS).
#[allow(unused_variables)]
pub fn float_over_fullscreen(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;
        if let Ok(ns) = window.ns_window() {
            let ns = ns as *mut AnyObject;
            if !ns.is_null() {
                // NSScreenSaverWindowLevel; canJoinAllSpaces | fullScreenAuxiliary | stationary.
                let window = window.clone();
                let ptr = ns as usize;
                let _ = window.run_on_main_thread(move || unsafe {
                    let ns = ptr as *mut AnyObject;
                    let _: () = msg_send![ns, setLevel: 1000isize];
                    let _: () = msg_send![ns, setCollectionBehavior: (1usize << 0) | (1usize << 8) | (1usize << 4)];
                });
            }
        }
    }
}

/// Display server family on Linux.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Desktop {
    Native,
    Hyprland,
    Sway,
    /// Other Wayland compositors (GNOME, KDE…): screenshot portal + in-image editor.
    Portal,
}

pub fn desktop() -> Desktop {
    if !cfg!(target_os = "linux") {
        return Desktop::Native;
    }
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let wayland = !env("WAYLAND_DISPLAY").is_empty();
    if !wayland {
        return Desktop::Native;
    }
    let current = env("XDG_CURRENT_DESKTOP").to_lowercase();
    if current.contains("hyprland")
        || (current.is_empty() && !env("HYPRLAND_INSTANCE_SIGNATURE").is_empty())
    {
        Desktop::Hyprland
    } else if current.contains("sway") || (current.is_empty() && !env("SWAYSOCK").is_empty()) {
        Desktop::Sway
    } else {
        Desktop::Portal
    }
}

pub fn wlroots() -> bool {
    matches!(desktop(), Desktop::Hyprland | Desktop::Sway)
}
