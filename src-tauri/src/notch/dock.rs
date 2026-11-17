//! Docking maths for the notch, in physical pixels. Pure and unit-tested.
//!
//! The notch is always attached to one edge of a monitor's *work area* (the
//! screen minus the taskbar). Its place along that edge is the "dock point":
//! the centre of the notch body.

use serde::{Deserialize, Serialize};

use crate::config::Edge;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// What the notch UI needs the window to be, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Shape {
    /// The edge this shape was laid out for. A shape for another edge is ignored.
    pub edge: Edge,
    pub w: f64,
    pub h: f64,
    /// Distance from the window's start (top or left) to the dock point.
    pub anchor: f64,
    /// Room the content needs before and after the dock point, so it stays on screen.
    pub min_before: f64,
    pub min_after: f64,
    /// The part of the window that is visible and clickable now, relative to the
    /// window. The window itself is a stable envelope; only this region changes
    /// as the notch animates, so the content never moves when it grows or shrinks.
    #[serde(default)]
    pub region: Option<Region>,
}

/// A rectangle in logical pixels, relative to the window's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The window region in physical pixels (left, top, right, bottom), rounded
/// outwards and kept inside a `w` × `h` window.
pub fn region_rect(region: &Region, scale: f64, w: i32, h: i32) -> (i32, i32, i32, i32) {
    let left = ((region.x * scale).floor() as i32).clamp(0, w);
    let top = ((region.y * scale).floor() as i32).clamp(0, h);
    let right = (((region.x + region.w) * scale).ceil() as i32).clamp(left, w);
    let bottom = (((region.y + region.h) * scale).ceil() as i32).clamp(top, h);
    (left, top, right, bottom)
}

/// Where the notch ended up, in logical pixels, for the UI to lay out against.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Placement {
    pub edge: Edge,
    /// Distance from the dock point to the start / end of the work area along the edge.
    pub before: f64,
    pub after: f64,
    /// Work-area size across the edge (how far content may extend into the screen).
    pub depth: f64,
    pub scale: f64,
}

/// (start, length) of the work area along an edge.
pub fn along_range(work: Rect, edge: Edge) -> (i32, i32) {
    if edge.is_vertical() {
        (work.top, work.bottom - work.top)
    } else {
        (work.left, work.right - work.left)
    }
}

pub fn depth_of(work: Rect, edge: Edge) -> i32 {
    if edge.is_vertical() {
        work.right - work.left
    } else {
        work.bottom - work.top
    }
}

/// Distance from a point to each edge of the work area.
fn distance(work: Rect, edge: Edge, x: i32, y: i32) -> i32 {
    match edge {
        Edge::Left => x - work.left,
        Edge::Right => work.right - x,
        Edge::Top => y - work.top,
        Edge::Bottom => work.bottom - y,
    }
}

/// The edge nearest to the pointer. The current edge is "sticky": another edge
/// must be closer by `sticky` pixels before the notch switches, so it does not
/// flicker between two edges near a corner.
pub fn nearest_edge(work: Rect, x: i32, y: i32, current: Edge, sticky: i32) -> Edge {
    let mut best = current;
    let mut best_distance = distance(work, current, x, y) - sticky;
    for edge in [Edge::Right, Edge::Left, Edge::Top, Edge::Bottom] {
        let d = distance(work, edge, x, y);
        if d < best_distance {
            best = edge;
            best_distance = d;
        }
    }
    best
}

/// Lowest and highest allowed dock point (absolute, along the edge). If the
/// content is longer than the work area the two collapse to the middle.
pub fn limits(start: i32, len: i32, min_before: f64, min_after: f64) -> (f64, f64) {
    let lo = start as f64 + min_before;
    let hi = (start + len) as f64 - min_after;
    if lo > hi {
        let mid = (lo + hi) / 2.0;
        (mid, mid)
    } else {
        (lo, hi)
    }
}

/// Keeps the dock point inside the limits.
pub fn clamp(point: f64, lo: f64, hi: f64) -> f64 {
    point.max(lo).min(hi)
}

/// Rubber-banding: past a limit the notch follows the pointer with growing
/// resistance and never moves more than `max_over` beyond it.
pub fn rubber_band(point: f64, lo: f64, hi: f64, max_over: f64) -> f64 {
    let soften = |over: f64| max_over * (1.0 - 1.0 / (over / max_over * 0.55 + 1.0));
    if point < lo {
        lo - soften(lo - point)
    } else if point > hi {
        hi + soften(point - hi)
    } else {
        point
    }
}

/// Snaps to the middle of the edge when released within `threshold` of it.
pub fn snap_to_centre(point: f64, start: i32, len: i32, threshold: f64) -> f64 {
    let centre = start as f64 + len as f64 / 2.0;
    if (point - centre).abs() <= threshold {
        centre
    } else {
        point
    }
}

/// Saved position (0..=1 along the edge) → dock point.
pub fn from_fraction(fraction: f64, start: i32, len: i32) -> f64 {
    start as f64 + fraction.clamp(0.0, 1.0) * len as f64
}

/// Dock point → saved position (0..=1).
pub fn to_fraction(point: f64, start: i32, len: i32) -> f64 {
    if len <= 0 {
        return 0.5;
    }
    ((point - start as f64) / len as f64).clamp(0.0, 1.0)
}

/// The native window rectangle (x, y, w, h) for a shape docked at `point`.
/// The window is flush with the edge and never extends past it, so it cannot
/// cover the taskbar side of the work area.
pub fn window_rect(work: Rect, edge: Edge, point: f64, shape: &Shape, scale: f64) -> (i32, i32, i32, i32) {
    let w = (shape.w * scale).round() as i32;
    let h = (shape.h * scale).round() as i32;
    let along = (point - shape.anchor * scale).round() as i32;
    match edge {
        Edge::Left => (work.left, along, w, h),
        Edge::Right => (work.right - w, along, w, h),
        Edge::Top => (along, work.top, w, h),
        Edge::Bottom => (along, work.bottom - h, w, h),
    }
}

pub fn placement(work: Rect, edge: Edge, point: f64, scale: f64) -> Placement {
    let (start, len) = along_range(work, edge);
    Placement {
        edge,
        before: (point - start as f64) / scale,
        after: ((start + len) as f64 - point) / scale,
        depth: depth_of(work, edge) as f64 / scale,
        scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1040 };
    // A second monitor to the left of the primary, with negative coordinates.
    const LEFT_MONITOR: Rect = Rect { left: -1280, top: 100, right: 0, bottom: 1124 };

    #[test]
    fn nearest_edge_picks_the_closest() {
        assert_eq!(nearest_edge(WORK, 1900, 500, Edge::Left, 0), Edge::Right);
        assert_eq!(nearest_edge(WORK, 30, 500, Edge::Right, 0), Edge::Left);
        assert_eq!(nearest_edge(WORK, 900, 12, Edge::Right, 0), Edge::Top);
        assert_eq!(nearest_edge(WORK, 900, 1030, Edge::Right, 0), Edge::Bottom);
        assert_eq!(nearest_edge(LEFT_MONITOR, -1270, 600, Edge::Right, 0), Edge::Left);
    }

    #[test]
    fn current_edge_is_sticky_near_corners() {
        // 40 px from the right edge, 30 px from the top: top is nearer, but not by the sticky margin.
        assert_eq!(nearest_edge(WORK, 1880, 30, Edge::Right, 24), Edge::Right);
        // 40 px from the right, 10 px from the top: now clearly the top edge.
        assert_eq!(nearest_edge(WORK, 1880, 10, Edge::Right, 24), Edge::Top);
        // With no current-edge advantage the nearer edge wins outright.
        assert_eq!(nearest_edge(WORK, 1880, 30, Edge::Right, 0), Edge::Top);
    }

    #[test]
    fn limits_and_clamping() {
        let (start, len) = along_range(WORK, Edge::Right);
        assert_eq!((start, len), (0, 1040));
        let (lo, hi) = limits(start, len, 180.0, 240.0);
        assert_eq!((lo, hi), (180.0, 800.0));
        assert_eq!(clamp(50.0, lo, hi), 180.0);
        assert_eq!(clamp(500.0, lo, hi), 500.0);
        assert_eq!(clamp(2000.0, lo, hi), 800.0);
        // Content longer than the screen: pinned to the middle instead of inverting.
        let (lo, hi) = limits(0, 300, 200.0, 200.0);
        assert_eq!((lo, hi), (150.0, 150.0));
    }

    #[test]
    fn rubber_band_resists_and_is_bounded() {
        let (lo, hi) = (100.0, 900.0);
        assert_eq!(rubber_band(500.0, lo, hi, 40.0), 500.0);
        let a = rubber_band(950.0, lo, hi, 40.0);
        let b = rubber_band(1100.0, lo, hi, 40.0);
        let c = rubber_band(100_000.0, lo, hi, 40.0);
        assert!(a > hi && a < 950.0, "moves less than the pointer");
        assert!(b > a && c > b, "keeps moving, with more resistance");
        assert!(c < hi + 40.0, "never more than max_over past the limit");
        assert!(rubber_band(20.0, lo, hi, 40.0) < lo && rubber_band(20.0, lo, hi, 40.0) > lo - 40.0);
    }

    #[test]
    fn centre_snapping() {
        assert_eq!(snap_to_centre(530.0, 0, 1040, 24.0), 520.0);
        assert_eq!(snap_to_centre(496.0, 0, 1040, 24.0), 520.0);
        assert_eq!(snap_to_centre(560.0, 0, 1040, 24.0), 560.0);
        // On a monitor that does not start at 0.
        assert_eq!(snap_to_centre(-630.0, -1280, 1280, 24.0), -640.0);
    }

    #[test]
    fn fractions_round_trip() {
        let (start, len) = along_range(LEFT_MONITOR, Edge::Top);
        assert_eq!(from_fraction(0.5, start, len), -640.0);
        assert_eq!(to_fraction(-640.0, start, len), 0.5);
        assert_eq!(to_fraction(5000.0, start, len), 1.0);
        assert_eq!(from_fraction(7.0, start, len), 0.0);
        assert_eq!(to_fraction(10.0, 0, 0), 0.5);
    }

    #[test]
    fn window_rect_is_flush_with_each_edge() {
        let shape = |edge| Shape { edge, w: 80.0, h: 300.0, anchor: 150.0, min_before: 0.0, min_after: 0.0, region: None };
        assert_eq!(window_rect(WORK, Edge::Right, 520.0, &shape(Edge::Right), 1.0), (1840, 370, 80, 300));
        assert_eq!(window_rect(WORK, Edge::Left, 520.0, &shape(Edge::Left), 1.0), (0, 370, 80, 300));
        let flat = |edge| Shape { edge, w: 300.0, h: 80.0, anchor: 150.0, min_before: 0.0, min_after: 0.0, region: None };
        assert_eq!(window_rect(WORK, Edge::Top, 960.0, &flat(Edge::Top), 1.0), (810, 0, 300, 80));
        assert_eq!(window_rect(WORK, Edge::Bottom, 960.0, &flat(Edge::Bottom), 1.0), (810, 960, 300, 80));
        // 150 % scaling: sizes and anchor scale, the edge stays flush.
        assert_eq!(window_rect(WORK, Edge::Right, 520.0, &shape(Edge::Right), 1.5), (1800, 295, 120, 450));
    }

    #[test]
    fn region_rounds_outwards_and_stays_inside_the_window() {
        let r = Region { x: 10.2, y: 0.0, w: 50.5, h: 300.0 };
        assert_eq!(region_rect(&r, 1.0, 80, 300), (10, 0, 61, 300));
        assert_eq!(region_rect(&r, 1.5, 120, 450), (15, 0, 92, 450));
        // Past the window: clipped to it, never inverted.
        let wide = Region { x: -5.0, y: 290.0, w: 200.0, h: 40.0 };
        assert_eq!(region_rect(&wide, 1.0, 80, 300), (0, 290, 80, 300));
    }

    #[test]
    fn placement_reports_logical_room() {
        let p = placement(WORK, Edge::Right, 260.0, 2.0);
        assert_eq!((p.before, p.after, p.depth), (130.0, 390.0, 960.0));
    }
}
