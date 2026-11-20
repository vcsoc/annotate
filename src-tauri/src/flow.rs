//! Capture workflow: hide the Console → (countdown) → mark on the live desktop
//! → capture the whole marked application → note beside the mark → ✓ saves a draft.
use crate::app::{
    self, lock, Area, Backdrop, Frozen, Marked, Selection, WindowSpec, MARK, NOTE, OVERLAY,
};
use crate::batch;
use crate::capture;
use crate::geometry::{self, Adjusted};
use crate::model::{AppWindow, Point, Rect};
use crate::platform::{self, Desktop};
use image::RgbaImage;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;

#[derive(Default, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureOptions {
    pub replace_id: Option<String>,
    pub copy_id: Option<String>,
    pub kind: Option<String>,
    #[serde(skip)]
    pub delay_ms: u64,
}

pub fn finish_capture(app: &AppHandle, message: Option<String>) {
    {
        let mut inner = lock(app);
        inner.frozen = None;
        inner.saving = false;
    }
    for label in [OVERLAY, NOTE, MARK] {
        app::destroy(app, label);
    }
    app::changed(app);
    app::show_console(app);
    if let Some(message) = message {
        app::status(app, message);
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

async fn sleep(ms: u64, cancel: &CancellationToken) -> Result<(), String> {
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(ms)) => Ok(()),
        _ = cancel.cancelled() => Err("Capture cancelled".into()),
    }
}

fn overflow_label(adjusted: &Adjusted, app_label: &str) -> String {
    match adjusted {
        Adjusted::Extended => format!("{app_label} · whole application + marked surroundings"),
        Adjusted::Clipped => format!("{app_label} · whole application (mark trimmed to the app)"),
        Adjusted::No => format!("{app_label} · whole application"),
    }
}

fn mark_points(
    adjusted: &Adjusted,
    kind: &str,
    relative: Vec<Point>,
) -> Result<Vec<Point>, String> {
    geometry::validate_mark(
        kind,
        &if *adjusted == Adjusted::Clipped {
            geometry::clamp_points(&relative)
        } else {
            relative
        },
    )
}

pub fn spawn_capture(app: &AppHandle, options: CaptureOptions) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { capture(app, options).await });
}

pub async fn capture(app: AppHandle, options: CaptureOptions) {
    let cancel = CancellationToken::new();
    {
        let mut inner = lock(&app);
        if inner.busy() {
            return;
        }
        inner.capturing = true;
        inner.cancel = Some(cancel.clone());
    }
    app::changed(&app);
    let result = run_capture(&app, &options, &cancel).await;
    if let Err(message) = result {
        finish_capture(&app, Some(message));
    }
    {
        let mut inner = lock(&app);
        inner.capturing = false;
        inner.cancel = None;
    }
    app::changed(&app);
}

struct Evidence {
    kind: String,
    image: RgbaImage,
    source: String,
    points: Vec<Point>,
    needs_app_bounds: bool,
}

async fn run_capture(
    app: &AppHandle,
    options: &CaptureOptions,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let (kind, previous_comment, overflow, mode, max) = {
        let mut inner = lock(app);
        let kind = options
            .kind
            .clone()
            .unwrap_or_else(|| inner.capture_kind.clone());
        if !["rectangle", "freehand"].contains(&kind.as_str())
            || (options.replace_id.is_some() && options.copy_id.is_some())
        {
            return Err("Invalid capture request".into());
        }
        inner.capture_kind = kind.clone();
        let reference = options.replace_id.as_ref().or(options.copy_id.as_ref());
        let previous = match reference {
            Some(id) => Some(
                inner
                    .drafts
                    .iter()
                    .find(|d| &d.id == id)
                    .ok_or("Annotation no longer exists")?
                    .comment
                    .clone(),
            ),
            None => None,
        };
        if options.replace_id.is_none() && inner.drafts.len() >= inner.settings.max_annotations {
            return Err(format!(
                "Send or remove some annotations first ({} per batch).",
                inner.settings.max_annotations
            ));
        }
        (
            kind,
            previous.unwrap_or_default(),
            inner.settings.mark_overflow.clone(),
            inner.settings.capture_mode.clone(),
            inner.settings.max_annotations,
        )
    };
    let _ = max;
    close_detail(app);
    app::hide_console(app);
    if options.delay_ms > 0 {
        sleep(options.delay_ms.min(30_000), cancel).await?;
    }
    if !cfg!(target_os = "macos") {
        sleep(350, cancel).await?;
    }
    if !platform::screen_capture_allowed(true) {
        return Err("Allow Annotate in System Settings → Privacy & Security → Screen Recording (Screen & System Audio Recording), then restart Annotate.".into());
    }
    let own = std::process::id();
    let desktop = platform::desktop();
    let mut region = Rect::default();
    let mut live_points = None;
    let evidence = match desktop {
        Desktop::Hyprland | Desktop::Sway => {
            let boxes = blocking(move || capture::app_windows(own)).await?;
            if boxes.is_empty() {
                return Err("No application windows are available. Bring the app onscreen.".into());
            }
            if mode == "preserved" {
                // Preserve menus before selector focus; evidence and backdrop share one frame.
                let areas = blocking(|| capture::wl::areas(false)).await?;
                let bounds = geometry::desktop_bounds(&areas)?;
                let image = blocking(move || capture::wl::grim(&bounds)).await?;
                if image.width() as f64 != bounds.width || image.height() as f64 != bounds.height {
                    return Err("Desktop geometry changed during capture. Capture again.".into());
                }
                let backdrop = Arc::new(Backdrop { image, bounds });
                let marked = select_on_desktop(app, &kind, Some(backdrop.clone()), cancel).await?;
                region = marked.region;
                let choice = geometry::target_for_mark(
                    &region,
                    &boxes,
                    &overflow,
                    marked.points.first().copied(),
                )?;
                let points = mark_points(
                    &choice.adjusted,
                    &marked.kind,
                    geometry::points_in_app(&marked.points, &choice.target),
                )?;
                live_points = Some(geometry::points_in_app(&marked.points, &region));
                let t = choice.target.round();
                let (x, y) = ((t.x - bounds.x) as i64, (t.y - bounds.y) as i64);
                if x < 0
                    || y < 0
                    || x as f64 + t.width > bounds.width
                    || y as f64 + t.height > bounds.height
                {
                    return Err("The entire application must be onscreen to preserve it. Move it onscreen and capture again.".into());
                }
                let image = image::imageops::crop_imm(
                    &backdrop.image,
                    x as u32,
                    y as u32,
                    t.width as u32,
                    t.height as u32,
                )
                .to_image();
                Evidence {
                    kind: marked.kind,
                    image,
                    source: format!(
                        "{} (preserved before marking)",
                        overflow_label(&choice.adjusted, &choice.app.label)
                    ),
                    points,
                    needs_app_bounds: false,
                }
            } else {
                let mut marked = None;
                if kind == "rectangle" {
                    let token = cancel.clone();
                    let list = boxes.clone();
                    region = blocking(move || capture::wl::slurp(&list, &token)).await?;
                } else {
                    let m = select_on_desktop(app, &kind, None, cancel).await?;
                    region = m.region;
                    live_points = Some(geometry::points_in_app(&m.points, &region));
                    marked = Some(m);
                }
                let fresh = blocking(move || capture::app_windows(own)).await?;
                let start = marked
                    .as_ref()
                    .and_then(|m| m.points.first().copied())
                    .unwrap_or([region.x, region.y]);
                let choice = geometry::target_for_mark(&region, &fresh, &overflow, Some(start))?;
                let used_kind = marked
                    .as_ref()
                    .map(|m| m.kind.clone())
                    .unwrap_or(kind.clone());
                let relative = match &marked {
                    Some(m) => geometry::points_in_app(&m.points, &choice.target),
                    None => geometry::relative_mark(&region, &choice.target),
                };
                let points = mark_points(&choice.adjusted, &used_kind, relative)?;
                sleep(150, cancel).await?;
                let target = choice.target;
                let image = blocking(move || capture::wl::grim(&target)).await?;
                Evidence {
                    kind: used_kind,
                    image,
                    source: overflow_label(&choice.adjusted, &choice.app.label),
                    points,
                    needs_app_bounds: false,
                }
            }
        }
        Desktop::Portal => {
            // No global window geometry on this compositor: capture the display,
            // then outline the whole app and mark inside the in-image editor.
            let image = blocking(|| capture::capture_display(None)).await?;
            Evidence {
                kind: kind.clone(),
                image,
                source: "Display capture · outline the whole application".into(),
                points: vec![],
                needs_app_bounds: true,
            }
        }
        Desktop::Native => {
            let candidates = blocking(move || capture::app_windows(own)).await?;
            if candidates.is_empty() {
                return Err("No application windows are available. Check capture permission and bring the app onscreen.".into());
            }
            let marked = select_on_desktop(app, &kind, None, cancel).await?;
            region = marked.region;
            let choice = geometry::target_for_mark(
                &region,
                &candidates,
                &overflow,
                marked.points.first().copied(),
            )?;
            let points = mark_points(
                &choice.adjusted,
                &marked.kind,
                geometry::points_in_app(&marked.points, &choice.target),
            )?;
            live_points = Some(geometry::points_in_app(&marked.points, &region));
            sleep(250, cancel).await?; // Remove every selector before capturing the application.
            let target_app: AppWindow = choice.app.clone();
            let current = blocking(move || capture::app_windows(own)).await?;
            let same = current.iter().find(|w| w.native_id == target_app.native_id);
            if same.is_none_or(|s| {
                (s.x - target_app.x).abs() > 1.0
                    || (s.y - target_app.y).abs() > 1.0
                    || (s.width - target_app.width).abs() > 1.0
                    || (s.height - target_app.height).abs() > 1.0
            }) {
                return Err(
                    "The marked application moved or closed during selection. Capture again."
                        .into(),
                );
            }
            let (adjusted, target) = (choice.adjusted.clone(), choice.target);
            let image = blocking(move || {
                if adjusted == Adjusted::Extended {
                    capture::capture_rect(&target)
                } else {
                    capture::capture_window(&target_app)
                }
            })
            .await?;
            Evidence {
                kind: marked.kind,
                image,
                source: overflow_label(&choice.adjusted, &choice.app.label),
                points,
                needs_app_bounds: false,
            }
        }
    };
    if cancel.is_cancelled() {
        return Err("Capture cancelled".into());
    }
    let (fitted, png) = blocking(move || batch::fit(evidence.image)).await?;
    let frozen = Frozen {
        image: batch::data_url(&png),
        size: fitted.dimensions(),
        source: evidence.source.chars().take(200).collect(),
        needs_app_bounds: evidence.needs_app_bounds,
        kind: evidence.kind,
        points: (!evidence.points.is_empty()).then_some(evidence.points),
        live_mark_points: live_points,
        replace_id: options.replace_id.clone(),
        comment: previous_comment,
    };
    let needs_editor = frozen.needs_app_bounds;
    {
        let mut inner = lock(app);
        if inner.quitting {
            return Ok(());
        }
        inner.frozen = Some(frozen);
    }
    if needs_editor {
        open_editor(app)
    } else {
        open_live_note(app, &region)
    }
}

/// In-image editor (displays without window geometry).
fn open_editor(app: &AppHandle) -> Result<(), String> {
    let areas = app::logical_areas(app);
    let area = areas
        .first()
        .copied()
        .unwrap_or(Rect::new(0.0, 0.0, 1280.0, 900.0));
    let (w, h) = (1200f64.min(area.width), 850f64.min(area.height));
    let bounds = Rect::new(
        area.x + (area.width - w) / 2.0,
        area.y + (area.height - h) / 2.0,
        w,
        h,
    );
    app::open_window(
        app,
        WindowSpec {
            label: OVERLAY,
            page: "overlay",
            title: "Annotate Capture",
            bounds,
            canonical: false,
            transparent: false,
            focus: true,
            resizable: true,
            passive: false,
            visible: true,
            taskbar: true,
        },
    )?;
    Ok(())
}

/// The live highlight over the app plus the note popup beside it.
fn open_live_note(app: &AppHandle, region: &Rect) -> Result<(), String> {
    let all = app::monitors(app, true);
    let logical: Vec<Rect> = all
        .iter()
        .map(|a| app::to_logical(&a.canonical, &all))
        .collect();
    let mark_logical = app::to_logical(region, &all);
    let note = geometry::place_note(&mark_logical, &logical, geometry::NOTE_SIZE)?;
    app::open_window(
        app,
        WindowSpec {
            label: MARK,
            page: "mark",
            title: "Annotate Highlight",
            bounds: *region,
            canonical: true,
            transparent: true,
            focus: false,
            resizable: false,
            passive: true,
            visible: true,
            taskbar: false,
        },
    )?;
    app::open_window(
        app,
        WindowSpec {
            label: NOTE,
            page: "note",
            title: "Annotate Note",
            bounds: note,
            canonical: false,
            transparent: false,
            focus: true,
            resizable: false,
            passive: false,
            visible: true,
            taskbar: false,
        },
    )?;
    Ok(())
}

pub fn close_detail(app: &AppHandle) {
    lock(app).detail_id = None;
    app::destroy(app, app::POPUP);
}

/// Covers every display with a transparent selection surface and waits for a mark.
async fn select_on_desktop(
    app: &AppHandle,
    kind: &str,
    backdrop: Option<Arc<Backdrop>>,
    cancel: &CancellationToken,
) -> Result<Marked, String> {
    let areas: Vec<Area> = match &backdrop {
        Some(_) => capture::wl::areas(false)?
            .into_iter()
            .map(|r| Area {
                canonical: r,
                scale: 1.0,
            })
            .collect(),
        None => app::monitors(app, false),
    };
    if areas.is_empty() {
        return Err("No displays are available".into());
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    let selectors: Vec<(String, Rect)> = areas
        .iter()
        .enumerate()
        .map(|(i, a)| (format!("desktop-{i}"), a.canonical))
        .collect();
    {
        let mut inner = lock(app);
        inner.selection = Some(Selection {
            selectors: selectors.clone(),
            kind: kind.to_string(),
            backdrop: backdrop.clone(),
            painted: HashSet::new(),
            tx: Some(tx),
        });
    }
    let escape = crate::shortcut::grab_escape(app);
    let cursor = app.cursor_position().ok();
    let mut opened = Ok(());
    for (i, (label, bounds)) in selectors.iter().enumerate() {
        let focus = cursor.map(|c| {
            let p = if cfg!(target_os = "macos") {
                [c.x / areas[i].scale, c.y / areas[i].scale]
            } else {
                [c.x, c.y]
            };
            bounds.contains_point(p)
        });
        let spec = WindowSpec {
            label,
            page: "desktop",
            title: &format!("Annotate Capture — display {}", i + 1),
            bounds: *bounds,
            canonical: true,
            transparent: true,
            focus: focus.unwrap_or(i == 0),
            resizable: false,
            passive: false,
            // With a preserved backdrop, show only after every display painted it.
            visible: backdrop.is_none(),
            taskbar: false,
        };
        if let Err(e) = app::open_window(app, spec) {
            opened = Err(e);
            break;
        }
    }
    let result = match opened {
        Err(e) => Err(e),
        Ok(()) => tokio::select! {
            r = rx => r.unwrap_or_else(|_| Err("Capture cancelled".into())),
            _ = cancel.cancelled() => Err("Capture cancelled".into()),
            _ = tokio::time::sleep(Duration::from_secs(120)) => Err("Capture selection timed out".into()),
        },
    };
    lock(app).selection = None;
    crate::shortcut::release_escape(app, escape);
    for (label, _) in &selectors {
        app::destroy(app, label);
    }
    result
}

pub fn cancel_selection(app: &AppHandle) {
    let tx = lock(app).selection.as_mut().and_then(|s| s.tx.take());
    if let Some(tx) = tx {
        let _ = tx.send(Err("Capture cancelled".into()));
    }
}

pub fn desktop_progress(app: &AppHandle, marked: &Marked) {
    let _ = app.emit(
        "desktop-progress",
        serde_json::json!({ "kind": marked.kind, "points": marked.points }),
    );
}

pub fn selector_ready(app: &AppHandle, label: &str) {
    let all_painted = {
        let mut inner = lock(app);
        let Some(selection) = inner.selection.as_mut() else {
            return;
        };
        if selection.backdrop.is_none() {
            return;
        }
        selection.painted.insert(label.to_string());
        (selection.painted.len() == selection.selectors.len()).then(|| selection.selectors.clone())
    };
    if let Some(selectors) = all_painted {
        for (l, bounds) in &selectors {
            if let Some(w) = app.get_webview_window(l) {
                app::show_window(app, &w, bounds, l == label);
            }
        }
    }
}

/// Preserved-frame background for one selector, as a PNG data URL.
pub fn backdrop_for(backdrop: &Backdrop, bounds: &Rect) -> Option<String> {
    let (x, y) = (
        (bounds.x - backdrop.bounds.x).round(),
        (bounds.y - backdrop.bounds.y).round(),
    );
    if x < 0.0 || y < 0.0 {
        return None;
    }
    let crop = image::imageops::crop_imm(
        &backdrop.image,
        x as u32,
        y as u32,
        bounds.width as u32,
        bounds.height as u32,
    )
    .to_image();
    Some(batch::data_url(&batch::encode_png(&crop)))
}

/// Cancels display-dependent work when monitors change during a capture.
pub fn displays_changed(app: &AppHandle) {
    let token = lock(app).cancel.clone();
    if let Some(token) = token {
        if lock(app).selection.is_some() {
            token.cancel();
            cancel_selection(app);
        }
    }
}

pub fn quit_now(app: &AppHandle) {
    {
        let mut inner = lock(app);
        inner.quitting = true;
        if let Some(c) = &inner.cancel {
            c.cancel();
        }
    }
    cancel_selection(app);
    app::remember_console(app);
    crate::shortcut::release_all(app);
    app.exit(0);
}

pub fn emit_kind(app: &AppHandle, kind: &str) {
    let _ = app.emit("desktop-kind", kind);
}
