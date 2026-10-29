//! Feathered polygon masks that hide parts of a surface (windows, pillars, the audience).

use crate::geom::Vec2;
use serde::{Deserialize, Serialize};

/// A closed polygon in normalised surface space with a soft edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    /// Polygon vertices in order; the polygon is closed implicitly.
    pub points: Vec<Vec2>,
    /// Feather width in normalised units. `0` is a hard edge.
    #[serde(default)]
    pub feather: f64,
    /// When true the polygon is a hole (content hidden inside); when false the polygon is a
    /// window (content hidden outside).
    #[serde(default = "default_true")]
    pub subtract: bool,
}

fn default_true() -> bool {
    true
}

impl Mask {
    /// Builds a hard-edged hole mask.
    pub fn hole(points: Vec<Vec2>) -> Self {
        Self {
            points,
            feather: 0.0,
            subtract: true,
        }
    }

    /// Builds a hard-edged window mask.
    pub fn window(points: Vec<Vec2>) -> Self {
        Self {
            points,
            feather: 0.0,
            subtract: false,
        }
    }

    /// True when the polygon has at least three finite vertices.
    pub fn is_valid(&self) -> bool {
        self.points.len() >= 3
            && self.points.iter().all(|p| p.is_finite())
            && self.feather.is_finite()
    }

    /// Even-odd point-in-polygon test.
    pub fn contains(&self, p: Vec2) -> bool {
        let n = self.points.len();
        if n < 3 {
            return false;
        }
        let mut inside = false;
        let mut j = n - 1;
        for i in 0..n {
            let (a, b) = (self.points[i], self.points[j]);
            if (a.y > p.y) != (b.y > p.y) {
                let x = (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x;
                if p.x < x {
                    inside = !inside;
                }
            }
            j = i;
        }
        inside
    }

    /// Distance from `p` to the nearest polygon edge.
    pub fn edge_distance(&self, p: Vec2) -> f64 {
        let n = self.points.len();
        if n == 0 {
            return f64::INFINITY;
        }
        let mut best = f64::INFINITY;
        for i in 0..n {
            let a = self.points[i];
            let b = self.points[(i + 1) % n];
            best = best.min(segment_distance(p, a, b));
        }
        best
    }

    /// Visibility multiplier at `p`: `1` fully visible, `0` fully hidden, feathered between.
    pub fn coverage(&self, p: Vec2) -> f64 {
        if !self.is_valid() {
            return 1.0;
        }
        let inside = self.contains(p);
        let feather = self.feather.max(0.0);
        // Signed distance: positive inside, negative outside.
        let signed = if inside {
            self.edge_distance(p)
        } else {
            -self.edge_distance(p)
        };
        let inside_amount = if feather <= 0.0 {
            if inside {
                1.0
            } else {
                0.0
            }
        } else {
            // Feather straddles the edge: fully "inside" at +feather/2, fully outside at -feather/2.
            ((signed / feather) + 0.5).clamp(0.0, 1.0)
        };
        if self.subtract {
            1.0 - inside_amount
        } else {
            inside_amount
        }
    }
}

/// Distance from `p` to segment `ab`.
fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f64 {
    let ab = b - a;
    let len2 = ab.dot(ab);
    if len2 < 1e-18 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + ab.scale(t))
}

/// Combined coverage of several masks (product of individual coverages).
pub fn combined_coverage(masks: &[Mask], p: Vec2) -> f64 {
    masks.iter().map(|m| m.coverage(p)).product()
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn square() -> Vec<Vec2> {
        vec![
            Vec2::new(0.25, 0.25),
            Vec2::new(0.75, 0.25),
            Vec2::new(0.75, 0.75),
            Vec2::new(0.25, 0.75),
        ]
    }

    #[test]
    fn hard_hole_and_window() {
        let hole = Mask::hole(square());
        assert!(hole.is_valid());
        assert_eq!(hole.coverage(Vec2::new(0.5, 0.5)), 0.0);
        assert_eq!(hole.coverage(Vec2::new(0.1, 0.1)), 1.0);
        let window = Mask::window(square());
        assert_eq!(window.coverage(Vec2::new(0.5, 0.5)), 1.0);
        assert_eq!(window.coverage(Vec2::new(0.1, 0.1)), 0.0);
    }

    #[test]
    fn contains_handles_concave_polygons() {
        // A "C" shape.
        let c = Mask::hole(vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 0.3),
            Vec2::new(0.3, 0.3),
            Vec2::new(0.3, 0.7),
            Vec2::new(1.0, 0.7),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ]);
        assert!(c.contains(Vec2::new(0.1, 0.5)));
        assert!(!c.contains(Vec2::new(0.6, 0.5)));
        assert!(c.contains(Vec2::new(0.6, 0.1)));
    }

    #[test]
    fn feather_ramps_across_the_edge() {
        let mut m = Mask::hole(square());
        m.feather = 0.1;
        // 0.05 inside the left edge: signed = +0.05 -> inside_amount 1.0 -> coverage 0
        assert!((m.coverage(Vec2::new(0.30, 0.5)) - 0.0).abs() < 1e-9);
        // exactly on the edge: 0.5
        assert!((m.coverage(Vec2::new(0.25, 0.5)) - 0.5).abs() < 1e-9);
        // 0.05 outside: 1.0
        assert!((m.coverage(Vec2::new(0.20, 0.5)) - 1.0).abs() < 1e-9);
        // 0.025 outside: 0.75
        assert!((m.coverage(Vec2::new(0.225, 0.5)) - 0.75).abs() < 1e-9);
    }

    #[test]
    fn invalid_masks_are_transparent() {
        let two = Mask::hole(vec![Vec2::ZERO, Vec2::new(1.0, 1.0)]);
        assert!(!two.is_valid());
        assert_eq!(two.coverage(Vec2::new(0.5, 0.5)), 1.0);
        assert!(!two.contains(Vec2::new(0.5, 0.5)));
        let empty = Mask::hole(vec![]);
        assert_eq!(empty.edge_distance(Vec2::ZERO), f64::INFINITY);
        let mut nan = Mask::hole(square());
        nan.feather = f64::NAN;
        assert!(!nan.is_valid());
    }

    #[test]
    fn segment_distance_cases() {
        let a = Vec2::new(0.0, 0.0);
        let b = Vec2::new(1.0, 0.0);
        assert!((segment_distance(Vec2::new(0.5, 1.0), a, b) - 1.0).abs() < 1e-12);
        assert!((segment_distance(Vec2::new(2.0, 0.0), a, b) - 1.0).abs() < 1e-12);
        assert!((segment_distance(Vec2::new(-1.0, 0.0), a, b) - 1.0).abs() < 1e-12);
        assert!((segment_distance(Vec2::new(3.0, 4.0), a, a) - 5.0).abs() < 1e-12);
        let m = Mask::hole(square());
        assert!((m.edge_distance(Vec2::new(0.5, 0.5)) - 0.25).abs() < 1e-12);
    }

    #[test]
    fn combined_masks_multiply() {
        let hole = Mask::hole(square());
        let window = Mask::window(vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ]);
        let masks = [hole, window];
        assert_eq!(combined_coverage(&masks, Vec2::new(0.5, 0.5)), 0.0);
        assert_eq!(combined_coverage(&masks, Vec2::new(0.1, 0.1)), 1.0);
        assert_eq!(combined_coverage(&masks, Vec2::new(1.5, 0.1)), 0.0);
        assert_eq!(combined_coverage(&[], Vec2::ZERO), 1.0);
    }

    #[test]
    fn serde_defaults() {
        let m: Mask =
            toml::from_str("points = [{x = 0.0, y = 0.0}, {x = 1.0, y = 0.0}, {x = 1.0, y = 1.0}]")
                .unwrap();
        assert!(m.subtract);
        assert_eq!(m.feather, 0.0);
    }
}
