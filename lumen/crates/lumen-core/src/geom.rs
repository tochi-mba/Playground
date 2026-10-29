//! Small, dependency-free linear algebra for 2-D projective geometry.

use serde::{Deserialize, Serialize};
use std::ops::{Add, Mul, Sub};

/// A 2-D point or vector in normalised output space, where `(0,0)` is the top-left corner of
/// an output and `(1,1)` the bottom-right.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec2 {
    /// Horizontal component.
    pub x: f64,
    /// Vertical component.
    pub y: f64,
}

impl Vec2 {
    /// Creates a vector.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Origin.
    pub const ZERO: Self = Self::new(0.0, 0.0);

    /// Scales both components.
    #[must_use]
    pub fn scale(self, s: f64) -> Self {
        Self::new(self.x * s, self.y * s)
    }

    /// Dot product.
    pub fn dot(self, o: Self) -> f64 {
        self.x * o.x + self.y * o.y
    }

    /// 2-D cross product (z component of the 3-D cross product).
    pub fn cross(self, o: Self) -> f64 {
        self.x * o.y - self.y * o.x
    }

    /// Euclidean length.
    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    /// Distance to another point.
    pub fn distance(self, o: Self) -> f64 {
        (self - o).length()
    }

    /// Linear interpolation; `t = 0` gives `self`, `t = 1` gives `o`.
    #[must_use]
    pub fn lerp(self, o: Self, t: f64) -> Self {
        self + (o - self).scale(t)
    }

    /// True when both components are finite numbers.
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Add for Vec2 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Vec2 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y)
    }
}

impl Mul<f64> for Vec2 {
    type Output = Self;
    fn mul(self, s: f64) -> Self {
        self.scale(s)
    }
}

/// Row-major 3×3 matrix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat3(pub [f64; 9]);

impl Mat3 {
    /// Identity matrix.
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);

    /// Element at `row`, `col` (both zero-based).
    pub fn at(&self, row: usize, col: usize) -> f64 {
        self.0[row * 3 + col]
    }

    /// Matrix product `self * o`.
    #[must_use]
    pub fn mul(&self, o: &Self) -> Self {
        let mut r = [0.0; 9];
        for i in 0..3 {
            for j in 0..3 {
                r[i * 3 + j] = (0..3).map(|k| self.at(i, k) * o.at(k, j)).sum();
            }
        }
        Self(r)
    }

    /// Determinant.
    pub fn det(&self) -> f64 {
        let m = &self.0;
        m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
            + m[2] * (m[3] * m[7] - m[4] * m[6])
    }

    /// Inverse, or `None` when the matrix is singular.
    pub fn inverse(&self) -> Option<Self> {
        let m = &self.0;
        let det = self.det();
        if !det.is_finite() || det.abs() < 1e-14 {
            return None;
        }
        let inv = 1.0 / det;
        Some(Self([
            (m[4] * m[8] - m[5] * m[7]) * inv,
            (m[2] * m[7] - m[1] * m[8]) * inv,
            (m[1] * m[5] - m[2] * m[4]) * inv,
            (m[5] * m[6] - m[3] * m[8]) * inv,
            (m[0] * m[8] - m[2] * m[6]) * inv,
            (m[2] * m[3] - m[0] * m[5]) * inv,
            (m[3] * m[7] - m[4] * m[6]) * inv,
            (m[1] * m[6] - m[0] * m[7]) * inv,
            (m[0] * m[4] - m[1] * m[3]) * inv,
        ]))
    }

    /// Applies the matrix to a homogeneous point and divides by `w`.
    ///
    /// Returns `None` when the point maps to infinity.
    pub fn apply(&self, p: Vec2) -> Option<Vec2> {
        let m = &self.0;
        let x = m[0] * p.x + m[1] * p.y + m[2];
        let y = m[3] * p.x + m[4] * p.y + m[5];
        let w = m[6] * p.x + m[7] * p.y + m[8];
        if w.abs() < 1e-12 || !w.is_finite() {
            return None;
        }
        Some(Vec2::new(x / w, y / w))
    }

    /// Applies the matrix and returns the un-divided homogeneous triple `(x, y, w)`.
    ///
    /// GPU warps use this so the rasteriser can interpolate `x/w`, `y/w`, `1/w`
    /// perspective-correctly instead of linearly.
    pub fn apply_homogeneous(&self, p: Vec2) -> [f64; 3] {
        let m = &self.0;
        [
            m[0] * p.x + m[1] * p.y + m[2],
            m[3] * p.x + m[4] * p.y + m[5],
            m[6] * p.x + m[7] * p.y + m[8],
        ]
    }

    /// Scales every element so that the bottom-right entry is `1`.
    ///
    /// Leaves the matrix untouched when that entry is zero.
    #[must_use]
    pub fn normalised(&self) -> Self {
        let w = self.0[8];
        if w.abs() < 1e-14 {
            return *self;
        }
        Self(self.0.map(|v| v / w))
    }
}

/// Errors from fitting a homography.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HomographyError {
    /// Three or more of the four points are collinear, or two coincide.
    #[error("degenerate point configuration: the four points do not span a quadrilateral")]
    Degenerate,
    /// A coordinate was NaN or infinite.
    #[error("non-finite coordinate")]
    NonFinite,
}

/// A projective transform mapping the unit square to an arbitrary convex quadrilateral.
///
/// This is the mathematical heart of corner-pin mapping: the four corners you drag in the
/// editor define the only perspective transform that maps the source rectangle onto them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Homography {
    forward: Mat3,
    inverse: Mat3,
}

impl Homography {
    /// Fits the homography sending the unit square corners
    /// `(0,0), (1,0), (1,1), (0,1)` to `dst` in the same order
    /// (top-left, top-right, bottom-right, bottom-left).
    pub fn unit_square_to(dst: [Vec2; 4]) -> Result<Self, HomographyError> {
        let src = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        Self::from_points(src, dst)
    }

    /// Fits the homography mapping each `src[i]` onto `dst[i]`.
    ///
    /// Uses the closed-form "square to quad" construction composed with its inverse, which is
    /// exact and far better conditioned than a general 8×8 solve for this use.
    pub fn from_points(src: [Vec2; 4], dst: [Vec2; 4]) -> Result<Self, HomographyError> {
        if !src.iter().chain(dst.iter()).all(|p| p.is_finite()) {
            return Err(HomographyError::NonFinite);
        }
        let s = square_to_quad(src)?;
        let d = square_to_quad(dst)?;
        let s_inv = s.inverse().ok_or(HomographyError::Degenerate)?;
        let forward = d.mul(&s_inv).normalised();
        let inverse = forward
            .inverse()
            .ok_or(HomographyError::Degenerate)?
            .normalised();
        Ok(Self { forward, inverse })
    }

    /// Identity transform.
    pub fn identity() -> Self {
        Self {
            forward: Mat3::IDENTITY,
            inverse: Mat3::IDENTITY,
        }
    }

    /// Forward matrix (source → destination).
    pub fn matrix(&self) -> &Mat3 {
        &self.forward
    }

    /// Inverse matrix (destination → source).
    pub fn inverse_matrix(&self) -> &Mat3 {
        &self.inverse
    }

    /// Maps a source point to destination space.
    pub fn map(&self, p: Vec2) -> Option<Vec2> {
        self.forward.apply(p)
    }

    /// Maps a destination point back to source space.
    pub fn unmap(&self, p: Vec2) -> Option<Vec2> {
        self.inverse.apply(p)
    }
}

/// Matrix mapping the unit square to `q` (Heckbert, "Fundamentals of Texture Mapping").
fn square_to_quad(q: [Vec2; 4]) -> Result<Mat3, HomographyError> {
    let [p0, p1, p2, p3] = q;
    // Reject coincident points and collinear triples up front: they give a rank-deficient matrix
    // that still passes the determinant check in some near-degenerate cases.
    for i in 0..4 {
        for j in (i + 1)..4 {
            if q[i].distance(q[j]) < 1e-9 {
                return Err(HomographyError::Degenerate);
            }
        }
    }
    let area = |a: Vec2, b: Vec2, c: Vec2| (b - a).cross(c - a);
    let triples = [(p0, p1, p2), (p1, p2, p3), (p2, p3, p0), (p3, p0, p1)];
    if triples.iter().any(|&(a, b, c)| area(a, b, c).abs() < 1e-12) {
        return Err(HomographyError::Degenerate);
    }

    let dx1 = p1.x - p2.x;
    let dx2 = p3.x - p2.x;
    let dx3 = p0.x - p1.x + p2.x - p3.x;
    let dy1 = p1.y - p2.y;
    let dy2 = p3.y - p2.y;
    let dy3 = p0.y - p1.y + p2.y - p3.y;

    let (g, h) = if dx3.abs() < 1e-12 && dy3.abs() < 1e-12 {
        (0.0, 0.0)
    } else {
        let den = dx1 * dy2 - dx2 * dy1;
        if den.abs() < 1e-14 {
            return Err(HomographyError::Degenerate);
        }
        ((dx3 * dy2 - dx2 * dy3) / den, (dx1 * dy3 - dx3 * dy1) / den)
    };
    let a = p1.x - p0.x + g * p1.x;
    let b = p3.x - p0.x + h * p3.x;
    let c = p0.x;
    let d = p1.y - p0.y + g * p1.y;
    let e = p3.y - p0.y + h * p3.y;
    let f = p0.y;
    let m = Mat3([a, b, c, d, e, f, g, h, 1.0]);
    if m.det().abs() < 1e-14 {
        return Err(HomographyError::Degenerate);
    }
    Ok(m)
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn close(a: Vec2, b: Vec2) -> bool {
        a.distance(b) < 1e-7
    }

    #[test]
    fn vec2_arithmetic() {
        let a = Vec2::new(1.0, 2.0);
        let b = Vec2::new(3.0, -1.0);
        assert_eq!(a + b, Vec2::new(4.0, 1.0));
        assert_eq!(a - b, Vec2::new(-2.0, 3.0));
        assert_eq!(a * 2.0, Vec2::new(2.0, 4.0));
        assert!((a.dot(b) - 1.0).abs() < 1e-12);
        assert!((a.cross(b) + 7.0).abs() < 1e-12);
        assert!((Vec2::new(3.0, 4.0).length() - 5.0).abs() < 1e-12);
        assert_eq!(a.lerp(b, 0.5), Vec2::new(2.0, 0.5));
        assert!(!Vec2::new(f64::NAN, 0.0).is_finite());
        assert_eq!(Vec2::ZERO, Vec2::default());
    }

    #[test]
    fn identity_matrix_is_neutral() {
        let p = Vec2::new(0.3, 0.7);
        assert_eq!(Mat3::IDENTITY.apply(p), Some(p));
        assert_eq!(Mat3::IDENTITY.mul(&Mat3::IDENTITY), Mat3::IDENTITY);
        assert_eq!(Mat3::IDENTITY.inverse(), Some(Mat3::IDENTITY));
        assert!((Mat3::IDENTITY.det() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn singular_matrix_has_no_inverse() {
        let m = Mat3([1.0, 2.0, 3.0, 2.0, 4.0, 6.0, 0.0, 0.0, 1.0]);
        assert_eq!(m.inverse(), None);
    }

    #[test]
    fn apply_rejects_points_at_infinity() {
        let m = Mat3([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0]);
        assert_eq!(m.apply(Vec2::new(0.0, 1.0)), None);
        assert_eq!(m.normalised(), m, "normalising with w=0 is a no-op");
    }

    #[test]
    fn identity_homography_from_unit_square() {
        let h = Homography::unit_square_to([
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ])
        .unwrap();
        let p = Vec2::new(0.25, 0.75);
        assert!(close(h.map(p).unwrap(), p));
        assert!(close(h.unmap(p).unwrap(), p));
        assert_eq!(Homography::identity().matrix(), &Mat3::IDENTITY);
    }

    #[test]
    fn corners_land_exactly() {
        let dst = [
            Vec2::new(0.1, 0.2),
            Vec2::new(0.9, 0.1),
            Vec2::new(0.8, 0.95),
            Vec2::new(0.05, 0.8),
        ];
        let h = Homography::unit_square_to(dst).unwrap();
        let src = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        for (s, d) in src.iter().zip(dst.iter()) {
            assert!(close(h.map(*s).unwrap(), *d));
            assert!(close(h.unmap(*d).unwrap(), *s));
        }
        let hom = h.matrix().apply_homogeneous(Vec2::new(0.5, 0.5));
        let div = h.map(Vec2::new(0.5, 0.5)).unwrap();
        assert!(close(Vec2::new(hom[0] / hom[2], hom[1] / hom[2]), div));
        assert_eq!(
            h.inverse_matrix()
                .mul(h.matrix())
                .normalised()
                .at(0, 0)
                .round(),
            1.0
        );
    }

    #[test]
    fn affine_quad_takes_the_parallelogram_branch() {
        let dst = [
            Vec2::new(0.1, 0.1),
            Vec2::new(0.6, 0.2),
            Vec2::new(0.7, 0.7),
            Vec2::new(0.2, 0.6),
        ];
        let h = Homography::unit_square_to(dst).unwrap();
        assert!(h.matrix().at(2, 0).abs() < 1e-12);
        assert!(h.matrix().at(2, 1).abs() < 1e-12);
        assert!(close(
            h.map(Vec2::new(0.5, 0.5)).unwrap(),
            Vec2::new(0.4, 0.4)
        ));
    }

    #[test]
    fn degenerate_quads_are_rejected() {
        let collinear = [
            Vec2::new(0.0, 0.0),
            Vec2::new(0.5, 0.5),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        assert_eq!(
            Homography::unit_square_to(collinear),
            Err(HomographyError::Degenerate)
        );
        let coincident = [
            Vec2::new(0.0, 0.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        assert_eq!(
            Homography::unit_square_to(coincident),
            Err(HomographyError::Degenerate)
        );
        let nan = [
            Vec2::new(f64::NAN, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        assert_eq!(
            Homography::unit_square_to(nan),
            Err(HomographyError::NonFinite)
        );
        assert_eq!(
            HomographyError::Degenerate.to_string(),
            "degenerate point configuration: the four points do not span a quadrilateral"
        );
    }

    #[test]
    fn general_point_correspondence() {
        let src = [
            Vec2::new(0.1, 0.1),
            Vec2::new(0.9, 0.2),
            Vec2::new(0.8, 0.9),
            Vec2::new(0.2, 0.8),
        ];
        let dst = [
            Vec2::new(-1.0, 0.0),
            Vec2::new(2.0, 0.5),
            Vec2::new(3.0, 3.0),
            Vec2::new(0.0, 2.0),
        ];
        let h = Homography::from_points(src, dst).unwrap();
        for (s, d) in src.iter().zip(dst.iter()) {
            assert!(close(h.map(*s).unwrap(), *d));
        }
    }

    fn convex_quad() -> impl Strategy<Value = [Vec2; 4]> {
        // Perturb each unit-square corner within a disc small enough to keep the quad convex.
        let jitter = || (-0.2f64..0.2, -0.2f64..0.2);
        (jitter(), jitter(), jitter(), jitter()).prop_map(|(a, b, c, d)| {
            [
                Vec2::new(a.0, a.1),
                Vec2::new(1.0 + b.0, b.1),
                Vec2::new(1.0 + c.0, 1.0 + c.1),
                Vec2::new(d.0, 1.0 + d.1),
            ]
        })
    }

    proptest! {
        #[test]
        fn homography_round_trips(dst in convex_quad(), x in 0.0f64..1.0, y in 0.0f64..1.0) {
            let h = Homography::unit_square_to(dst).unwrap();
            let p = Vec2::new(x, y);
            let q = h.map(p).unwrap();
            let back = h.unmap(q).unwrap();
            prop_assert!(back.distance(p) < 1e-6, "{p:?} -> {q:?} -> {back:?}");
        }

        #[test]
        fn inverse_times_forward_is_identity(dst in convex_quad()) {
            let h = Homography::unit_square_to(dst).unwrap();
            let i = h.inverse_matrix().mul(h.matrix()).normalised();
            for (a, b) in i.0.iter().zip(Mat3::IDENTITY.0.iter()) {
                prop_assert!((a - b).abs() < 1e-6);
            }
        }

        #[test]
        fn matrix_inverse_round_trips(v in prop::array::uniform9(-5.0f64..5.0)) {
            let m = Mat3(v);
            if let Some(inv) = m.inverse() {
                let p = Vec2::new(0.3, 0.4);
                if let (Some(q), Some(back)) = (m.apply(p), inv.apply(m.apply(p).unwrap())) {
                    prop_assert!(q.is_finite());
                    prop_assert!(back.distance(p) < 1e-4 * (1.0 + m.det().abs().recip()));
                }
            }
        }
    }
}
