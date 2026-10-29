//! Soft-edge blending for overlapping projectors.
//!
//! Where two projectors overlap, each must fade out across the overlap so the sum of their
//! light equals one projector's worth. Two details separate a seamless blend from a visible
//! band, and both are handled here:
//!
//! 1. The fade must be computed in **linear light** and then encoded with the projector's
//!    gamma, because projectors add light linearly but accept gamma-encoded signals.
//! 2. The two sides of an overlap must use **complementary curves** so that
//!    `fade_a(t) + fade_b(1 - t) == 1` for every `t`. [`BlendCurve`] guarantees this.

use serde::{Deserialize, Serialize};

/// Shape of the fade across a blend region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendCurve {
    /// Straight ramp. Rarely right, but useful for calibration.
    Linear,
    /// Smoothstep (`3t² − 2t³`): the standard soft edge.
    #[default]
    Smooth,
    /// Half-cosine ramp; slightly steeper centre than smoothstep.
    Cosine,
}

impl BlendCurve {
    /// Evaluates the curve at `t ∈ [0,1]`; `0` is fully faded, `1` is full brightness.
    ///
    /// Every curve is point-symmetric about `(0.5, 0.5)`, so `f(t) + f(1 − t) = 1`.
    pub fn eval(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::Smooth => t * t * (3.0 - 2.0 * t),
            Self::Cosine => 0.5 - 0.5 * (std::f64::consts::PI * t).cos(),
        }
    }
}

/// One edge of an output's blend configuration.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BlendEdge {
    /// Width of the fade as a fraction of the output's size along that axis. `0` disables.
    pub width: f64,
    /// Fade shape.
    pub curve: BlendCurve,
}

impl Default for BlendEdge {
    fn default() -> Self {
        Self {
            width: 0.0,
            curve: BlendCurve::Smooth,
        }
    }
}

/// Blend configuration for all four edges of an output.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct EdgeBlend {
    /// Left edge.
    pub left: BlendEdge,
    /// Right edge.
    pub right: BlendEdge,
    /// Top edge.
    pub top: BlendEdge,
    /// Bottom edge.
    pub bottom: BlendEdge,
    /// Projector gamma used to encode the linear-light fade. Typical projectors are 2.2.
    pub gamma: f64,
}

impl EdgeBlend {
    /// No blending, gamma 2.2.
    pub fn none() -> Self {
        Self {
            gamma: 2.2,
            ..Default::default()
        }
    }

    /// Returns the configuration with its gamma clamped to a sane range.
    #[must_use]
    pub fn sanitised(mut self) -> Self {
        if !self.gamma.is_finite() || self.gamma <= 0.0 {
            self.gamma = 2.2;
        }
        self.gamma = self.gamma.clamp(0.5, 4.0);
        for e in [
            &mut self.left,
            &mut self.right,
            &mut self.top,
            &mut self.bottom,
        ] {
            if !e.width.is_finite() {
                e.width = 0.0;
            }
            e.width = e.width.clamp(0.0, 1.0);
        }
        self
    }

    /// True when every edge has zero width.
    pub fn is_identity(&self) -> bool {
        [self.left, self.right, self.top, self.bottom]
            .iter()
            .all(|e| e.width <= 0.0)
    }

    /// Linear-light attenuation at normalised position `(x, y)` in `[0,1]²`.
    ///
    /// Each edge contributes a factor in `[0,1]`; they multiply so corners blend correctly.
    pub fn linear_factor(&self, x: f64, y: f64) -> f64 {
        let s = self.sanitised();
        let edge = |e: BlendEdge, d: f64| {
            if e.width <= 0.0 {
                1.0
            } else {
                e.curve.eval(d / e.width)
            }
        };
        edge(s.left, x) * edge(s.right, 1.0 - x) * edge(s.top, y) * edge(s.bottom, 1.0 - y)
    }

    /// Gamma-encoded multiplier to apply to the output's sRGB-ish signal.
    ///
    /// Equals `linear_factor^(1/gamma)`.
    pub fn encoded_factor(&self, x: f64, y: f64) -> f64 {
        let s = self.sanitised();
        self.linear_factor(x, y).powf(1.0 / s.gamma)
    }

    /// Samples the encoded factor into a `width × height` lookup table, row-major, for GPU
    /// upload. Samples are taken at pixel centres.
    pub fn lut(&self, width: u32, height: u32) -> Vec<f32> {
        let (w, h) = (width.max(1), height.max(1));
        let mut out = Vec::with_capacity((w * h) as usize);
        for j in 0..h {
            let y = (f64::from(j) + 0.5) / f64::from(h);
            for i in 0..w {
                let x = (f64::from(i) + 0.5) / f64::from(w);
                out.push(self.encoded_factor(x, y) as f32);
            }
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn curves_are_complementary() {
        for curve in [BlendCurve::Linear, BlendCurve::Smooth, BlendCurve::Cosine] {
            for i in 0..=100 {
                let t = f64::from(i) / 100.0;
                let sum = curve.eval(t) + curve.eval(1.0 - t);
                assert!((sum - 1.0).abs() < 1e-12, "{curve:?} at {t}: {sum}");
            }
            assert_eq!(curve.eval(0.0), 0.0);
            assert!((curve.eval(1.0) - 1.0).abs() < 1e-12);
            assert_eq!(curve.eval(-1.0), 0.0, "clamped below");
            assert!((curve.eval(2.0) - 1.0).abs() < 1e-12, "clamped above");
        }
        assert_eq!(BlendCurve::default(), BlendCurve::Smooth);
    }

    #[test]
    fn curves_are_monotonic() {
        for curve in [BlendCurve::Linear, BlendCurve::Smooth, BlendCurve::Cosine] {
            let mut prev = -1.0;
            for i in 0..=200 {
                let v = curve.eval(f64::from(i) / 200.0);
                assert!(v >= prev, "{curve:?} decreased at step {i}");
                prev = v;
            }
        }
    }

    #[test]
    fn no_blend_is_identity() {
        let b = EdgeBlend::none();
        assert!(b.is_identity());
        assert_eq!(b.linear_factor(0.0, 0.0), 1.0);
        assert_eq!(b.encoded_factor(0.5, 0.5), 1.0);
        assert!(b.lut(4, 2).iter().all(|&v| v == 1.0));
        assert_eq!(b.lut(0, 0).len(), 1);
        assert_eq!(EdgeBlend::default().gamma, 0.0);
        assert_eq!(EdgeBlend::default().sanitised().gamma, 2.2);
    }

    #[test]
    fn overlap_sums_to_one_in_linear_light() {
        let width = 0.25;
        let left_proj = EdgeBlend {
            right: BlendEdge {
                width,
                curve: BlendCurve::Smooth,
            },
            gamma: 2.2,
            ..Default::default()
        };
        let right_proj = EdgeBlend {
            left: BlendEdge {
                width,
                curve: BlendCurve::Smooth,
            },
            gamma: 2.2,
            ..Default::default()
        };
        // The right projector's x=0 coincides with the left projector's x=1-width.
        for i in 0..=50 {
            let t = f64::from(i) / 50.0; // position across the overlap
            let a = left_proj.linear_factor(1.0 - width + t * width, 0.5);
            let b = right_proj.linear_factor(t * width, 0.5);
            assert!((a + b - 1.0).abs() < 1e-9, "t={t}: {a} + {b}");
            // After gamma encoding the sum must NOT be one; that is the whole point.
            let ea = left_proj.encoded_factor(1.0 - width + t * width, 0.5);
            let eb = right_proj.encoded_factor(t * width, 0.5);
            assert!(ea.powf(2.2) + eb.powf(2.2) - 1.0 < 1e-9);
        }
    }

    #[test]
    fn corners_multiply() {
        let b = EdgeBlend {
            left: BlendEdge {
                width: 0.5,
                curve: BlendCurve::Linear,
            },
            top: BlendEdge {
                width: 0.5,
                curve: BlendCurve::Linear,
            },
            gamma: 1.0,
            ..Default::default()
        };
        assert!((b.linear_factor(0.25, 0.25) - 0.25).abs() < 1e-12);
        assert!((b.encoded_factor(0.25, 0.25) - 0.25).abs() < 1e-12);
        assert!((b.linear_factor(0.25, 0.9) - 0.5).abs() < 1e-12);
        let lut = b.lut(2, 2);
        assert_eq!(lut.len(), 4);
        assert!(lut[3] > lut[0]);
    }

    #[test]
    fn sanitise_fixes_bad_values() {
        let b = EdgeBlend {
            left: BlendEdge {
                width: f64::NAN,
                curve: BlendCurve::Linear,
            },
            right: BlendEdge {
                width: 7.0,
                curve: BlendCurve::Linear,
            },
            gamma: -3.0,
            ..Default::default()
        }
        .sanitised();
        assert_eq!(b.left.width, 0.0);
        assert_eq!(b.right.width, 1.0);
        assert_eq!(b.gamma, 2.2);
        assert_eq!(
            EdgeBlend {
                gamma: 99.0,
                ..Default::default()
            }
            .sanitised()
            .gamma,
            4.0
        );
        assert_eq!(
            EdgeBlend {
                gamma: f64::INFINITY,
                ..Default::default()
            }
            .sanitised()
            .gamma,
            2.2
        );
    }

    proptest! {
        #[test]
        fn factor_stays_in_unit_range(
            x in 0.0f64..1.0, y in 0.0f64..1.0,
            w in 0.0f64..1.0, g in 0.5f64..4.0,
        ) {
            let b = EdgeBlend {
                left: BlendEdge { width: w, curve: BlendCurve::Cosine },
                bottom: BlendEdge { width: w, curve: BlendCurve::Smooth },
                gamma: g,
                ..Default::default()
            };
            let f = b.encoded_factor(x, y);
            prop_assert!((0.0..=1.0).contains(&f));
            if g >= 1.0 {
                prop_assert!(b.linear_factor(x, y) <= f + 1e-12, "gamma >= 1 never darkens");
            }
        }
    }
}
