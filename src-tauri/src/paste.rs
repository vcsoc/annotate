//! Clipboard and "paste into another app" delivery for desktop/browser harnesses.
use crate::batch::{decode_png, Package};
use crate::platform::{self, Desktop};
use std::borrow::Cow;
use std::thread::sleep;
use std::time::Duration;

pub fn set_text(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text.to_string()))
        .map_err(|e| format!("Clipboard: {e}"))
}

pub fn set_image_b64(b64: &str) -> Result<(), String> {
    let img = decode_png(b64)?;
    let (width, height) = img.dimensions();
    arboard::Clipboard::new()
        .and_then(|mut c| {
            c.set_image(arboard::ImageData {
                width: width as usize,
                height: height as usize,
                bytes: Cow::Owned(img.into_raw()),
            })
        })
        .map_err(|e| format!("Clipboard: {e}"))
}

pub fn set_html(html: &str, alt: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_html(html, Some(alt)))
        .map_err(|e| format!("Clipboard: {e}"))
}

fn previous_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

/// Brings an installed desktop app to the front (launching it if needed).
pub fn activate(location: &str, names: &[String]) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = names;
        platform::run("open", &[location], 15_000)?;
        // `open` returns before activation finishes; give the window time to take focus.
        return Ok(());
    }
    #[cfg(windows)]
    {
        let _ = platform::hidden_command("explorer.exe")
            .arg(format!("shell:AppsFolder\\{location}"))
            .spawn()
            .map_err(|e| e.to_string())?;
        sleep(Duration::from_millis(600));
        if let Some(name) = names.first() {
            let script = format!(
                "(New-Object -ComObject WScript.Shell).AppActivate('{}') | Out-Null",
                name.replace('\'', "''")
            );
            let _ = platform::run(
                "powershell.exe",
                &["-NoProfile", "-NonInteractive", "-Command", &script],
                5000,
            );
        }
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = names;
        let id = std::path::Path::new(location)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        if platform::run("gtk-launch", &[&id], 10_000).is_err() {
            platform::run("gio", &["launch", location], 10_000)?;
        }
        Ok(())
    }
}

/// Presses a key chord such as "mod+v", "mod+l", "enter" in the focused app.
pub fn press(chord: &str) -> Result<(), String> {
    if platform::desktop() != Desktop::Native {
        return press_wayland(chord);
    }
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| format!("Keyboard automation unavailable: {e}"))?;
    let mut mods = vec![];
    let mut key = None;
    for part in chord.split('+').map(|p| p.trim().to_lowercase()) {
        match part.as_str() {
            "mod" => mods.push(if cfg!(target_os = "macos") {
                Key::Meta
            } else {
                Key::Control
            }),
            "cmd" | "meta" | "super" => mods.push(Key::Meta),
            "ctrl" | "control" => mods.push(Key::Control),
            "shift" => mods.push(Key::Shift),
            "alt" | "option" => mods.push(Key::Alt),
            "enter" | "return" => key = Some(Key::Return),
            k if k.chars().count() == 1 => key = Some(Key::Unicode(k.chars().next().unwrap())),
            other => return Err(format!("Unsupported key: {other}")),
        }
    }
    let key = key.ok_or("Missing key")?;
    for m in &mods {
        enigo.key(*m, Direction::Press).map_err(|e| e.to_string())?;
    }
    let result = enigo.key(key, Direction::Click).map_err(|e| e.to_string());
    for m in mods.iter().rev() {
        let _ = enigo.key(*m, Direction::Release);
    }
    result
}

fn press_wayland(chord: &str) -> Result<(), String> {
    // wtype works on wlroots compositors; other Wayland desktops block synthetic input.
    let mut args: Vec<String> = vec![];
    let mut key = String::new();
    for part in chord.split('+').map(|p| p.trim().to_lowercase()) {
        match part.as_str() {
            "mod" | "ctrl" | "control" => args.extend(["-M".into(), "ctrl".into()]),
            "shift" => args.extend(["-M".into(), "shift".into()]),
            "alt" => args.extend(["-M".into(), "alt".into()]),
            "super" | "cmd" | "meta" => args.extend(["-M".into(), "logo".into()]),
            "enter" | "return" => key = "Return".into(),
            k => key = k.to_string(),
        }
    }
    if key.len() == 1 {
        args.push(key);
    } else {
        args.extend(["-k".into(), key]);
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    platform::run("wtype", &refs, 5000)
        .map(|_| ())
        .map_err(|e| format!("{e}. Install wtype to paste automatically on this Wayland desktop."))
}

/// Pastes a batch into the focused app: the combined text, then each screenshot.
/// `submit` presses Enter at the end.
pub fn paste_package(
    package: &Package,
    prepare_keys: &str,
    delay_ms: u32,
    submit: bool,
) -> Result<(), String> {
    if !platform::accessibility_allowed() {
        platform::open_accessibility_settings();
        return Err("Allow Annotate in System Settings → Privacy & Security → Accessibility so it can paste into the app, then Send again. Nothing was pasted.".into());
    }
    let delay = Duration::from_millis(delay_ms.max(300) as u64);
    let restore = previous_text();
    sleep(delay);
    if !prepare_keys.is_empty() {
        press(prepare_keys)?;
        sleep(Duration::from_millis(500));
    }
    set_text(&package.chat_text())?;
    sleep(Duration::from_millis(120));
    press("mod+v")?;
    for item in &package.items {
        sleep(Duration::from_millis(450));
        set_image_b64(&item.image)?;
        sleep(Duration::from_millis(120));
        press("mod+v")?;
    }
    if submit {
        // Let the app finish attaching the images before sending.
        sleep(Duration::from_millis(
            1200 + 300 * package.items.len() as u64,
        ));
        press("enter")?;
    }
    sleep(Duration::from_millis(400));
    if let Some(text) = restore {
        let _ = set_text(&text);
    }
    Ok(())
}
