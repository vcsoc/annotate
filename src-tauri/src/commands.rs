//! IPC commands used by the UI (`window.annotate` in ui/tauri-api.js). Each
//! command checks which window called it, mirroring the original sender checks.
use crate::app::{self, lock, WindowSpec, MARK, NOTE, OVERLAY, POPUP, TOOLBAR};
use crate::batch;
use crate::flow::{self, CaptureOptions};
use crate::geometry;
use crate::model::{Draft, Mark, Point};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, WebviewWindow};

type R<T> = Result<T, String>;

fn only(window: &WebviewWindow, labels: &[&str], message: &str) -> R<()> {
    if labels.contains(&window.label()) {
        Ok(())
    } else {
        Err(message.into())
    }
}

fn is_selector(window: &WebviewWindow) -> bool {
    window.label().starts_with("desktop-")
}

#[tauri::command]
pub fn state(app: AppHandle) -> Value {
    let inner = lock(&app);
    let destination = inner.destination();
    let drafts: Vec<Value> = inner
        .drafts
        .iter()
        .map(|d| {
            let mut v = serde_json::to_value(d).unwrap();
            let obj = v.as_object_mut().unwrap();
            obj.remove("image");
            obj.insert("preview".into(), json!(d.preview));
            obj.insert("ready".into(), json!(d.ready()));
            v
        })
        .collect();
    json!({
        "settings": inner.settings,
        "preservedCaptureAvailable": crate::platform::wlroots(),
        "notice": inner.notice,
        "project": destination.as_ref().map(|d| if d.project.is_empty() { d.label.clone() } else { d.project.clone() }).unwrap_or_default(),
        "destinationId": inner.destination_id,
        "destination": destination,
        "destinations": inner.destinations(),
        "destinationAvailable": destination.is_some(),
        "folder": inner.settings.project_dir,
        "shortcut": inner.capture_shortcut(),
        "defaultShortcut": inner.default_shortcut,
        "shortcutReady": inner.shortcut.ready && inner.shortcut.error.is_none(),
        "shortcutGlobal": inner.shortcut.global,
        "updatingShortcut": inner.shortcut.updating,
        "selectedId": inner.selected_id,
        "detailId": inner.detail_id,
        "pillMode": inner.pill_mode,
        "sending": inner.sending,
        "capturing": inner.capturing || inner.frozen.is_some(),
        "captureKind": inner.capture_kind,
        "platform": std::env::consts::OS,
        "drafts": drafts,
    })
}

#[tauri::command]
pub fn settings(app: AppHandle, window: WebviewWindow, value: Value) -> R<Value> {
    only(&window, &[TOOLBAR], "Only the Console can change settings")?;
    let (next, shortcut_changed, layout_changed, count) = {
        let inner = lock(&app);
        inner.require_idle()?;
        let next = crate::settings::merge(&inner.settings, &value)?;
        (
            next.clone(),
            next.capture_shortcut != inner.settings.capture_shortcut
                || next.global_shortcut != inner.settings.global_shortcut,
            next.console_layout != inner.settings.console_layout,
            inner.drafts.len(),
        )
    };
    if next.max_annotations < count {
        return Err(format!(
            "There are {count} unsent annotations. Send or remove some before lowering the limit."
        ));
    }
    if shortcut_changed {
        lock(&app).shortcut.updating = true;
        app::changed(&app);
        let proposed = if next.capture_shortcut.is_empty() {
            lock(&app).default_shortcut.clone()
        } else {
            next.capture_shortcut.clone()
        };
        let result = crate::shortcut::apply(&app, &proposed, next.global_shortcut);
        lock(&app).shortcut.updating = false;
        if let Err(e) = result {
            app::changed(&app);
            return Err(e);
        }
        lock(&app).shortcut.error = None;
        let recording = lock(&app).shortcut.recording;
        if recording {
            crate::shortcut::set_recording(&app, true);
        }
    }
    {
        let mut inner = lock(&app);
        if next.mark_tool != inner.settings.mark_tool {
            inner.capture_kind = next.mark_tool.clone();
        }
        inner.save_settings(next)?;
        if layout_changed {
            inner.pill_mode = "collapsed".into();
        }
    }
    if layout_changed {
        flow::close_detail(&app);
        app::relayout_console(&app);
    }
    app::status(&app, "Settings saved.");
    app::changed(&app);
    Ok(serde_json::to_value(&lock(&app).settings).unwrap())
}

#[tauri::command]
pub fn shortcut_recording(app: AppHandle, window: WebviewWindow, active: bool) -> R<()> {
    only(&window, &[TOOLBAR], "Only the Console can record shortcuts")?;
    crate::shortcut::set_recording(&app, active);
    Ok(())
}

#[tauri::command]
pub fn shortcut_capture(app: AppHandle, window: WebviewWindow) -> R<bool> {
    only(
        &window,
        &[TOOLBAR],
        "Only the Console can use the Console shortcut",
    )?;
    Ok(crate::shortcut::console_trigger(&app))
}

#[tauri::command]
pub fn console_size(app: AppHandle, window: WebviewWindow, mode: String) -> R<()> {
    only(&window, &[TOOLBAR], "Only the Console can resize itself")?;
    {
        let mut inner = lock(&app);
        if inner.settings.console_layout != "pill"
            || !["collapsed", "expanded", "settings"].contains(&mode.as_str())
        {
            return Err("Invalid Console size".into());
        }
        if inner.pill_mode == mode {
            return Ok(());
        }
        inner.pill_mode = mode;
    }
    app::relayout_console(&app);
    Ok(())
}

#[tauri::command]
pub fn detail_open(app: AppHandle, window: WebviewWindow, id: String, top: f64) -> R<()> {
    only(
        &window,
        &[TOOLBAR],
        "Only the Console can open annotation details",
    )?;
    {
        let inner = lock(&app);
        inner.require_idle()?;
        if !inner.drafts.iter().any(|d| d.id == id) {
            return Err("Annotation no longer exists".into());
        }
    }
    let anchor = app::remember_console(&app).ok_or("The Console position is unavailable")?;
    let areas = app::logical_areas(&app);
    let top = if top.is_finite() {
        top.clamp(0.0, anchor.height)
    } else {
        0.0
    };
    let bounds = geometry::beside_bounds(&anchor, &areas, top, geometry::DETAIL_SIZE)
        .ok_or("No display work area is available")?;
    {
        let mut inner = lock(&app);
        inner.detail_id = Some(id.clone());
        inner.selected_id = Some(id);
    }
    if let Some(existing) = app.get_webview_window(POPUP) {
        app::set_logical_bounds(&existing, &bounds);
        let _ = existing.set_focus();
    } else {
        app::open_window(
            &app,
            WindowSpec {
                label: POPUP,
                page: "popup",
                title: "Annotate Detail",
                bounds,
                canonical: false,
                transparent: true,
                focus: true,
                resizable: true,
                passive: false,
                visible: true,
                taskbar: false,
            },
        )?;
    }
    app::changed(&app);
    Ok(())
}

#[tauri::command]
pub fn detail(app: AppHandle, window: WebviewWindow) -> R<Value> {
    only(&window, &[POPUP], "No annotation detail in this window")?;
    Ok(json!({ "id": lock(&app).detail_id }))
}

#[tauri::command]
pub fn detail_edit(app: AppHandle, window: WebviewWindow) -> R<Value> {
    only(&window, &[POPUP], "No annotation detail in this window")?;
    let inner = lock(&app);
    let id = inner
        .detail_id
        .clone()
        .ok_or("Annotation no longer exists")?;
    let item = inner
        .drafts
        .iter()
        .find(|d| d.id == id)
        .ok_or("Annotation no longer exists")?;
    Ok(
        json!({ "id": item.id, "base": item.base.as_ref().map(|b| format!("data:image/png;base64,{b}")), "width": item.width, "height": item.height, "marks": item.marks }),
    )
}

#[derive(serde::Deserialize)]
pub struct DetailValue {
    marks: Vec<Mark>,
    image: Option<String>,
}

#[tauri::command]
pub fn detail_save(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
    value: DetailValue,
) -> R<Value> {
    only(&window, &[POPUP], "No annotation detail in this window")?;
    let mut inner = lock(&app);
    inner.require_idle()?;
    let index = inner
        .drafts
        .iter()
        .position(|d| d.id == id)
        .ok_or("Annotation no longer exists")?;
    let item = inner.drafts[index].clone();
    if item.base.is_none() {
        return Err("This annotation cannot be edited. Retake it to add marks.".into());
    }
    if value.marks.is_empty() || value.marks.len() > batch::MAX_MARKS {
        return Err("Use 1–20 marks per annotation".into());
    }
    let marks: Vec<Mark> = value
        .marks
        .iter()
        .map(|m| {
            if m.comment.len() > 8000 {
                return Err("Invalid comment".to_string());
            }
            Ok(Mark {
                kind: m.kind.clone(),
                points: geometry::validate_mark(&m.kind, &m.points)?,
                comment: m.comment.clone(),
            })
        })
        .collect::<R<_>>()?;
    let mut updated = item.clone();
    if let Some(image) = value.image {
        let (w, h) = batch::png_size(&image)?;
        if (w, h) != (item.width, item.height) {
            return Err("The edited screenshot must keep the complete app".into());
        }
        updated.preview = batch::preview(&image);
        updated.image = image;
    }
    updated.kind = marks[0].kind.clone();
    updated.points = marks[0].points.clone();
    updated.comment = if marks.len() == 1 {
        marks[0].comment.clone()
    } else {
        batch::mark_comments(
            &marks
                .iter()
                .map(|m| Mark {
                    comment: if m.comment.trim().is_empty() {
                        "…".into()
                    } else {
                        m.comment.clone()
                    },
                    ..m.clone()
                })
                .collect::<Vec<_>>(),
        )
    };
    updated.marks = marks;
    let count = updated.marks.len();
    inner.drafts[index] = updated;
    inner.new_batch();
    drop(inner);
    // The popup keeps its own editing state; the Console refreshes.
    let _ = tauri::Emitter::emit_to(&app, TOOLBAR, "state-changed", ());
    Ok(json!({ "count": count }))
}

#[tauri::command]
pub fn capture_kind(app: AppHandle, window: WebviewWindow, kind: String) -> R<()> {
    only(
        &window,
        &[TOOLBAR],
        "Only the Console can choose the mark tool",
    )?;
    if !["rectangle", "freehand"].contains(&kind.as_str()) {
        return Err("Invalid mark tool".into());
    }
    lock(&app).capture_kind = kind;
    Ok(())
}

#[tauri::command]
pub fn desktop_kind(app: AppHandle, window: WebviewWindow, kind: String) -> R<()> {
    if !is_selector(&window) {
        return Err("No desktop selection".into());
    }
    if !["rectangle", "freehand"].contains(&kind.as_str()) {
        return Err("Invalid mark tool".into());
    }
    {
        let mut inner = lock(&app);
        let selection = inner.selection.as_mut().ok_or("No desktop selection")?;
        selection.kind = kind.clone();
        inner.capture_kind = kind.clone();
    }
    flow::emit_kind(&app, &kind);
    app::changed(&app);
    Ok(())
}

#[tauri::command]
pub fn detail_show(app: AppHandle, window: WebviewWindow, id: String) -> R<()> {
    only(&window, &[POPUP], "No annotation detail in this window")?;
    let mut inner = lock(&app);
    if !inner.drafts.iter().any(|d| d.id == id) {
        return Err("Annotation no longer exists".into());
    }
    inner.detail_id = Some(id.clone());
    inner.selected_id = Some(id);
    drop(inner);
    app::changed(&app);
    Ok(())
}

#[tauri::command]
pub fn detail_close(app: AppHandle, window: WebviewWindow) -> R<()> {
    only(&window, &[TOOLBAR, POPUP], "No annotation detail to close")?;
    flow::close_detail(&app);
    app::changed(&app);
    Ok(())
}

#[tauri::command]
pub fn developer_page(window: WebviewWindow) -> R<()> {
    only(&window, &[TOOLBAR], "Only the Console can open this link")?;
    crate::deliver::open_path("https://github.com/vcsoc")
}

#[tauri::command]
pub fn target(app: AppHandle, window: WebviewWindow, id: String) -> R<()> {
    only(
        &window,
        &[TOOLBAR],
        "Only the Console can change destination",
    )?;
    let mut inner = lock(&app);
    inner.require_idle()?;
    if !inner.destinations().iter().any(|d| d.id == id) {
        return Err(
            "That destination is no longer available. Open or reload the harness session first."
                .into(),
        );
    }
    inner.destination_id = id.clone();
    let mut next = inner.settings.clone();
    next.destination = id;
    let _ = inner.save_settings(next);
    drop(inner);
    app::changed(&app);
    Ok(())
}

#[tauri::command]
pub fn desktop_config(app: AppHandle, window: WebviewWindow) -> R<Value> {
    let inner = lock(&app);
    let selection = inner.selection.as_ref().ok_or("No desktop selection")?;
    let bounds = selection
        .selectors
        .iter()
        .find(|(l, _)| l == window.label())
        .map(|(_, b)| *b)
        .ok_or("No desktop selection")?;
    let background = selection
        .backdrop
        .as_ref()
        .and_then(|b| flow::backdrop_for(b, &bounds));
    Ok(json!({ "bounds": bounds, "kind": selection.kind, "background": background }))
}

#[tauri::command]
pub fn desktop_ready(app: AppHandle, window: WebviewWindow) -> R<()> {
    if is_selector(&window) {
        flow::selector_ready(&app, window.label());
    }
    Ok(())
}

#[tauri::command]
pub fn desktop_mark(
    app: AppHandle,
    window: WebviewWindow,
    points: Vec<Point>,
    complete: bool,
) -> R<()> {
    let mut inner = lock(&app);
    let selection = inner.selection.as_mut().ok_or("No desktop selection")?;
    if !selection.selectors.iter().any(|(l, _)| l == window.label()) {
        return Err("No desktop selection".into());
    }
    let displays: Vec<_> = selection.selectors.iter().map(|(_, b)| *b).collect();
    let region = geometry::desktop_selection(&selection.kind, &points, &displays);
    let marked = region.map(|region| app::Marked {
        kind: selection.kind.clone(),
        points: points.clone(),
        region,
    });
    if complete {
        match marked {
            Ok(m) => {
                if let Some(tx) = selection.tx.take() {
                    let _ = tx.send(Ok(m));
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    } else {
        drop(inner);
        if let Ok(m) = marked {
            flow::desktop_progress(&app, &m);
        }
        Ok(())
    }
}

#[tauri::command]
pub fn capture(app: AppHandle, window: WebviewWindow, options: Option<CaptureOptions>) -> R<()> {
    only(&window, &[TOOLBAR, POPUP], "Only the Console can capture")?;
    let mut options = options.unwrap_or_default();
    options.delay_ms = lock(&app).settings.countdown_seconds as u64 * 1000;
    flow::spawn_capture(&app, options);
    Ok(())
}

#[tauri::command]
pub fn select(app: AppHandle, id: String) {
    let mut inner = lock(&app);
    if inner.drafts.iter().any(|d| d.id == id) {
        inner.selected_id = Some(id);
    }
}

#[tauri::command]
pub fn crop_app(app: AppHandle, window: WebviewWindow, points: Vec<Point>) -> R<Value> {
    only(&window, &[OVERLAY], "No application boundary to select")?;
    let mut inner = lock(&app);
    let frozen = inner
        .frozen
        .as_mut()
        .filter(|f| f.needs_app_bounds)
        .ok_or("No application boundary to select")?;
    let image = batch::decode_png(&frozen.image)?;
    let (w, h) = image.dimensions();
    if points.len() != 2 || !batch::valid_points(&points) {
        return Err("Draw a capture area first".into());
    }
    let (l, t) = (
        points[0][0].min(points[1][0]),
        points[0][1].min(points[1][1]),
    );
    let (r, b) = (
        points[0][0].max(points[1][0]),
        points[0][1].max(points[1][1]),
    );
    let x = (l * w as f64).floor() as u32;
    let y = (t * h as f64).floor() as u32;
    let cw = ((r * w as f64).ceil() as u32)
        .min(w)
        .saturating_sub(x)
        .max(1);
    let ch = ((b * h as f64).ceil() as u32)
        .min(h)
        .saturating_sub(y)
        .max(1);
    let cropped = image::imageops::crop_imm(&image, x, y, cw, ch).to_image();
    let (fitted, png) = batch::fit(cropped)?;
    frozen.image = batch::data_url(&png);
    frozen.size = fitted.dimensions();
    frozen.source = "Display capture · manually bounded whole app".into();
    frozen.needs_app_bounds = false;
    frozen.points = None;
    Ok(serde_json::to_value(&*frozen).unwrap())
}

#[tauri::command]
pub fn mark(app: AppHandle, window: WebviewWindow, kind: String, points: Vec<Point>) -> R<Value> {
    only(
        &window,
        &[OVERLAY],
        "Identify the whole application before marking an area",
    )?;
    let mut inner = lock(&app);
    let frozen = inner
        .frozen
        .as_mut()
        .filter(|f| !f.needs_app_bounds)
        .ok_or("Identify the whole application before marking an area")?;
    frozen.points = Some(geometry::validate_mark(&kind, &points)?);
    frozen.kind = kind;
    Ok(serde_json::to_value(&*frozen).unwrap())
}

#[tauri::command]
pub fn frozen(app: AppHandle, window: WebviewWindow) -> R<Value> {
    only(
        &window,
        &[OVERLAY, NOTE, MARK],
        "No annotation in this window",
    )?;
    Ok(serde_json::to_value(&lock(&app).frozen).unwrap())
}

#[tauri::command]
pub fn cancel(app: AppHandle, window: WebviewWindow) -> R<()> {
    if is_selector(&window) {
        flow::cancel_selection(&app);
    } else if [OVERLAY, NOTE].contains(&window.label()) {
        flow::finish_capture(
            &app,
            Some("Capture cancelled. Existing annotations were kept.".into()),
        );
    }
    Ok(())
}

#[derive(serde::Deserialize)]
pub struct NewItem {
    image: String,
    comment: String,
}

#[tauri::command]
pub fn add(app: AppHandle, window: WebviewWindow, item: NewItem) -> R<Value> {
    only(
        &window,
        &[OVERLAY, NOTE],
        "Mark an area within the complete application first",
    )?;
    let message;
    let count;
    {
        let mut inner = lock(&app);
        let pending = inner
            .frozen
            .clone()
            .filter(|f| !f.needs_app_bounds && f.points.is_some())
            .ok_or("Mark an area within the complete application first")?;
        if inner.sending || inner.saving {
            return Err("Save or Send already in progress".into());
        }
        let image = batch::strip_data_url(&item.image).to_string();
        let (w, h) = batch::png_size(&image)?;
        if (w, h) != pending.size {
            return Err("The screenshot must include the complete app, not a cropped mark".into());
        }
        let comment = item.comment.trim().to_string();
        if comment.is_empty() || comment.len() > 8000 {
            return Err("Annotation needs a comment".into());
        }
        let points = pending.points.clone().unwrap();
        let replacing = pending.replace_id.is_some();
        if !replacing && inner.drafts.len() >= inner.settings.max_annotations {
            return Err(format!(
                "Batch limit reached ({} annotations).",
                inner.settings.max_annotations
            ));
        }
        let entry = Draft {
            id: pending
                .replace_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            preview: batch::preview(&image),
            image,
            base: Some(batch::strip_data_url(&pending.image).to_string()),
            comment: comment.clone(),
            kind: pending.kind.clone(),
            points: points.clone(),
            marks: vec![Mark {
                kind: pending.kind.clone(),
                points,
                comment,
            }],
            width: w,
            height: h,
            source: pending.source.clone(),
        };
        let mut next = inner.drafts.clone();
        match &pending.replace_id {
            Some(id) => {
                let i = next
                    .iter()
                    .position(|d| &d.id == id)
                    .ok_or("The annotation being retaken no longer exists")?;
                next[i] = entry;
            }
            None => {
                if next.len() >= batch::MAX_ITEMS {
                    return Err("Send or remove some annotations first (12 per batch).".into());
                }
                next.push(entry);
            }
        }
        let size: usize = next
            .iter()
            .map(|d| d.image.len() + d.comment.len() + 2048)
            .sum();
        if size > batch::MAX_BODY - 1024 {
            return Err(
                "This batch is at its size limit. Send existing annotations before adding more."
                    .into(),
            );
        }
        inner.selected_id = Some(
            pending
                .replace_id
                .clone()
                .unwrap_or_else(|| next.last().unwrap().id.clone()),
        );
        inner.drafts = next;
        inner.new_batch();
        inner.saving = true;
        count = inner.drafts.len();
        message = format!("{} annotation. The full app and highlighted area are together in one image. Nothing sent yet.", if replacing { "Replaced" } else { "Saved" });
    }
    // Let the note window receive its acknowledgement before it is destroyed.
    let app2 = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(60));
        let app3 = app2.clone();
        let _ = app2.run_on_main_thread(move || flow::finish_capture(&app3, Some(message)));
    });
    Ok(json!({ "count": count }))
}

#[tauri::command]
pub fn remove(app: AppHandle, id: String) -> R<()> {
    let close;
    {
        let mut inner = lock(&app);
        inner.require_idle()?;
        let index = inner.drafts.iter().position(|d| d.id == id);
        inner.drafts.retain(|d| d.id != id);
        if inner.selected_id.as_deref() == Some(&id) {
            let len = inner.drafts.len();
            inner.selected_id = index
                .and_then(|i| inner.drafts.get(i.min(len.saturating_sub(1))))
                .map(|d| d.id.clone());
        }
        close = inner.detail_id.as_deref() == Some(&id) && inner.selected_id.is_none();
        if inner.detail_id.as_deref() == Some(&id) && inner.selected_id.is_some() {
            inner.detail_id = inner.selected_id.clone();
        }
        inner.new_batch();
    }
    if close {
        flow::close_detail(&app);
    }
    app::changed(&app);
    Ok(())
}

#[tauri::command]
pub fn comment(app: AppHandle, window: WebviewWindow, id: String, comment: String) -> R<()> {
    {
        let mut inner = lock(&app);
        inner.require_idle()?;
        if comment.len() > 8000 {
            return Err("Invalid comment".into());
        }
        let Some(item) = inner.drafts.iter_mut().find(|d| d.id == id) else {
            return Ok(());
        };
        if item.marks.len() > 1 {
            return Err(
                "This annotation has numbered marks. Edit each comment in the annotation popup."
                    .into(),
            );
        }
        item.comment = comment.clone();
        if let Some(m) = item.marks.first_mut() {
            m.comment = comment;
        }
        inner.selected_id = Some(id);
        inner.new_batch();
    }
    // The typing window keeps its own state; the other window refreshes.
    for label in [TOOLBAR, POPUP] {
        if label != window.label() {
            let _ = tauri::Emitter::emit_to(&app, label, "state-changed", ());
        }
    }
    Ok(())
}

/// Shows one confirmation at a time; a second request while one is open is ignored.
fn confirm(app: &AppHandle, message: &str, ok: &str, cancel: &str) -> bool {
    {
        let mut inner = lock(app);
        if inner.confirming {
            return false;
        }
        inner.confirming = true;
    }
    let answer = confirm_dialog(app, message, ok, cancel);
    lock(app).confirming = false;
    answer
}

fn confirm_dialog(app: &AppHandle, message: &str, ok: &str, cancel: &str) -> bool {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let mut builder = app
        .dialog()
        .message(message)
        .title("Annotate Console")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            ok.into(),
            cancel.into(),
        ));
    if let Some(w) = app.get_webview_window(TOOLBAR) {
        builder = builder.parent(&w);
    }
    builder.blocking_show()
}

#[tauri::command]
pub async fn clear(app: AppHandle) -> R<()> {
    lock(&app).require_idle()?;
    let a = app.clone();
    let yes = tauri::async_runtime::spawn_blocking(move || {
        confirm(&a, "Discard all unsent annotations?", "Discard", "Keep")
    })
    .await
    .unwrap_or(false);
    let mut inner = lock(&app);
    if yes && !inner.busy() {
        inner.drafts.clear();
        inner.selected_id = None;
        inner.new_batch();
        drop(inner);
        flow::close_detail(&app);
        app::changed(&app);
    }
    Ok(())
}

#[tauri::command]
pub async fn send(
    app: AppHandle,
    window: WebviewWindow,
    target_id: String,
    deliver_as: String,
) -> R<Value> {
    only(&window, &[TOOLBAR], "Only the Console can send")?;
    let (drafts, batch_id, destination, record, settings) = {
        let mut inner = lock(&app);
        inner.require_idle()?;
        if inner.drafts.is_empty() {
            return Err("No annotations ready".into());
        }
        if let Some(i) = inner.drafts.iter().position(|d| !d.ready()) {
            return Err(format!(
                "Annotation {} has a numbered mark without a comment.",
                i + 1
            ));
        }
        if inner.drafts.len() > inner.settings.max_annotations {
            return Err("Batch exceeds the configured annotation limit".into());
        }
        if target_id != inner.destination_id {
            return Err(
                "Destination changed. Review the destination dropdown and Send again.".into(),
            );
        }
        let destination = inner
            .destination()
            .ok_or("Selected destination is offline. Choose an active destination.")?;
        if !destination.modes.iter().any(|m| m.id == deliver_as) {
            return Err("Invalid delivery mode".into());
        }
        if let Some(previous) = inner.attempts.get(&inner.batch_id) {
            if previous != &destination.id {
                return Err("This batch was already attempted for another destination. Retry the original destination or change the batch before switching.".into());
            }
        }
        let record = inner
            .live
            .iter()
            .find(|(_, d)| d.id == destination.id)
            .map(|(r, _)| r.clone());
        if destination.group == "session" && record.is_none() {
            return Err(
                "Selected session is offline. Annotations have not been redirected.".into(),
            );
        }
        let batch_id = inner.batch_id.clone();
        inner
            .attempts
            .insert(batch_id.clone(), destination.id.clone());
        if inner.attempts.len() > 100 {
            let key = inner.attempts.keys().next().cloned().unwrap();
            inner.attempts.remove(&key);
        }
        inner.sending = true;
        (
            inner.drafts.clone(),
            batch_id,
            destination,
            record,
            inner.settings.clone(),
        )
    };
    app::changed(&app);
    app::remember_console(&app);
    // Desktop paste needs the target app focused, so keep the Console out of the way.
    let pastes = matches!(destination.group.as_str(), "desktop" | "web");
    if pastes {
        app::hide_console(&app);
    }
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::deliver::deliver(
            &drafts,
            &batch_id,
            &destination,
            record.as_ref(),
            &deliver_as,
            &settings,
        )
    })
    .await
    .map_err(|e| e.to_string())?;
    match result {
        Ok(outcome) => {
            let keep = {
                let mut inner = lock(&app);
                inner.drafts.clear();
                inner.selected_id = None;
                inner.new_batch();
                inner.sending = false;
                inner.settings.keep_open_after_send
            };
            flow::close_detail(&app);
            if keep {
                app::status(&app, outcome.message.clone());
                if pastes {
                    app::show_console(&app);
                }
                app::changed(&app);
            } else {
                let a = app.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    flow::quit_now(&a);
                });
            }
            Ok(json!({ "ok": true, "message": outcome.message, "savedTo": outcome.saved_to }))
        }
        Err(error) => {
            lock(&app).sending = false;
            if pastes {
                app::show_console(&app);
            }
            app::status(&app, error.clone());
            app::changed(&app);
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn quit(app: AppHandle) -> R<()> {
    let (sending, dirty) = {
        let inner = lock(&app);
        (
            inner.sending,
            !inner.drafts.is_empty() || inner.frozen.is_some() || inner.capturing,
        )
    };
    if sending {
        app::status(
            &app,
            "Sending annotations… The Console will close when delivery succeeds.",
        );
        return Ok(());
    }
    if dirty {
        let a = app.clone();
        let yes = tauri::async_runtime::spawn_blocking(move || {
            confirm(
                &a,
                "Close Annotate Console and discard unsent annotations?",
                "Discard and close",
                "Keep working",
            )
        })
        .await
        .unwrap_or(false);
        if !yes {
            return Ok(());
        }
    }
    flow::quit_now(&app);
    Ok(())
}

/// Full-resolution marked screenshot (for copying).
#[tauri::command]
pub fn annotation_image(app: AppHandle, id: String) -> R<String> {
    let inner = lock(&app);
    let item = inner
        .drafts
        .iter()
        .find(|d| d.id == id)
        .ok_or("Annotation no longer exists")?;
    Ok(format!("data:image/png;base64,{}", item.image))
}

/// Native clipboard fallback when the web clipboard is unavailable.
#[tauri::command]
pub fn clipboard_write(kind: String, data: String, alt: Option<String>) -> R<()> {
    match kind.as_str() {
        "image" => crate::paste::set_image_b64(batch::strip_data_url(&data)),
        "html" => crate::paste::set_html(&data, alt.as_deref().unwrap_or("")),
        "text" => crate::paste::set_text(&data),
        _ => Err("Unknown clipboard format".into()),
    }
}

#[tauri::command]
pub async fn choose_folder(app: AppHandle, window: WebviewWindow) -> R<String> {
    only(
        &window,
        &[TOOLBAR],
        "Only the Console can choose the folder",
    )?;
    use tauri_plugin_dialog::DialogExt;
    let start = lock(&app).settings.project_dir.clone();
    let a = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = a
            .dialog()
            .file()
            .set_title("Project folder for new harness sessions");
        if !start.is_empty() {
            dialog = dialog.set_directory(start);
        }
        dialog.blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(lock(&app).settings.project_dir.clone());
    };
    let mut inner = lock(&app);
    let mut next = inner.settings.clone();
    next.project_dir = path.display().to_string();
    inner.save_settings(next)?;
    drop(inner);
    app::changed(&app);
    Ok(path.display().to_string())
}

#[tauri::command]
pub fn integrations() -> Value {
    json!({ "items": crate::integrations::list(), "snippet": crate::integrations::mcp_snippet(), "terminals": terminals() })
}

fn terminals() -> Vec<&'static str> {
    if cfg!(target_os = "macos") {
        let mut found = vec!["Terminal"];
        for (name, app) in [
            ("iTerm", "iTerm"),
            ("Ghostty", "Ghostty"),
            ("WezTerm", "WezTerm"),
            ("kitty", "kitty"),
            ("Alacritty", "Alacritty"),
            ("Warp", "Warp"),
        ] {
            if std::path::Path::new(&format!("/Applications/{app}.app")).exists() {
                found.push(name);
            }
        }
        found
    } else if cfg!(windows) {
        vec!["wt", "cmd"]
    } else {
        [
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
        .into_iter()
        .filter(|t| crate::harness::which_bin(t).is_some())
        .collect()
    }
}

#[tauri::command]
pub async fn integration_set(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
    install: bool,
) -> R<String> {
    only(
        &window,
        &[TOOLBAR],
        "Only the Console can change integrations",
    )?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        if install {
            crate::integrations::install(&id)
        } else {
            crate::integrations::uninstall(&id)
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    app::changed(&app);
    result
}

#[tauri::command]
pub async fn rescan(app: AppHandle) -> R<()> {
    let statics = tauri::async_runtime::spawn_blocking(|| {
        crate::harness::rescan();
        crate::harness::static_destinations()
    })
    .await
    .map_err(|e| e.to_string())?;
    lock(&app).statics = statics;
    app::changed(&app);
    Ok(())
}

#[tauri::command]
pub fn open_folder(app: AppHandle) -> R<()> {
    let dir = lock(&app).settings.project_dir.clone();
    if dir.is_empty() {
        return Err("No folder chosen".into());
    }
    crate::deliver::open_path(&dir)
}

pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        state,
        settings,
        shortcut_recording,
        shortcut_capture,
        console_size,
        detail_open,
        detail,
        detail_edit,
        detail_save,
        capture_kind,
        desktop_kind,
        detail_show,
        detail_close,
        developer_page,
        target,
        desktop_config,
        desktop_ready,
        desktop_mark,
        capture,
        select,
        crop_app,
        mark,
        frozen,
        cancel,
        add,
        remove,
        comment,
        clear,
        send,
        quit,
        annotation_image,
        clipboard_write,
        choose_folder,
        integrations,
        integration_set,
        rescan,
        open_folder
    ]
}
