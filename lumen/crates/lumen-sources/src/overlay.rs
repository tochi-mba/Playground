//! Calibration overlays drawn over an output while aligning projectors.
//!
//! Unlike test patterns these have transparent areas, so the content stays visible under a
//! grid or crosshair. They are generated at the output's resolution and cached by the scene.

use crate::Frame;
use lumen_core::project::Overlay;

const WHITE: [u8; 4] = [255, 255, 255, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];
const CYAN: [u8; 4] = [0, 220, 255, 255];
const MAGENTA: [u8; 4] = [255, 0, 200, 255];

/// Generates an overlay at the given size.
pub fn generate(kind: Overlay, width: u32, height: u32) -> Frame {
    match kind {
        Overlay::Grid => grid(width, height),
        Overlay::Crosshair => crosshair(width, height),
        Overlay::Bars => {
            crate::pattern::generate(lumen_core::project::TestPattern::Bars, width, height)
        }
        Overlay::GreySteps => grey_steps(width, height),
        Overlay::BlendCheck => Frame::solid(width.max(1), height.max(1), WHITE),
    }
}

fn line_width(w: u32, h: u32) -> f64 {
    (f64::from(w.min(h)) / 360.0).max(1.0)
}

/// 16 × 9 cells (so the lines land on the same wall positions for every 16:9 output),
/// a thicker centre cross and a white border.
fn grid(w: u32, h: u32) -> Frame {
    let line = line_width(w, h);
    let (fw, fh) = (f64::from(w), f64::from(h));
    Frame::from_fn(w, h, |x, y| {
        let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
        let border = line * 2.0;
        if fx < border || fy < border || fx > fw - border || fy > fh - border {
            return WHITE;
        }
        if (fx - fw / 2.0).abs() < line * 1.5 || (fy - fh / 2.0).abs() < line * 1.5 {
            return CYAN;
        }
        let gx = (fx / fw * 16.0).fract() * fw / 16.0;
        let gy = (fy / fh * 9.0).fract() * fh / 9.0;
        if gx < line || gy < line {
            return [255, 255, 255, 160];
        }
        CLEAR
    })
}

/// Centre cross, both diagonals and three concentric circles.
fn crosshair(w: u32, h: u32) -> Frame {
    let line = line_width(w, h);
    let (fw, fh) = (f64::from(w), f64::from(h));
    let (cx, cy) = (fw / 2.0, fh / 2.0);
    let r = f64::from(w.min(h)) / 2.0;
    Frame::from_fn(w, h, |x, y| {
        let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
        let (dx, dy) = (fx - cx, fy - cy);
        if dx.abs() < line * 1.5 || dy.abs() < line * 1.5 {
            return MAGENTA;
        }
        // Diagonals through the corners.
        let along = (dx * fh - dy * fw).abs() / (fw * fw + fh * fh).sqrt();
        let anti = (dx * fh + dy * fw).abs() / (fw * fw + fh * fh).sqrt();
        if along < line || anti < line {
            return [255, 255, 255, 200];
        }
        let d = (dx * dx + dy * dy).sqrt();
        for k in [0.25, 0.5, 0.9] {
            if (d - r * k).abs() < line {
                return CYAN;
            }
        }
        CLEAR
    })
}

/// Eleven vertical grey steps from black to white, with the step value written as a bar
/// height along the bottom so a camera or an eye can read it across a seam.
fn grey_steps(w: u32, h: u32) -> Frame {
    let steps = 11.0;
    let (fw, fh) = (f64::from(w), f64::from(h));
    Frame::from_fn(w, h, |x, y| {
        let fx = f64::from(x) + 0.5;
        let fy = f64::from(y) + 0.5;
        let i = (fx / fw * steps).floor().min(steps - 1.0);
        let v = (i / (steps - 1.0) * 255.0).round() as u8;
        // Bottom 10 %: a bar whose height encodes the step, on mid grey.
        if fy > fh * 0.9 {
            let bar_top = fh * (1.0 - 0.1 * (i + 1.0) / steps);
            return if fy > bar_top {
                WHITE
            } else {
                [128, 128, 128, 255]
            };
        }
        [v, v, v, 255]
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn every_overlay_is_consistent_and_has_the_right_transparency() {
        for kind in Overlay::ALL {
            let f = generate(kind, 320, 180);
            assert!(f.is_consistent(), "{kind:?}");
            assert_eq!((f.width, f.height), (320, 180));
            let opaque = f.rgba.chunks_exact(4).filter(|p| p[3] == 255).count();
            let total = (f.width * f.height) as usize;
            match kind {
                Overlay::Grid | Overlay::Crosshair => {
                    assert!(opaque < total / 2, "{kind:?} lets content through");
                    assert!(opaque > 0);
                }
                Overlay::Bars | Overlay::GreySteps | Overlay::BlendCheck => {
                    assert_eq!(opaque, total, "{kind:?} is opaque");
                }
            }
            assert!(generate(kind, 1, 1).is_consistent(), "{kind:?} at 1x1");
            assert!(!kind.label().is_empty());
        }
    }

    #[test]
    fn grid_crosshair_and_steps_have_their_features() {
        let g = generate(Overlay::Grid, 320, 180);
        assert_eq!(g.pixel(0, 90), WHITE, "border");
        assert_eq!(g.pixel(160, 40), CYAN, "centre cross");
        assert_eq!(g.pixel(10, 10), CLEAR, "cell interior");
        assert_eq!(g.pixel(20, 10), [255, 255, 255, 160], "grid line at 320/16");
        let c = generate(Overlay::Crosshair, 320, 180);
        assert_eq!(c.pixel(160, 20), MAGENTA);
        assert_eq!(c.pixel(20, 20), CLEAR);
        assert_eq!(c.pixel(80, 45), [255, 255, 255, 200], "diagonal");
        let s = generate(Overlay::GreySteps, 330, 100);
        assert_eq!(s.pixel(5, 10), [0, 0, 0, 255]);
        assert_eq!(s.pixel(325, 10), WHITE);
        assert_eq!(s.pixel(5, 95), [128, 128, 128, 255], "short bar for step 0");
        assert_eq!(s.pixel(325, 91), WHITE, "tall bar for step 10");
        let b = generate(Overlay::BlendCheck, 4, 4);
        assert!(b.rgba.iter().all(|&v| v == 255));
    }
}
