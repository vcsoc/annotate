//! Shared data types used by the Console, the capture pipeline and delivery.
use serde::{Deserialize, Serialize};

pub type Point = [f64; 2];

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    pub fn valid(&self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|n| n.is_finite())
            && self.width > 0.0
            && self.height > 0.0
    }
    pub fn contains_point(&self, p: Point) -> bool {
        p[0] >= self.x
            && p[0] < self.x + self.width
            && p[1] >= self.y
            && p[1] < self.y + self.height
    }
    pub fn round(&self) -> Self {
        Self::new(
            self.x.round(),
            self.y.round(),
            self.width.round(),
            self.height.round(),
        )
    }
}

/// A highlighted area: a two-point rectangle or a freehand outline, normalized
/// to the full application image (0..1 on both axes).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    pub kind: String,
    pub points: Vec<Point>,
    pub comment: String,
}

/// One saved annotation: the complete application screenshot with every mark
/// painted in, the unmarked `base` (kept for re-marking, never sent) and notes.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub id: String,
    /// Base64 PNG with the marks painted in.
    pub image: String,
    #[serde(skip_serializing)]
    pub base: Option<String>,
    /// Small data-URL preview for the Console list.
    #[serde(skip_serializing)]
    pub preview: String,
    pub comment: String,
    pub kind: String,
    pub points: Vec<Point>,
    pub marks: Vec<Mark>,
    pub width: u32,
    pub height: u32,
    pub source: String,
}

impl Draft {
    pub fn ready(&self) -> bool {
        !self.marks.is_empty() && self.marks.iter().all(|m| !m.comment.trim().is_empty())
    }
}

/// A window of another application that can be marked and captured.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppWindow {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub label: String,
    /// Native window id (CGWindowID, HWND, X11 id) or compositor address.
    pub native_id: String,
    pub floating: bool,
    pub focus_order: i64,
}

impl AppWindow {
    pub fn rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.width, self.height)
    }
}
