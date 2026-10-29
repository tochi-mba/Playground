//! Image comparison metrics for golden and cross-renderer tests.

use image::RgbaImage;

/// Summary of the per-channel absolute differences between two images.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Diff {
    /// Largest single-channel difference, 0–255.
    pub max: u8,
    /// Mean difference across all channels.
    pub mean: f64,
    /// Fraction of pixels where any channel differs by more than `tolerance`.
    pub fraction_over: f64,
    /// Tolerance used for `fraction_over`.
    pub tolerance: u8,
}

/// Compares two same-sized images. Returns `None` when sizes differ.
pub fn diff(a: &RgbaImage, b: &RgbaImage, tolerance: u8) -> Option<Diff> {
    if a.dimensions() != b.dimensions() {
        return None;
    }
    let mut max = 0u8;
    let mut sum = 0u64;
    let mut over = 0u64;
    for (pa, pb) in a.pixels().zip(b.pixels()) {
        let mut pixel_max = 0u8;
        for c in 0..3 {
            let d = pa.0[c].abs_diff(pb.0[c]);
            sum += u64::from(d);
            pixel_max = pixel_max.max(d);
        }
        max = max.max(pixel_max);
        if pixel_max > tolerance {
            over += 1;
        }
    }
    let pixels = u64::from(a.width()) * u64::from(a.height());
    Some(Diff {
        max,
        mean: sum as f64 / (pixels * 3).max(1) as f64,
        fraction_over: over as f64 / pixels.max(1) as f64,
        tolerance,
    })
}

/// Acceptance thresholds for a comparison.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    /// Per-channel tolerance for counting a pixel as "off".
    pub tolerance: u8,
    /// Maximum allowed fraction of off pixels.
    pub max_fraction_over: f64,
    /// Maximum allowed mean difference.
    pub max_mean: f64,
}

impl Thresholds {
    /// Software renderer against its own golden: only libm rounding may differ.
    pub const EXACT: Self = Self {
        tolerance: 2,
        max_fraction_over: 0.0,
        max_mean: 0.05,
    };
    /// GPU against the software oracle: texture filtering and 8-bit intermediates differ.
    pub const GPU: Self = Self {
        tolerance: 24,
        max_fraction_over: 0.01,
        max_mean: 1.0,
    };

    /// Checks a diff against these thresholds.
    pub fn accepts(&self, d: &Diff) -> bool {
        d.fraction_over <= self.max_fraction_over && d.mean <= self.max_mean
    }
}

/// Writes an image showing where two images differ (white = different), for debugging.
pub fn diff_image(a: &RgbaImage, b: &RgbaImage, tolerance: u8) -> Option<RgbaImage> {
    if a.dimensions() != b.dimensions() {
        return None;
    }
    let mut out = RgbaImage::new(a.width(), a.height());
    for ((pa, pb), po) in a.pixels().zip(b.pixels()).zip(out.pixels_mut()) {
        let d = (0..3).map(|c| pa.0[c].abs_diff(pb.0[c])).max().unwrap_or(0);
        let v = if d > tolerance {
            255
        } else {
            d.saturating_mul(4)
        };
        *po = image::Rgba([v, v, v, 255]);
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_have_zero_diff() {
        let a = RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]));
        let d = diff(&a, &a, 0).unwrap();
        assert_eq!(
            d,
            Diff {
                max: 0,
                mean: 0.0,
                fraction_over: 0.0,
                tolerance: 0
            }
        );
        assert!(Thresholds::EXACT.accepts(&d));
        assert!(Thresholds::GPU.accepts(&d));
    }

    #[test]
    fn differences_are_measured_and_alpha_ignored() {
        let a = RgbaImage::from_pixel(2, 1, image::Rgba([0, 0, 0, 255]));
        let mut b = a.clone();
        b.put_pixel(1, 0, image::Rgba([30, 0, 0, 0]));
        let d = diff(&a, &b, 10).unwrap();
        assert_eq!(d.max, 30);
        assert_eq!(d.fraction_over, 0.5);
        assert!((d.mean - 5.0).abs() < 1e-12);
        assert!(!Thresholds::EXACT.accepts(&d));
        let t = Thresholds {
            tolerance: 10,
            max_fraction_over: 0.5,
            max_mean: 5.0,
        };
        assert!(t.accepts(&d));
        let img = diff_image(&a, &b, 10).unwrap();
        assert_eq!(img.get_pixel(1, 0).0, [255, 255, 255, 255]);
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0, 255]);
        let small = diff_image(
            &a,
            &RgbaImage::from_pixel(2, 1, image::Rgba([3, 0, 0, 255])),
            10,
        )
        .unwrap();
        assert_eq!(small.get_pixel(0, 0).0[0], 12);
    }

    #[test]
    fn size_mismatch_is_none() {
        let a = RgbaImage::new(2, 2);
        let b = RgbaImage::new(3, 2);
        assert!(diff(&a, &b, 0).is_none());
        assert!(diff_image(&a, &b, 0).is_none());
        assert_eq!(
            diff(&RgbaImage::new(0, 0), &RgbaImage::new(0, 0), 0)
                .unwrap()
                .mean,
            0.0
        );
    }
}
