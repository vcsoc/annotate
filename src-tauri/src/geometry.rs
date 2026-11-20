//! Pure geometry: mark validation, app targeting, note placement and Console
//! layout/docking. Ported 1:1 from the original Console so behaviour matches.
use crate::model::{AppWindow, Point, Rect};

pub const SNAP_GAP: f64 = 6.0;
pub const SNAP_THRESHOLD: f64 = 32.0;
pub const TIMELINE_WIDTH: f64 = 320.0;
pub const TIMELINE_HEIGHT_RATIO: f64 = 0.6;
pub const TIMELINE_MIN: (f64, f64) = (280.0, 360.0);
pub const DETAIL_SIZE: (f64, f64) = (640.0, 560.0);
pub const DETAIL_GAP: f64 = 8.0;
pub const NOTE_SIZE: (f64, f64) = (340.0, 246.0);

pub fn pill_size(mode: &str) -> (f64, f64) {
    match mode {
        "expanded" => (600.0, 440.0),
        "settings" => (600.0, 560.0),
        _ => (600.0, 56.0),
    }
}

// ---------------------------------------------------------------- marks

pub fn validate_mark(kind: &str, points: &[Point]) -> Result<Vec<Point>, String> {
    let bad = !["rectangle", "freehand"].contains(&kind)
        || points.len() < 2
        || points.len() > 1000
        || (kind == "rectangle" && points.len() != 2)
        || points
            .iter()
            .any(|p| p.iter().any(|n| !n.is_finite() || *n < 0.0 || *n > 1.0));
    if bad {
        return Err("Invalid annotation mark".into());
    }
    let (min_x, max_x, min_y, max_y) = extent(points);
    if max_x - min_x < 0.004 || max_y - min_y < 0.004 {
        return Err("Draw a larger area to annotate".into());
    }
    Ok(points.to_vec())
}

fn extent(points: &[Point]) -> (f64, f64, f64, f64) {
    let xs = points.iter().map(|p| p[0]);
    let ys = points.iter().map(|p| p[1]);
    (
        xs.clone().fold(f64::INFINITY, f64::min),
        xs.fold(f64::NEG_INFINITY, f64::max),
        ys.clone().fold(f64::INFINITY, f64::min),
        ys.fold(f64::NEG_INFINITY, f64::max),
    )
}

pub fn clamp_points(points: &[Point]) -> Vec<Point> {
    points
        .iter()
        .map(|p| [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)])
        .collect()
}

pub fn points_in_app(points: &[Point], target: &Rect) -> Vec<Point> {
    points
        .iter()
        .map(|p| {
            [
                (p[0] - target.x) / target.width,
                (p[1] - target.y) / target.height,
            ]
        })
        .collect()
}

pub fn relative_mark(region: &Rect, app: &Rect) -> Vec<Point> {
    let c = |n: f64| n.clamp(0.0, 1.0);
    vec![
        [
            c((region.x - app.x) / app.width),
            c((region.y - app.y) / app.height),
        ],
        [
            c((region.x + region.width - app.x) / app.width),
            c((region.y + region.height - app.y) / app.height),
        ],
    ]
}

/// Validates desktop-global coordinates from a selection surface and returns the bounding region.
pub fn desktop_selection(kind: &str, points: &[Point], displays: &[Rect]) -> Result<Rect, String> {
    if !["rectangle", "freehand"].contains(&kind)
        || points.len() < 2
        || points.len() > 1000
        || (kind == "rectangle" && points.len() != 2)
    {
        return Err("Invalid desktop mark".into());
    }
    let on_display = |p: &Point| {
        displays
            .iter()
            .any(|d| p[0] >= d.x && p[0] <= d.x + d.width && p[1] >= d.y && p[1] <= d.y + d.height)
    };
    if points
        .iter()
        .any(|p| !p[0].is_finite() || !p[1].is_finite() || !on_display(p))
    {
        return Err("Mark must stay on the connected displays".into());
    }
    let (min_x, max_x, min_y, max_y) = extent(points);
    let x = min_x.floor();
    let y = min_y.floor();
    let region = Rect::new(x, y, max_x.ceil() - x, max_y.ceil() - y);
    if region.width < 3.0 || region.height < 3.0 {
        return Err("Draw a larger marked area".into());
    }
    Ok(region)
}

// ---------------------------------------------------------------- app targeting

fn order(a: &AppWindow, b: &AppWindow) -> std::cmp::Ordering {
    b.floating
        .cmp(&a.floating)
        .then(a.focus_order.cmp(&b.focus_order))
}

pub fn application_for_region<'a>(
    region: &Rect,
    windows: &'a [AppWindow],
) -> Result<&'a AppWindow, String> {
    let mut candidates: Vec<&AppWindow> = windows
        .iter()
        .filter(|w| {
            region.x >= w.x - 1.0
                && region.y >= w.y - 1.0
                && region.x + region.width <= w.x + w.width + 1.0
                && region.y + region.height <= w.y + w.height + 1.0
        })
        .collect();
    candidates
        .sort_by(|a, b| order(a, b).then((a.width * a.height).total_cmp(&(b.width * b.height))));
    candidates.first().copied().ok_or_else(|| "Draw the marked area inside one application window. The full app—not just the marked area—must be captured.".into())
}

#[derive(Clone, Debug, PartialEq)]
pub enum Adjusted {
    No,
    Clipped,
    Extended,
}

#[derive(Clone, Debug)]
pub struct TargetChoice {
    pub app: AppWindow,
    pub target: Rect,
    pub adjusted: Adjusted,
}

/// A mark that crosses the app edge: pick the window where drawing started (else the
/// one it overlaps most). `clip` keeps that window and trims the mark; `extend`
/// widens the capture to the window plus the whole mark.
pub fn target_for_mark(
    region: &Rect,
    windows: &[AppWindow],
    overflow: &str,
    start: Option<Point>,
) -> Result<TargetChoice, String> {
    if let Ok(app) = application_for_region(region, windows) {
        return Ok(TargetChoice {
            app: app.clone(),
            target: app.rect(),
            adjusted: Adjusted::No,
        });
    }
    let overlap = |w: &AppWindow| {
        (f64::min(region.x + region.width, w.x + w.width) - f64::max(region.x, w.x)).max(0.0)
            * (f64::min(region.y + region.height, w.y + w.height) - f64::max(region.y, w.y))
                .max(0.0)
    };
    let contains = |w: &AppWindow, p: Option<Point>| {
        p.is_some_and(|p| {
            p[0] >= w.x && p[0] <= w.x + w.width && p[1] >= w.y && p[1] <= w.y + w.height
        })
    };
    let touching: Vec<&AppWindow> = windows.iter().filter(|w| overlap(w) > 0.0).collect();
    if touching.is_empty() {
        return Err("Start the mark inside the application you want to annotate.".into());
    }
    let mut started: Vec<&AppWindow> = touching
        .iter()
        .copied()
        .filter(|w| contains(w, start))
        .collect();
    started.sort_by(|a, b| order(a, b));
    let app = match started.first() {
        Some(app) => (*app).clone(),
        None => {
            let mut by_overlap = touching.clone();
            by_overlap.sort_by(|a, b| overlap(b).total_cmp(&overlap(a)).then(order(a, b)));
            by_overlap[0].clone()
        }
    };
    if overflow == "clip" {
        return Ok(TargetChoice {
            target: app.rect(),
            app,
            adjusted: Adjusted::Clipped,
        });
    }
    let x = f64::min(app.x, region.x).floor();
    let y = f64::min(app.y, region.y).floor();
    let target = Rect::new(
        x,
        y,
        f64::max(app.x + app.width, region.x + region.width).ceil() - x,
        f64::max(app.y + app.height, region.y + region.height).ceil() - y,
    );
    Ok(TargetChoice {
        app,
        target,
        adjusted: Adjusted::Extended,
    })
}

/// Note popup beside a mark, kept inside the work area containing the mark.
pub fn place_note(mark: &Rect, areas: &[Rect], size: (f64, f64)) -> Result<Rect, String> {
    let cx = mark.x + mark.width / 2.0;
    let cy = mark.y + mark.height / 2.0;
    let area = areas
        .iter()
        .find(|a| a.contains_point([cx, cy]))
        .or(areas.first())
        .ok_or("No display is available for the annotation note")?;
    let width = size.0.min(area.width);
    let height = size.1.min(area.height);
    let gap = 12.0;
    let candidates = [
        (mark.x + mark.width + gap, mark.y),
        (mark.x - width - gap, mark.y),
        (mark.x, mark.y + mark.height + gap),
        (mark.x, mark.y - height - gap),
    ];
    let chosen = candidates
        .iter()
        .find(|(x, y)| {
            *x >= area.x
                && *y >= area.y
                && x + width <= area.x + area.width
                && y + height <= area.y + area.height
        })
        .unwrap_or(&candidates[0]);
    Ok(Rect::new(
        chosen
            .0
            .min(area.x + area.width - width)
            .max(area.x)
            .round(),
        chosen
            .1
            .min(area.y + area.height - height)
            .max(area.y)
            .round(),
        width,
        height,
    ))
}

// ---------------------------------------------------------------- Console layout

fn overlap_area(a: &Rect, b: &Rect) -> f64 {
    (f64::min(a.x + a.width, b.x + b.width) - f64::max(a.x, b.x)).max(0.0)
        * (f64::min(a.y + a.height, b.y + b.height) - f64::max(a.y, b.y)).max(0.0)
}

/// Work area the window belongs to: the one containing its centre, else the most overlapped.
pub fn area_for(bounds: &Rect, areas: &[Rect]) -> Option<Rect> {
    let usable: Vec<Rect> = areas.iter().copied().filter(Rect::valid).collect();
    if !bounds.valid() || usable.is_empty() {
        return None;
    }
    let c = [
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height / 2.0,
    ];
    usable
        .iter()
        .copied()
        .find(|a| a.contains_point(c))
        .or_else(|| {
            usable
                .iter()
                .copied()
                .fold(None, |best: Option<Rect>, a| match best {
                    Some(b) if overlap_area(bounds, &b) >= overlap_area(bounds, &a) => Some(b),
                    _ => Some(a),
                })
        })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Side {
    Left,
    Right,
}

/// Docked bounds for a window released near a side edge, or None.
pub fn dock_bounds(
    bounds: &Rect,
    areas: &[Rect],
    height_ratio: Option<f64>,
) -> Option<(Side, Rect)> {
    let area = area_for(bounds, areas)?;
    let left = bounds.x - area.x;
    let right = area.x + area.width - (bounds.x + bounds.width);
    let side = if left <= SNAP_THRESHOLD && left <= right {
        Side::Left
    } else if right <= SNAP_THRESHOLD {
        Side::Right
    } else {
        return None;
    };
    let width = bounds.width.min(area.width - 2.0 * SNAP_GAP).round();
    let full = area.height - 2.0 * SNAP_GAP;
    let height = match height_ratio {
        Some(r) => full.min((360f64.min(full)).max(area.height * r)),
        None => full,
    }
    .round();
    let x = if side == Side::Left {
        area.x + SNAP_GAP
    } else {
        area.x + area.width - SNAP_GAP - width
    }
    .round();
    let y = match height_ratio {
        Some(_) => bounds
            .y
            .max(area.y + SNAP_GAP)
            .min(area.y + area.height - SNAP_GAP - height),
        None => area.y + SNAP_GAP,
    }
    .round();
    Some((side, Rect::new(x, y, width, height)))
}

/// A docked window dragged away from the edge gets its pre-dock height back.
pub fn undock_bounds(bounds: &Rect, areas: &[Rect], height: f64) -> Option<Rect> {
    let area = area_for(bounds, areas)?;
    let h = height.min(area.height).round();
    let y = bounds.y.max(area.y).min(area.y + area.height - h).round();
    Some(Rect::new(bounds.x.round(), y, bounds.width.round(), h))
}

pub fn same(a: &Rect, b: &Rect) -> bool {
    (a.x - b.x).abs() <= 1.0
        && (a.y - b.y).abs() <= 1.0
        && (a.width - b.width).abs() <= 1.0
        && (a.height - b.height).abs() <= 1.0
}

fn fit_in(b: Rect, area: &Rect) -> Rect {
    let width = b.width.min(area.width);
    let height = b.height.min(area.height);
    Rect::new(
        b.x.max(area.x).min(area.x + area.width - width).round(),
        b.y.max(area.y).min(area.y + area.height - height).round(),
        width,
        height,
    )
}

pub fn timeline_size(area: &Rect) -> (f64, f64) {
    let full = area.height - 2.0 * SNAP_GAP;
    (
        TIMELINE_WIDTH.min(area.width - 2.0 * SNAP_GAP),
        full.min((TIMELINE_MIN.1.min(full)).max(area.height * TIMELINE_HEIGHT_RATIO))
            .round(),
    )
}

/// Bounds for a Console design at (or near) its current position. `initial`
/// docks a fresh timeline right/centred and places a fresh pill top-centre.
pub fn layout_bounds(
    bounds: Option<Rect>,
    areas: &[Rect],
    layout: &str,
    mode: &str,
    initial: bool,
) -> Result<Rect, String> {
    let probe = bounds.map(|b| Rect::new(b.x, b.y, b.width.max(1.0), b.height.max(1.0)));
    let area = probe
        .and_then(|b| area_for(&b, areas))
        .or_else(|| area_for(&Rect::new(0.0, 0.0, 1.0, 1.0), areas))
        .ok_or("No display work area is available")?;
    if layout == "pill" {
        let (w, h) = pill_size(mode);
        return Ok(match bounds {
            Some(b) if !initial => fit_in(Rect::new(b.x, b.y, w, h), &area),
            _ => fit_in(
                Rect::new(area.x + (area.width - w) / 2.0, area.y + 12.0, w, h),
                &area,
            ),
        });
    }
    let (w, h) = timeline_size(&area);
    let right = area.x + area.width - SNAP_GAP - w;
    let Some(b) = bounds.filter(|_| !initial) else {
        return Ok(Rect::new(
            right.round(),
            (area.y + (area.height - h) / 2.0).round(),
            w,
            h,
        ));
    };
    let near_left = b.x - area.x <= SNAP_THRESHOLD;
    let near_right = area.x + area.width - (b.x + b.width) <= SNAP_THRESHOLD;
    let x = if near_left {
        area.x + SNAP_GAP
    } else if near_right {
        right
    } else {
        b.x
    };
    let y =
        b.y.max(area.y + SNAP_GAP)
            .min(area.y + area.height - SNAP_GAP - h);
    Ok(fit_in(Rect::new(x, y, w, h), &area))
}

/// Popup beside `anchor` (the Console), level with the row at `top` px inside it.
pub fn beside_bounds(anchor: &Rect, areas: &[Rect], top: f64, size: (f64, f64)) -> Option<Rect> {
    let area = area_for(anchor, areas)?;
    let width = size.0.min(area.width - 2.0 * SNAP_GAP);
    let height = size.1.min(area.height - 2.0 * SNAP_GAP);
    let room_left = anchor.x - DETAIL_GAP - (area.x + SNAP_GAP);
    let room_right = area.x + area.width - SNAP_GAP - (anchor.x + anchor.width + DETAIL_GAP);
    let prefer_left = anchor.x + anchor.width / 2.0 > area.x + area.width / 2.0;
    let left = if room_left >= width && (prefer_left || room_right < width) {
        true
    } else if room_right >= width {
        false
    } else {
        room_left >= room_right
    };
    let x = if left {
        (area.x + SNAP_GAP).max(anchor.x - DETAIL_GAP - width)
    } else {
        (area.x + area.width - SNAP_GAP - width).min(anchor.x + anchor.width + DETAIL_GAP)
    };
    let y = (anchor.y + top)
        .max(area.y + SNAP_GAP)
        .min(area.y + area.height - SNAP_GAP - height);
    Some(Rect::new(
        x.round(),
        y.round(),
        width.round(),
        height.round(),
    ))
}

/// Restores stored Console bounds onscreen after display changes.
pub fn console_bounds(
    value: Option<Rect>,
    areas: &[Rect],
    min: (f64, f64),
) -> Result<Rect, String> {
    let areas: Vec<Rect> = areas.iter().copied().filter(Rect::valid).collect();
    if areas.is_empty() {
        return Err("No display work area is available".into());
    }
    let size = |v: Option<Rect>, limit: (f64, f64)| {
        let w = v
            .map(|r| r.width)
            .filter(|n| n.is_finite())
            .unwrap_or(400.0)
            .round();
        let h = v
            .map(|r| r.height)
            .filter(|n| n.is_finite())
            .unwrap_or(600.0)
            .round();
        (
            limit.0.min(w.min(4000.0).max(min.0)),
            limit.1.min(h.min(4000.0).max(min.1)),
        )
    };
    let (w, h) = size(value, (4000.0, 4000.0));
    if let Some(v) = value.filter(|v| v.x.is_finite() && v.y.is_finite()) {
        let min_x = areas.iter().map(|a| a.x).fold(f64::INFINITY, f64::min);
        let min_y = areas.iter().map(|a| a.y).fold(f64::INFINITY, f64::min);
        let max_x = areas
            .iter()
            .map(|a| a.x + a.width)
            .fold(f64::NEG_INFINITY, f64::max);
        let max_y = areas
            .iter()
            .map(|a| a.y + a.height)
            .fold(f64::NEG_INFINITY, f64::max);
        let within = v.x >= min_x && v.y >= min_y && v.x + w <= max_x && v.y + h <= max_y;
        if within
            && areas.iter().any(|a| {
                f64::min(v.x + w, a.x + a.width) - f64::max(v.x, a.x) >= w.min(100.0)
                    && v.y >= a.y
                    && v.y + 24.0 <= a.y + a.height
            })
        {
            return Ok(Rect::new(v.x.round(), v.y.round(), w, h));
        }
        let dist = |r: &Rect| {
            ((v.x - (r.x + r.width / 2.0)).powi(2) + (v.y - (r.y + r.height / 2.0)).powi(2)).sqrt()
        };
        let area = *areas
            .iter()
            .min_by(|a, b| dist(a).total_cmp(&dist(b)))
            .unwrap();
        let (fw, fh) = size(Some(v), (area.width, area.height));
        return Ok(Rect::new(
            (area.x + (area.width - fw) / 2.0).round(),
            (area.y + (area.height - fh) / 2.0).round(),
            fw,
            fh,
        ));
    }
    let area = areas[0];
    let (fw, fh) = size(value, (area.width, area.height));
    Ok(Rect::new(
        (area.x + (area.width - fw) / 2.0).round(),
        (area.y + (area.height - fh) / 2.0).round(),
        fw,
        fh,
    ))
}

pub fn desktop_bounds(areas: &[Rect]) -> Result<Rect, String> {
    if areas.is_empty() {
        return Err("Cannot determine valid display bounds".into());
    }
    let x = areas.iter().map(|a| a.x).fold(f64::INFINITY, f64::min);
    let y = areas.iter().map(|a| a.y).fold(f64::INFINITY, f64::min);
    let width = areas
        .iter()
        .map(|a| a.x + a.width)
        .fold(f64::NEG_INFINITY, f64::max)
        - x;
    let height = areas
        .iter()
        .map(|a| a.y + a.height)
        .fold(f64::NEG_INFINITY, f64::max)
        - y;
    if width > 32768.0 || height > 32768.0 || width * height > 32.0 * 1024.0 * 1024.0 {
        return Err("Desktop span is too large to preserve safely".into());
    }
    Ok(Rect::new(x, y, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(x: f64, y: f64, w: f64, h: f64, order: i64) -> AppWindow {
        AppWindow {
            x,
            y,
            width: w,
            height: h,
            label: format!("w{order}"),
            native_id: order.to_string(),
            floating: false,
            focus_order: order,
        }
    }

    #[test]
    fn validates_marks() {
        assert!(validate_mark("rectangle", &[[0.1, 0.1], [0.5, 0.5]]).is_ok());
        assert!(validate_mark("rectangle", &[[0.1, 0.1], [0.5, 0.5], [0.6, 0.6]]).is_err());
        assert!(validate_mark("freehand", &[[0.1, 0.1], [0.101, 0.101]]).is_err());
        assert!(validate_mark("circle", &[[0.1, 0.1], [0.5, 0.5]]).is_err());
        assert!(validate_mark("rectangle", &[[0.1, 0.1], [1.5, 0.5]]).is_err());
    }

    #[test]
    fn picks_front_window_containing_region() {
        let windows = [
            win(0.0, 0.0, 1000.0, 800.0, 1),
            win(100.0, 100.0, 400.0, 300.0, 0),
        ];
        let region = Rect::new(150.0, 150.0, 50.0, 50.0);
        assert_eq!(
            application_for_region(&region, &windows).unwrap().label,
            "w0"
        );
    }

    #[test]
    fn overflow_clip_and_extend() {
        let windows = [win(100.0, 100.0, 400.0, 300.0, 0)];
        let region = Rect::new(450.0, 150.0, 100.0, 50.0);
        let clip = target_for_mark(&region, &windows, "clip", Some([460.0, 160.0])).unwrap();
        assert_eq!(clip.adjusted, Adjusted::Clipped);
        assert_eq!(clip.target, Rect::new(100.0, 100.0, 400.0, 300.0));
        let ext = target_for_mark(&region, &windows, "extend", Some([460.0, 160.0])).unwrap();
        assert_eq!(ext.adjusted, Adjusted::Extended);
        assert_eq!(ext.target, Rect::new(100.0, 100.0, 450.0, 300.0));
        assert!(
            target_for_mark(&Rect::new(900.0, 900.0, 10.0, 10.0), &windows, "clip", None).is_err()
        );
    }

    #[test]
    fn note_goes_right_then_left() {
        let areas = [Rect::new(0.0, 0.0, 1920.0, 1080.0)];
        let right = place_note(&Rect::new(100.0, 100.0, 200.0, 100.0), &areas, NOTE_SIZE).unwrap();
        assert_eq!((right.x, right.y), (312.0, 100.0));
        let left = place_note(&Rect::new(1700.0, 100.0, 200.0, 100.0), &areas, NOTE_SIZE).unwrap();
        assert_eq!(left.x, 1700.0 - 340.0 - 12.0);
    }

    #[test]
    fn docks_near_edges() {
        let areas = [Rect::new(0.0, 0.0, 1920.0, 1080.0)];
        let (side, b) =
            dock_bounds(&Rect::new(1600.0, 200.0, 300.0, 500.0), &areas, Some(0.6)).unwrap();
        assert_eq!(side, Side::Right);
        assert_eq!(b.x, 1920.0 - 6.0 - 300.0);
        assert_eq!(b.height, 648.0);
        assert!(dock_bounds(&Rect::new(800.0, 200.0, 300.0, 500.0), &areas, Some(0.6)).is_none());
        let undocked =
            undock_bounds(&Rect::new(800.0, 200.0, 300.0, 648.0), &areas, 500.0).unwrap();
        assert_eq!(undocked.height, 500.0);
    }

    #[test]
    fn initial_layouts() {
        let areas = [Rect::new(0.0, 0.0, 1920.0, 1080.0)];
        let t = layout_bounds(None, &areas, "timeline", "collapsed", true).unwrap();
        assert_eq!(
            (t.x, t.width, t.height),
            (1920.0 - 6.0 - 320.0, 320.0, 648.0)
        );
        let p = layout_bounds(None, &areas, "pill", "collapsed", true).unwrap();
        assert_eq!((p.x, p.y, p.width, p.height), (660.0, 12.0, 600.0, 56.0));
    }

    #[test]
    fn beside_prefers_centre() {
        let areas = [Rect::new(0.0, 0.0, 1920.0, 1080.0)];
        let b = beside_bounds(
            &Rect::new(1594.0, 216.0, 320.0, 648.0),
            &areas,
            40.0,
            DETAIL_SIZE,
        )
        .unwrap();
        assert_eq!(b.x, 1594.0 - 8.0 - 640.0);
        assert_eq!(b.y, 256.0);
    }

    #[test]
    fn console_bounds_recovers_offscreen() {
        let areas = [Rect::new(0.0, 0.0, 1920.0, 1080.0)];
        let b = console_bounds(
            Some(Rect::new(5000.0, 5000.0, 400.0, 600.0)),
            &areas,
            (320.0, 360.0),
        )
        .unwrap();
        assert!(b.x >= 0.0 && b.x + b.width <= 1920.0);
        let kept = console_bounds(
            Some(Rect::new(100.0, 100.0, 400.0, 600.0)),
            &areas,
            (320.0, 360.0),
        )
        .unwrap();
        assert_eq!(kept, Rect::new(100.0, 100.0, 400.0, 600.0));
    }

    #[test]
    fn desktop_selection_region() {
        let displays = [Rect::new(0.0, 0.0, 1000.0, 1000.0)];
        let r = desktop_selection("rectangle", &[[10.2, 10.7], [50.5, 60.1]], &displays).unwrap();
        assert_eq!(r, Rect::new(10.0, 10.0, 41.0, 51.0));
        assert!(desktop_selection("rectangle", &[[10.0, 10.0], [11.0, 11.0]], &displays).is_err());
        assert!(
            desktop_selection("rectangle", &[[10.0, 10.0], [1500.0, 11.0]], &displays).is_err()
        );
    }
}
