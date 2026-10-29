//! Built-in calibration patterns.
//!
//! Patterns are generated procedurally so the binary ships no image assets and the goldens
//! are reproducible from code alone.

use crate::Frame;
use lumen_core::project::TestPattern;

/// Generates a pattern at the given size.
pub fn generate(pattern: TestPattern, width: u32, height: u32) -> Frame {
    match pattern {
        TestPattern::Grid => grid(width, height),
        TestPattern::Bars => bars(width, height),
        TestPattern::Checker => checker(width, height),
        TestPattern::Ramp => ramp(width, height),
    }
}

/// Alignment grid: 8×8 cells, thick white border, cyan centre crosshair and a circle to reveal
/// aspect errors.
fn grid(w: u32, h: u32) -> Frame {
    let cells = 8.0;
    let line = (f64::from(w.min(h)) / 256.0).max(1.0);
    let border = line * 3.0;
    let cx = f64::from(w) / 2.0;
    let cy = f64::from(h) / 2.0;
    let radius = f64::from(w.min(h)) * 0.4;
    Frame::from_fn(w, h, |x, y| {
        let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
        if fx < border || fy < border || fx > f64::from(w) - border || fy > f64::from(h) - border {
            return [255, 255, 255, 255];
        }
        let on_cross = (fx - cx).abs() < line * 1.5 || (fy - cy).abs() < line * 1.5;
        if on_cross {
            return [0, 220, 255, 255];
        }
        let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
        if (d - radius).abs() < line {
            return [255, 200, 0, 255];
        }
        let gx = (fx / f64::from(w) * cells).fract() * f64::from(w) / cells;
        let gy = (fy / f64::from(h) * cells).fract() * f64::from(h) / cells;
        if gx < line || gy < line {
            return [200, 200, 200, 255];
        }
        [20, 20, 20, 255]
    })
}

/// Seven colour bars.
fn bars(w: u32, h: u32) -> Frame {
    const COLORS: [[u8; 4]; 7] = [
        [255, 255, 255, 255],
        [255, 255, 0, 255],
        [0, 255, 255, 255],
        [0, 255, 0, 255],
        [255, 0, 255, 255],
        [255, 0, 0, 255],
        [0, 0, 255, 255],
    ];
    Frame::from_fn(w, h, |x, _| {
        let i = ((u64::from(x) * 7) / u64::from(w.max(1))) as usize;
        COLORS[i.min(6)]
    })
}

/// 16×16 black and white checkerboard.
fn checker(w: u32, h: u32) -> Frame {
    Frame::from_fn(w, h, |x, y| {
        let cx = (u64::from(x) * 16) / u64::from(w.max(1));
        let cy = (u64::from(y) * 16) / u64::from(h.max(1));
        if (cx + cy) % 2 == 0 {
            [240, 240, 240, 255]
        } else {
            [16, 16, 16, 255]
        }
    })
}

/// Horizontal grey ramp, 0 at the left edge to 255 at the right.
fn ramp(w: u32, h: u32) -> Frame {
    Frame::from_fn(w, h, |x, _| {
        let v = if w <= 1 {
            0
        } else {
            ((u64::from(x) * 255) / u64::from(w - 1)) as u8
        };
        [v, v, v, 255]
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_is_consistent_and_opaque() {
        for p in [
            TestPattern::Grid,
            TestPattern::Bars,
            TestPattern::Checker,
            TestPattern::Ramp,
        ] {
            let f = generate(p, 128, 72);
            assert!(f.is_consistent(), "{p:?}");
            assert!(f.rgba.chunks_exact(4).all(|px| px[3] == 255), "{p:?}");
            assert_eq!((f.width, f.height), (128, 72));
        }
    }

    #[test]
    fn grid_has_border_cross_and_circle() {
        let f = generate(TestPattern::Grid, 256, 256);
        assert_eq!(f.pixel(0, 100), [255, 255, 255, 255], "border");
        assert_eq!(f.pixel(128, 40), [0, 220, 255, 255], "vertical crosshair");
        assert_eq!(f.pixel(40, 128), [0, 220, 255, 255], "horizontal crosshair");
        // Circle of radius 102.4 crosses the x axis at about x = 128 + 102 on the row y=128,
        // but that row is the crosshair; test on the diagonal instead.
        let d = (102.4f64 / 2f64.sqrt()).round() as i64;
        assert_eq!(f.pixel(128 + d, 128 + d), [255, 200, 0, 255], "circle");
        assert_eq!(f.pixel(20, 20), [20, 20, 20, 255], "cell interior");
        assert_eq!(f.pixel(32, 20), [200, 200, 200, 255], "grid line");
        assert!(
            generate(TestPattern::Grid, 2, 2).is_consistent(),
            "tiny sizes do not panic"
        );
    }

    #[test]
    fn bars_cover_all_seven_colours_in_order() {
        let f = generate(TestPattern::Bars, 70, 1);
        let firsts: Vec<[u8; 4]> = (0..7).map(|i| f.pixel(i * 10, 0)).collect();
        assert_eq!(firsts[0], [255, 255, 255, 255]);
        assert_eq!(firsts[6], [0, 0, 255, 255]);
        assert_eq!(
            firsts
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            7
        );
    }

    #[test]
    fn checker_alternates_and_ramp_is_monotonic() {
        let c = generate(TestPattern::Checker, 32, 32);
        assert_ne!(c.pixel(0, 0), c.pixel(2, 0));
        assert_eq!(c.pixel(0, 0), c.pixel(2, 2));
        let r = generate(TestPattern::Ramp, 256, 1);
        assert_eq!(r.pixel(0, 0)[0], 0);
        assert_eq!(r.pixel(255, 0)[0], 255);
        assert!((1..256).all(|x| r.pixel(x, 0)[0] >= r.pixel(x - 1, 0)[0]));
        assert_eq!(generate(TestPattern::Ramp, 1, 1).pixel(0, 0)[0], 0);
    }
}
