//! Window discovery and screenshots for every supported desktop.
//!
//! Coordinates ("canonical space"):
//! * macOS — global points (CoreGraphics), shared by window lists and overlays.
//! * Windows / X11 — physical pixels of the virtual desktop.
//! * Hyprland / sway — compositor logical coordinates.
use crate::model::{AppWindow, Rect};
use crate::platform::{self, Desktop};
use image::RgbaImage;
use serde_json::Value;

fn clean(label: &str) -> String {
    label
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(160)
        .collect()
}

#[cfg(any(target_os = "macos", windows))]
fn from_rows(rows: &[Value], own_pid: u32) -> Vec<AppWindow> {
    rows.iter()
        .enumerate()
        .filter(|(_, w)| w["pid"].as_u64() != Some(own_pid as u64))
        .filter_map(|(i, w)| {
            let num = |k: &str| {
                w[k].as_f64()
                    .or_else(|| w[k].as_str().and_then(|s| s.parse().ok()))
            };
            let id = w["id"]
                .as_str()
                .map(String::from)
                .or_else(|| w["id"].as_u64().map(|n| n.to_string()))?;
            let win = AppWindow {
                x: num("x")?,
                y: num("y")?,
                width: num("width")?,
                height: num("height")?,
                label: clean(w["title"].as_str().unwrap_or("Application")),
                native_id: id,
                floating: false,
                focus_order: w["order"].as_i64().unwrap_or(i as i64),
            };
            (win.native_id.chars().all(|c| c.is_ascii_digit())
                && win.width > 1.0
                && win.height > 1.0)
                .then_some(win)
        })
        .collect()
}

/// Visible top-level windows of other applications, front-most first.
pub fn app_windows(own_pid: u32) -> Result<Vec<AppWindow>, String> {
    match platform::desktop() {
        Desktop::Hyprland | Desktop::Sway => wl::windows(own_pid),
        Desktop::Portal => Ok(vec![]),
        Desktop::Native => native_windows(own_pid),
    }
}

#[cfg(target_os = "macos")]
fn native_windows(own_pid: u32) -> Result<Vec<AppWindow>, String> {
    let script = include_str!("../native/windows-macos.js");
    let out = platform::run(
        "/usr/bin/osascript",
        &["-l", "JavaScript", "-e", script],
        15_000,
    )?;
    let rows: Vec<Value> = serde_json::from_slice(&out).map_err(|e| format!("Window list: {e}"))?;
    Ok(from_rows(&rows, own_pid))
}

#[cfg(windows)]
fn native_windows(own_pid: u32) -> Result<Vec<AppWindow>, String> {
    use base64::Engine;
    let script = include_str!("../native/windows-win32.ps1");
    let utf16: Vec<u8> = script
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    let out = platform::run(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded],
        15_000,
    )?;
    let text = String::from_utf8_lossy(&out);
    let rows: Vec<Value> = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("Window list: {e}"))?;
    Ok(from_rows(&rows, own_pid))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn native_windows(own_pid: u32) -> Result<Vec<AppWindow>, String> {
    let windows = xcap::Window::all().map_err(|e| format!("Window list: {e}"))?;
    let mut out: Vec<AppWindow> = windows
        .iter()
        .filter(|w| w.pid().ok() != Some(own_pid) && !w.is_minimized().unwrap_or(false))
        .filter_map(|w| {
            Some(AppWindow {
                x: w.x().ok()? as f64,
                y: w.y().ok()? as f64,
                width: w.width().ok()? as f64,
                height: w.height().ok()? as f64,
                label: clean(
                    &w.title()
                        .ok()
                        .filter(|t| !t.is_empty())
                        .or_else(|| w.app_name().ok())
                        .unwrap_or_else(|| "Application".into()),
                ),
                native_id: w.id().ok()?.to_string(),
                floating: false,
                focus_order: -(w.z().unwrap_or(0) as i64),
            })
        })
        .filter(|w| w.width > 1.0 && w.height > 1.0)
        .collect();
    out.sort_by_key(|w| w.focus_order);
    Ok(out)
}

/// Screenshot of exactly one native window (not used on wlroots).
pub fn capture_window(win: &AppWindow) -> Result<RgbaImage, String> {
    #[cfg(target_os = "macos")]
    {
        let dir = std::env::temp_dir().join(format!("annotate-window-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let file = dir.join("window.png");
        let result = platform::run(
            "/usr/sbin/screencapture",
            &[
                "-x",
                "-o",
                "-l",
                &win.native_id,
                "-t",
                "png",
                &file.display().to_string(),
            ],
            15_000,
        )
        .and_then(|_| {
            image::open(&file).map(|i| i.to_rgba8()).map_err(|e| {
                format!("Window capture failed: {e}. Allow Screen Recording for Annotate.")
            })
        });
        let _ = std::fs::remove_dir_all(&dir);
        result
    }
    #[cfg(not(target_os = "macos"))]
    {
        let id: u32 = win
            .native_id
            .parse()
            .map_err(|_| "Invalid window id".to_string())?;
        let window = xcap::Window::all()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|w| w.id().ok() == Some(id))
            .ok_or("The marked application is not available for capture. Bring the app onscreen, then retry.")?;
        window
            .capture_image()
            .map_err(|e| format!("Window capture failed: {e}"))
    }
}

/// Screenshot of a desktop rectangle (marks extended past their app window).
pub fn capture_rect(rect: &Rect) -> Result<RgbaImage, String> {
    #[cfg(target_os = "macos")]
    {
        let dir = std::env::temp_dir().join(format!("annotate-region-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let file = dir.join("region.png");
        let r = rect.round();
        let region = format!("{},{},{},{}", r.x, r.y, r.width, r.height);
        let result = platform::run(
            "/usr/sbin/screencapture",
            &[
                "-x",
                "-R",
                &region,
                "-t",
                "png",
                &file.display().to_string(),
            ],
            15_000,
        )
        .and_then(|_| {
            image::open(&file)
                .map(|i| i.to_rgba8())
                .map_err(|e| format!("Region capture failed: {e}"))
        });
        let _ = std::fs::remove_dir_all(&dir);
        result
    }
    #[cfg(not(target_os = "macos"))]
    {
        if platform::wlroots() {
            return wl::grim(rect);
        }
        let monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
        let monitor = monitors
            .into_iter()
            .find(|m| {
                let (x, y, w, h) = (m.x().unwrap_or(0) as f64, m.y().unwrap_or(0) as f64, m.width().unwrap_or(0) as f64, m.height().unwrap_or(0) as f64);
                rect.x >= x && rect.y >= y && rect.x + rect.width <= x + w && rect.y + rect.height <= y + h
            })
            .ok_or("The extended capture spans several displays. Keep the mark on one display, or choose Trim in Settings.")?;
        let image = monitor
            .capture_image()
            .map_err(|e| format!("Display capture failed: {e}"))?;
        let ratio = image.width() as f64 / monitor.width().unwrap_or(image.width()) as f64;
        let (mx, my) = (
            monitor.x().unwrap_or(0) as f64,
            monitor.y().unwrap_or(0) as f64,
        );
        let crop = |v: f64| (v * ratio).round().max(0.0) as u32;
        let (x, y) = (crop(rect.x - mx), crop(rect.y - my));
        let (w, h) = (
            crop(rect.width).min(image.width() - x),
            crop(rect.height).min(image.height() - y),
        );
        Ok(image::imageops::crop_imm(&image, x, y, w, h).to_image())
    }
}

/// Fallback for Wayland desktops without window geometry (GNOME, KDE…): a
/// screenshot of the display under the pointer, through xcap's portal support.
pub fn capture_display(point: Option<(f64, f64)>) -> Result<RgbaImage, String> {
    let monitors = xcap::Monitor::all().map_err(|e| format!("Display capture failed: {e}"))?;
    let monitor = point
        .and_then(|(px, py)| {
            monitors.iter().find(|m| {
                let (x, y, w, h) = (
                    m.x().unwrap_or(0) as f64,
                    m.y().unwrap_or(0) as f64,
                    m.width().unwrap_or(0) as f64,
                    m.height().unwrap_or(0) as f64,
                );
                px >= x && py >= y && px < x + w && py < y + h
            })
        })
        .or_else(|| monitors.iter().find(|m| m.is_primary().unwrap_or(false)))
        .or(monitors.first())
        .ok_or("No display is available")?;
    monitor.capture_image().map_err(|e| {
        format!("Display capture failed: {e}. Check the screenshot portal permission.")
    })
}

/// Hyprland / sway helpers (compositor IPC + grim/slurp), as in the original Console.
pub mod wl {
    use super::*;

    fn json(program: &str, args: &[&str]) -> Result<Value, String> {
        let out = platform::run(program, args, 3000)?;
        serde_json::from_slice(&out).map_err(|e| format!("{program}: {e}"))
    }

    fn hypr() -> bool {
        platform::desktop() == Desktop::Hyprland
    }

    pub fn windows(own_pid: u32) -> Result<Vec<AppWindow>, String> {
        if hypr() {
            let clients = json("hyprctl", &["-j", "clients"])?;
            let monitors = json("hyprctl", &["-j", "monitors"])?;
            let workspaces: Vec<i64> = monitors
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|m| {
                    [
                        m["activeWorkspace"]["id"].as_i64(),
                        m["specialWorkspace"]["id"].as_i64(),
                    ]
                })
                .flatten()
                .filter(|id| *id != 0)
                .collect();
            Ok(clients
                .as_array()
                .into_iter()
                .flatten()
                .filter(|c| {
                    c["mapped"] == true
                        && c["hidden"] != true
                        && c["pid"].as_u64() != Some(own_pid as u64)
                })
                .filter(|c| {
                    c["pinned"] == true
                        || c["workspace"]["id"]
                            .as_i64()
                            .is_some_and(|id| workspaces.contains(&id))
                })
                .filter_map(|c| {
                    let w = c["size"][0].as_f64()?;
                    let h = c["size"][1].as_f64()?;
                    (w > 1.0 && h > 1.0).then(|| AppWindow {
                        x: c["at"][0].as_f64().unwrap_or(0.0),
                        y: c["at"][1].as_f64().unwrap_or(0.0),
                        width: w,
                        height: h,
                        label: clean(
                            c["title"]
                                .as_str()
                                .filter(|t| !t.is_empty())
                                .or(c["class"].as_str())
                                .unwrap_or("Window"),
                        ),
                        native_id: c["address"].as_str().unwrap_or("").to_string(),
                        floating: c["floating"] == true,
                        focus_order: c["focusHistoryID"].as_i64().unwrap_or(99_999),
                    })
                })
                .collect())
        } else {
            let tree = json("swaymsg", &["-t", "get_tree", "-r"])?;
            let visible: Vec<i64> = json("swaymsg", &["-t", "get_workspaces", "-r"])?
                .as_array()
                .into_iter()
                .flatten()
                .filter(|w| w["visible"] == true)
                .filter_map(|w| w["id"].as_i64())
                .collect();
            let mut out = vec![];
            fn visit(
                node: &Value,
                active: bool,
                visible: &[i64],
                own_pid: u32,
                out: &mut Vec<AppWindow>,
            ) {
                let shown = active
                    && (node["type"] != "workspace"
                        || node["id"].as_i64().is_some_and(|id| visible.contains(&id)));
                let rect = &node["rect"];
                if shown
                    && node["pid"].as_u64().is_some_and(|p| p != own_pid as u64)
                    && (node["app_id"].is_string() || node["window_properties"].is_object())
                    && rect["width"].as_f64().unwrap_or(0.0) > 1.0
                    && rect["height"].as_f64().unwrap_or(0.0) > 1.0
                {
                    out.push(AppWindow {
                        x: rect["x"].as_f64().unwrap_or(0.0),
                        y: rect["y"].as_f64().unwrap_or(0.0),
                        width: rect["width"].as_f64().unwrap_or(0.0),
                        height: rect["height"].as_f64().unwrap_or(0.0),
                        label: clean(
                            node["name"]
                                .as_str()
                                .or(node["app_id"].as_str())
                                .unwrap_or("Window"),
                        ),
                        native_id: node["id"].to_string(),
                        floating: node["floating"].as_str().is_some_and(|f| f.ends_with("on")),
                        focus_order: if node["focused"] == true { 0 } else { 1 },
                    });
                }
                for child in node["nodes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .chain(node["floating_nodes"].as_array().into_iter().flatten())
                {
                    visit(child, shown, visible, own_pid, out);
                }
            }
            visit(&tree, true, &visible, own_pid, &mut out);
            Ok(out)
        }
    }

    /// Display bounds (logical), or work areas when `work` excludes reserved space.
    pub fn areas(work: bool) -> Result<Vec<Rect>, String> {
        let mut areas = vec![];
        if hypr() {
            let mut monitors: Vec<Value> = json("hyprctl", &["-j", "monitors"])?
                .as_array()
                .cloned()
                .unwrap_or_default();
            monitors.sort_by_key(|m| if m["focused"] == true { 0 } else { 1 });
            for m in monitors.iter().filter(|m| m["disabled"] != true) {
                let scale = m["scale"].as_f64().unwrap_or(1.0).max(0.1);
                let rotated = m["transform"].as_i64().unwrap_or(0) % 2 == 1;
                let (pw, ph) = (
                    m["width"].as_f64().unwrap_or(0.0),
                    m["height"].as_f64().unwrap_or(0.0),
                );
                let (w, h) = if rotated { (ph, pw) } else { (pw, ph) };
                let r: Vec<f64> = if work {
                    m["reserved"]
                        .as_array()
                        .map(|a| a.iter().filter_map(Value::as_f64).collect())
                        .unwrap_or_default()
                } else {
                    vec![]
                };
                let get = |i: usize| r.get(i).copied().unwrap_or(0.0);
                areas.push(Rect::new(
                    m["x"].as_f64().unwrap_or(0.0) + get(0),
                    m["y"].as_f64().unwrap_or(0.0) + get(1),
                    (w / scale).round() - get(0) - get(2),
                    (h / scale).round() - get(1) - get(3),
                ));
            }
        } else {
            for o in json("swaymsg", &["-t", "get_outputs", "-r"])?
                .as_array()
                .into_iter()
                .flatten()
                .filter(|o| o["active"] == true)
            {
                let r = &o["rect"];
                areas.push(Rect::new(
                    r["x"].as_f64().unwrap_or(0.0),
                    r["y"].as_f64().unwrap_or(0.0),
                    r["width"].as_f64().unwrap_or(0.0),
                    r["height"].as_f64().unwrap_or(0.0),
                ));
            }
        }
        if areas.is_empty()
            || areas.len() > 16
            || areas.iter().any(|a| a.width < 2.0 || a.height < 2.0)
        {
            return Err("Cannot determine valid display bounds".into());
        }
        Ok(areas)
    }

    pub fn geometry(r: &Rect) -> String {
        format!(
            "{},{} {}x{}",
            r.x.round(),
            r.y.round(),
            r.width.round(),
            r.height.round()
        )
    }

    /// Screenshot at one pixel per logical coordinate.
    pub fn grim(rect: &Rect) -> Result<RgbaImage, String> {
        let out = platform::run("grim", &["-s", "1", "-g", &geometry(rect), "-"], 15_000)?;
        image::load_from_memory(&out)
            .map(|i| i.to_rgba8())
            .map_err(|e| format!("grim: {e}"))
    }

    /// Native slurp area selection over the live desktop (window boxes preselectable).
    pub fn slurp(
        boxes: &[AppWindow],
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Rect, String> {
        use std::io::Write;
        let mut child = std::process::Command::new("slurp")
            .args([
                "-d",
                "-f",
                "%x,%y %wx%h",
                "-c",
                "#ff3b65",
                "-B",
                "#ff3b6520",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    "Install slurp to mark directly on the live desktop.".to_string()
                } else {
                    e.to_string()
                }
            })?;
        if let Some(mut stdin) = child.stdin.take() {
            let list: String = boxes
                .iter()
                .map(|b| format!("{} {}\n", geometry(&b.rect()), b.label))
                .collect();
            let _ = stdin.write_all(list.as_bytes());
        }
        let started = std::time::Instant::now();
        loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                let mut out = String::new();
                use std::io::Read;
                if let Some(mut stdout) = child.stdout.take() {
                    let _ = stdout.read_to_string(&mut out);
                }
                if !status.success() {
                    return Err("Capture cancelled".into());
                }
                return parse_geometry(&out);
            }
            if cancel.is_cancelled() || started.elapsed().as_secs() > 120 {
                let _ = child.kill();
                return Err("Capture cancelled".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
    }

    pub fn parse_geometry(value: &str) -> Result<Rect, String> {
        let bad = || "The region picker returned invalid geometry".to_string();
        let (pos, size) = value.trim().split_once(' ').ok_or_else(bad)?;
        let (x, y) = pos.split_once(',').ok_or_else(bad)?;
        let (w, h) = size.split_once('x').ok_or_else(bad)?;
        let n = |s: &str| s.parse::<f64>().map_err(|_| bad());
        let rect = Rect::new(n(x)?, n(y)?, n(w)?, n(h)?);
        if rect.width < 2.0 || rect.height < 2.0 || rect.width > 32768.0 || rect.height > 32768.0 {
            return Err("Choose a region at least 2×2 pixels".into());
        }
        Ok(rect)
    }

    /// Floats, sizes and positions one of our own windows (identified by pid + title).
    pub fn place(title: &str, own_pid: u32, rect: &Rect) -> Result<(), String> {
        for _ in 0..20 {
            if hypr() {
                let clients = json("hyprctl", &["-j", "clients"])?;
                if let Some(c) = clients.as_array().into_iter().flatten().find(|c| {
                    c["pid"].as_u64() == Some(own_pid as u64)
                        && c["title"].as_str() == Some(title)
                        && c["mapped"] == true
                }) {
                    let address = c["address"].as_str().unwrap_or("");
                    if !address.starts_with("0x") {
                        return Err("Invalid window address".into());
                    }
                    let selector = format!("address:{address}");
                    let r = rect.round();
                    let _ = platform::run("hyprctl", &["dispatch", "setfloating", &selector], 2000);
                    let _ = platform::run(
                        "hyprctl",
                        &[
                            "dispatch",
                            "resizewindowpixel",
                            &format!("exact {} {},{selector}", r.width, r.height),
                        ],
                        2000,
                    );
                    let _ = platform::run(
                        "hyprctl",
                        &[
                            "dispatch",
                            "movewindowpixel",
                            &format!("exact {} {},{selector}", r.x, r.y),
                        ],
                        2000,
                    );
                    return Ok(());
                }
            } else {
                let tree = json("swaymsg", &["-t", "get_tree", "-r"])?;
                fn find(node: &Value, pid: u32, title: &str) -> Option<i64> {
                    if node["pid"].as_u64() == Some(pid as u64)
                        && node["name"].as_str() == Some(title)
                    {
                        return node["id"].as_i64();
                    }
                    node["nodes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .chain(node["floating_nodes"].as_array().into_iter().flatten())
                        .find_map(|c| find(c, pid, title))
                }
                if let Some(id) = find(&tree, own_pid, title) {
                    let r = rect.round();
                    platform::run("swaymsg", &[&format!("[con_id={id}] floating enable, sticky enable, resize set {} px {} px, move absolute position {} px {} px", r.width, r.height, r.x, r.y)], 2000)?;
                    return Ok(());
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err("Could not find the annotation window in the compositor".into())
    }

    /// Actual bounds of one of our floating windows.
    pub fn bounds(title: &str, own_pid: u32) -> Option<Rect> {
        if hypr() {
            let clients = json("hyprctl", &["-j", "clients"]).ok()?;
            let c = clients.as_array()?.iter().find(|c| {
                c["pid"].as_u64() == Some(own_pid as u64)
                    && c["title"].as_str() == Some(title)
                    && c["mapped"] == true
            })?;
            Some(Rect::new(
                c["at"][0].as_f64()?,
                c["at"][1].as_f64()?,
                c["size"][0].as_f64()?,
                c["size"][1].as_f64()?,
            ))
        } else {
            let tree = json("swaymsg", &["-t", "get_tree", "-r"]).ok()?;
            fn find(node: &Value, pid: u32, title: &str) -> Option<Rect> {
                if node["pid"].as_u64() == Some(pid as u64) && node["name"].as_str() == Some(title)
                {
                    let r = &node["rect"];
                    return Some(Rect::new(
                        r["x"].as_f64()?,
                        r["y"].as_f64()?,
                        r["width"].as_f64()?,
                        r["height"].as_f64()?,
                    ));
                }
                node["nodes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .chain(node["floating_nodes"].as_array().into_iter().flatten())
                    .find_map(|c| find(c, pid, title))
            }
            find(&tree, own_pid, title)
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn parses_slurp() {
            let r = super::parse_geometry("-10,20 300x200\n").unwrap();
            assert_eq!((r.x, r.y, r.width, r.height), (-10.0, 20.0, 300.0, 200.0));
            assert!(super::parse_geometry("garbage").is_err());
        }
    }
}
