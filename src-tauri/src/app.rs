//! Annotate Console process: shared state, windows and Console placement.
use crate::bridge::Record;
use crate::geometry::{self, Side};
use crate::harness::Destination;
use crate::model::{Draft, Point, Rect};
use crate::platform;
use crate::settings::{self, Settings};
use image::RgbaImage;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, PhysicalPosition, PhysicalSize,
    WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

pub const TOOLBAR: &str = "toolbar";
pub const POPUP: &str = "popup";
pub const NOTE: &str = "note";
pub const MARK: &str = "mark";
pub const OVERLAY: &str = "overlay";

#[derive(Clone, Debug)]
pub struct Marked {
    pub kind: String,
    pub points: Vec<Point>,
    pub region: Rect,
}

/// Preserved desktop frame (Hyprland/sway "Preserved frame" mode).
pub struct Backdrop {
    pub image: RgbaImage,
    pub bounds: Rect,
}

pub struct Selection {
    pub selectors: Vec<(String, Rect)>,
    pub kind: String,
    pub backdrop: Option<Arc<Backdrop>>,
    pub painted: HashSet<String>,
    pub tx: Option<tokio::sync::oneshot::Sender<Result<Marked, String>>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Frozen {
    /// Unmarked full-app image as a data URL.
    pub image: String,
    pub size: (u32, u32),
    pub source: String,
    pub needs_app_bounds: bool,
    pub kind: String,
    pub points: Option<Vec<Point>>,
    pub live_mark_points: Option<Vec<Point>>,
    pub replace_id: Option<String>,
    pub comment: String,
}

#[derive(Default)]
pub struct ShortcutState {
    pub current: String,
    pub ready: bool,
    pub global: bool,
    pub error: Option<String>,
    pub recording: bool,
    pub updating: bool,
    /// Compositor binding to remove on exit (wlroots).
    pub compositor: Option<String>,
}

#[derive(Default)]
pub struct ConsolePlace {
    pub bounds: Option<Rect>,
    pub docked: Option<(Side, f64, f64)>,
    pub placing: bool,
    pub generation: u64,
}

pub struct Inner {
    pub settings: Settings,
    pub settings_file: PathBuf,
    pub bounds_file: PathBuf,
    pub drafts: Vec<Draft>,
    pub selected_id: Option<String>,
    pub batch_id: String,
    pub sending: bool,
    pub capturing: bool,
    pub saving: bool,
    pub quitting: bool,
    pub frozen: Option<Frozen>,
    pub capture_kind: String,
    pub notice: String,
    pub detail_id: Option<String>,
    pub pill_mode: String,
    pub statics: Vec<Destination>,
    pub live: Vec<(Record, Destination)>,
    pub destination_id: String,
    pub attempts: HashMap<String, String>,
    pub shortcut: ShortcutState,
    pub selection: Option<Selection>,
    pub cancel: Option<tokio_util::sync::CancellationToken>,
    pub console: ConsolePlace,
    pub default_shortcut: String,
    /// A quit/clear confirmation is open (never stack dialogs).
    pub confirming: bool,
}

impl Inner {
    pub fn busy(&self) -> bool {
        self.quitting
            || self.sending
            || self.capturing
            || self.shortcut.updating
            || self.frozen.is_some()
    }

    pub fn require_idle(&self) -> Result<(), String> {
        if self.busy() {
            Err("Finish the current capture or send first".into())
        } else {
            Ok(())
        }
    }

    pub fn new_batch(&mut self) {
        self.batch_id = uuid::Uuid::new_v4().to_string();
    }

    pub fn destinations(&self) -> Vec<Destination> {
        self.live
            .iter()
            .map(|(_, d)| d.clone())
            .chain(self.statics.iter().cloned())
            .collect()
    }

    pub fn destination(&self) -> Option<Destination> {
        self.destinations()
            .into_iter()
            .find(|d| d.id == self.destination_id)
    }

    pub fn capture_shortcut(&self) -> String {
        if self.settings.capture_shortcut.is_empty() {
            self.default_shortcut.clone()
        } else {
            self.settings.capture_shortcut.clone()
        }
    }

    pub fn save_settings(&mut self, next: Settings) -> Result<(), String> {
        self.settings = settings::save(&self.settings_file, &next)?;
        Ok(())
    }
}

pub struct Shared {
    pub inner: Mutex<Inner>,
}

pub fn lock(app: &AppHandle) -> MutexGuard<'_, Inner> {
    app.state::<Arc<Shared>>()
        .inner()
        .inner
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

pub fn changed(app: &AppHandle) {
    let _ = app.emit("state-changed", ());
}

pub fn status(app: &AppHandle, message: impl Into<String>) {
    let message = message.into();
    lock(app).notice = message.clone();
    let _ = app.emit_to(TOOLBAR, "notice", message);
}

// ---------------------------------------------------------------- coordinates

#[derive(Clone, Copy)]
pub struct Area {
    pub canonical: Rect,
    pub scale: f64,
}

/// Monitors as canonical rectangles (see capture.rs) plus their scale factor.
pub fn monitors(app: &AppHandle, work: bool) -> Vec<Area> {
    if platform::wlroots() {
        return crate::capture::wl::areas(work)
            .unwrap_or_default()
            .into_iter()
            .map(|r| Area {
                canonical: r,
                scale: 1.0,
            })
            .collect();
    }
    // Testing aid: ANNOTATE_DISPLAYS=0,2 limits Annotate to those displays (primary first).
    let only: Option<Vec<usize>> = std::env::var("ANNOTATE_DISPLAYS")
        .ok()
        .map(|v| v.split(',').filter_map(|n| n.trim().parse().ok()).collect());
    let list = app.available_monitors().unwrap_or_default();
    let primary = app.primary_monitor().ok().flatten().map(|m| *m.position());
    let mut areas: Vec<(bool, Area)> = list
        .iter()
        .map(|m| {
            let scale = m.scale_factor();
            let (pos, size) = if work {
                (m.work_area().position, m.work_area().size)
            } else {
                (*m.position(), *m.size())
            };
            let rect = if cfg!(target_os = "macos") {
                Rect::new(
                    pos.x as f64 / scale,
                    pos.y as f64 / scale,
                    size.width as f64 / scale,
                    size.height as f64 / scale,
                )
            } else {
                Rect::new(
                    pos.x as f64,
                    pos.y as f64,
                    size.width as f64,
                    size.height as f64,
                )
            };
            (
                Some(*m.position()) == primary,
                Area {
                    canonical: rect,
                    scale,
                },
            )
        })
        .collect();
    areas.sort_by_key(|(primary, _)| !primary);
    areas
        .into_iter()
        .enumerate()
        .filter(|(i, _)| only.as_ref().is_none_or(|o| o.contains(i)))
        .map(|(_, (_, a))| a)
        .collect()
}

/// Canonical → logical (identity except on Windows/X11 with scaling).
pub fn to_logical(rect: &Rect, areas: &[Area]) -> Rect {
    if cfg!(target_os = "macos") || platform::wlroots() {
        return *rect;
    }
    let c = [rect.x + rect.width / 2.0, rect.y + rect.height / 2.0];
    let scale = areas
        .iter()
        .find(|a| a.canonical.contains_point(c))
        .or(areas.first())
        .map(|a| a.scale)
        .unwrap_or(1.0);
    Rect::new(
        rect.x / scale,
        rect.y / scale,
        rect.width / scale,
        rect.height / scale,
    )
}

/// Logical work areas for Console/popup/note placement.
pub fn logical_areas(app: &AppHandle) -> Vec<Rect> {
    let areas = monitors(app, true);
    areas
        .iter()
        .map(|a| to_logical(&a.canonical, &areas))
        .collect()
}

// ---------------------------------------------------------------- windows

pub struct WindowSpec<'a> {
    pub label: &'a str,
    pub page: &'a str,
    pub title: &'a str,
    pub bounds: Rect,
    /// Bounds are canonical (overlay surfaces) rather than logical.
    pub canonical: bool,
    pub transparent: bool,
    pub focus: bool,
    pub resizable: bool,
    pub passive: bool,
    pub visible: bool,
    pub taskbar: bool,
}

pub fn open_window(app: &AppHandle, spec: WindowSpec) -> Result<WebviewWindow, String> {
    if let Some(existing) = app.get_webview_window(spec.label) {
        let _ = existing.destroy();
    }
    let url = WebviewUrl::App(format!("index.html?page={}", spec.page).into());
    let b = spec.bounds;
    let mut builder = WebviewWindowBuilder::new(app, spec.label, url)
        .title(spec.title)
        .decorations(false)
        .always_on_top(true)
        .resizable(spec.resizable)
        .maximizable(false)
        .minimizable(false)
        .shadow(false)
        .skip_taskbar(!spec.taskbar)
        .visible(false)
        .focused(spec.focus)
        .visible_on_all_workspaces(true)
        .transparent(spec.transparent || spec.passive);
    #[cfg(target_os = "macos")]
    {
        builder = builder.accept_first_mouse(true);
    }
    if spec.canonical && !cfg!(target_os = "macos") && !platform::wlroots() {
        builder = builder.inner_size(1.0, 1.0);
    } else {
        builder = builder.inner_size(b.width, b.height).position(b.x, b.y);
    }
    let window = builder
        .build()
        .map_err(|e| format!("Cannot open window: {e}"))?;
    if spec.canonical && !cfg!(target_os = "macos") && !platform::wlroots() {
        let _ = window.set_position(PhysicalPosition::new(
            b.x.round() as i32,
            b.y.round() as i32,
        ));
        let _ = window.set_size(PhysicalSize::new(
            b.width.round() as u32,
            b.height.round() as u32,
        ));
    }
    if spec.passive {
        let _ = window.set_ignore_cursor_events(true);
    }
    platform::float_over_fullscreen(&window);
    if spec.visible {
        show_window(app, &window, &b, spec.focus);
    }
    Ok(window)
}

pub fn show_window(app: &AppHandle, window: &WebviewWindow, bounds: &Rect, focus: bool) {
    let _ = window.show();
    if focus {
        let _ = window.set_focus();
    }
    if platform::wlroots() {
        let title = window.title().unwrap_or_default();
        let rect = *bounds;
        let app = app.clone();
        std::thread::spawn(move || {
            if let Err(e) = crate::capture::wl::place(&title, std::process::id(), &rect) {
                status(&app, format!("Window positioning: {e}"));
            }
        });
    }
}

pub fn set_logical_bounds(window: &WebviewWindow, b: &Rect) {
    if platform::wlroots() {
        let title = window.title().unwrap_or_default();
        let rect = *b;
        std::thread::spawn(move || {
            let _ = crate::capture::wl::place(&title, std::process::id(), &rect);
        });
        return;
    }
    let _ = window.set_size(LogicalSize::new(b.width, b.height));
    let _ = window.set_position(LogicalPosition::new(b.x, b.y));
}

pub fn logical_bounds(window: &WebviewWindow) -> Option<Rect> {
    if platform::wlroots() {
        return crate::capture::wl::bounds(&window.title().ok()?, std::process::id());
    }
    let scale = window.scale_factor().ok()?;
    let pos = window.outer_position().ok()?.to_logical::<f64>(scale);
    let size = window.inner_size().ok()?.to_logical::<f64>(scale);
    Some(Rect::new(pos.x, pos.y, size.width, size.height))
}

pub fn destroy(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.destroy();
    }
}

// ---------------------------------------------------------------- Console placement

pub fn console_min(inner: &Inner) -> (f64, f64) {
    if inner.settings.console_layout == "pill" {
        geometry::pill_size(&inner.pill_mode)
    } else {
        geometry::TIMELINE_MIN
    }
}

pub fn save_bounds(inner: &Inner, b: &Rect) {
    let value = serde_json::json!({ "x": b.x, "y": b.y, "width": b.width, "height": b.height, "layout": inner.settings.console_layout });
    let _ = settings::write_atomic(&inner.bounds_file, value.to_string().as_bytes());
}

pub fn load_bounds(file: &PathBuf) -> (Option<Rect>, Option<String>) {
    let Ok(text) = std::fs::read_to_string(file) else {
        return (None, None);
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return (None, None);
    };
    let rect = (|| {
        Some(Rect::new(
            v["x"].as_f64()?,
            v["y"].as_f64()?,
            v["width"].as_f64()?,
            v["height"].as_f64()?,
        ))
    })();
    (rect, v["layout"].as_str().map(String::from))
}

fn apply_limits(window: &WebviewWindow, inner: &Inner, current: &Rect) {
    if inner.settings.console_layout == "pill" {
        let _ = window.set_resizable(false);
        let _ = window.set_min_size(Some(LogicalSize::new(current.width, current.height)));
        let _ = window.set_max_size(Some(LogicalSize::new(current.width, current.height)));
    } else {
        let (w, h) = geometry::TIMELINE_MIN;
        let _ = window.set_max_size(None::<LogicalSize<f64>>);
        let _ = window.set_min_size(Some(LogicalSize::new(
            w.min(current.width),
            h.min(current.height),
        )));
        let _ = window.set_resizable(true);
    }
}

/// Reads the Console's actual bounds and persists them.
pub fn remember_console(app: &AppHandle) -> Option<Rect> {
    let window = app.get_webview_window(TOOLBAR)?;
    if !window.is_visible().unwrap_or(false) {
        return lock(app).console.bounds;
    }
    let actual = logical_bounds(&window).filter(Rect::valid);
    let mut inner = lock(app);
    if inner.console.placing {
        return inner.console.bounds;
    }
    if let Some(b) = actual {
        inner.console.bounds = Some(b);
        save_bounds(&inner, &b);
    }
    inner.console.bounds
}

pub fn show_console(app: &AppHandle) {
    let Some(window) = app.get_webview_window(TOOLBAR) else {
        return;
    };
    if lock(app).quitting {
        return;
    }
    if window.is_visible().unwrap_or(false) {
        let _ = window.set_focus();
        return;
    }
    let areas = logical_areas(app);
    let mut inner = lock(app);
    let min = console_min(&inner);
    let target = geometry::console_bounds(inner.console.bounds, &areas, min)
        .unwrap_or(Rect::new(100.0, 100.0, 320.0, 600.0));
    inner.console.bounds = Some(target);
    inner.console.placing = true;
    inner.console.generation += 1;
    apply_limits(&window, &inner, &target);
    drop(inner);
    set_logical_bounds(&window, &target);
    show_window(app, &window, &target, true);
    lock(app).console.placing = false;
}

pub fn hide_console(app: &AppHandle) {
    remember_console(app);
    let mut inner = lock(app);
    inner.console.generation += 1;
    drop(inner);
    if let Some(w) = app.get_webview_window(TOOLBAR) {
        let _ = w.hide();
    }
}

/// Programmatic resize/move (design switch, pill tray/settings).
pub fn relayout_console(app: &AppHandle) {
    let current = remember_console(app);
    let areas = logical_areas(app);
    let mut inner = lock(app);
    let initial = current.is_none();
    let Ok(target) = geometry::layout_bounds(
        current,
        &areas,
        &inner.settings.console_layout,
        &inner.pill_mode,
        initial,
    ) else {
        return;
    };
    inner.console.bounds = Some(target);
    inner.console.placing = true;
    inner.console.generation += 1;
    let Some(window) = app.get_webview_window(TOOLBAR) else {
        inner.console.placing = false;
        return;
    };
    let _ = window.set_resizable(false);
    let _ = window.set_max_size(None::<LogicalSize<f64>>);
    let _ = window.set_min_size(Some(LogicalSize::new(1.0, 1.0)));
    set_logical_bounds(&window, &target);
    apply_limits(&window, &inner, &target);
    save_bounds(&inner, &target);
    inner.console.placing = false;
}

/// After a drag settles: dock to a nearby left/right edge, or restore the
/// pre-dock height when a docked Console is dragged away (timeline only).
pub fn schedule_settle(app: &AppHandle) {
    let generation = {
        let mut inner = lock(app);
        if inner.console.placing {
            return;
        }
        inner.console.generation += 1;
        inner.console.generation
    };
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(350));
        if lock(&app).console.generation != generation {
            return;
        }
        let Some(window) = app.get_webview_window(TOOLBAR) else {
            return;
        };
        let Some(actual) = logical_bounds(&window) else {
            return;
        };
        let snap = lock(&app).settings.console_layout == "timeline" && !platform::wlroots();
        if snap {
            let areas = logical_areas(&app);
            let mut target = None;
            let mut inner = lock(&app);
            if let Some((side, docked)) =
                geometry::dock_bounds(&actual, &areas, Some(geometry::TIMELINE_HEIGHT_RATIO))
            {
                let restore = inner.console.docked.map(|d| d.1).unwrap_or(actual.height);
                inner.console.docked = Some((side, restore, docked.height));
                target = Some(docked);
            } else if let Some((_, restore, docked_height)) = inner.console.docked.take() {
                if (actual.height - docked_height).abs() <= 1.0 {
                    target = geometry::undock_bounds(&actual, &areas, restore);
                }
            }
            drop(inner);
            if let Some(t) = target.filter(|t| !geometry::same(t, &actual)) {
                lock(&app).console.placing = true;
                set_logical_bounds(&window, &t);
                std::thread::sleep(std::time::Duration::from_millis(50));
                lock(&app).console.placing = false;
            }
        }
        remember_console(&app);
    });
}
