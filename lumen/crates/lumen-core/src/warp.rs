//! Warps: the geometry that bends a flat source image onto a physical surface.
//!
//! Two warp types cover the vast majority of real shows:
//!
//! - [`Quad`]: a corner-pin. Four corners define a homography, so straight lines stay straight
//!   and texture is sampled perspective-correctly. Right for flat walls, boxes and screens.
//! - [`BezierMesh`]: a grid of bicubic Bezier patches. Right for curved screens, domes, columns
//!   and anything a quad cannot hug.
//!
//! Both tessellate into a [`WarpMesh`] of triangles in normalised output space with texture
//! coordinates in normalised source space; the renderer never needs to know which warp made it.

use crate::geom::{Homography, HomographyError, Vec2};
use serde::{Deserialize, Serialize};

/// Errors produced while building a warp.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WarpError {
    /// The quad corners are degenerate.
    #[error(transparent)]
    Homography(#[from] HomographyError),
    /// A mesh dimension or control-point count is invalid.
    #[error("invalid mesh: {0}")]
    InvalidMesh(String),
}

/// A tessellated warp ready for upload to the GPU or for the software renderer.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WarpMesh {
    /// Vertex positions in normalised output space.
    pub positions: Vec<Vec2>,
    /// Homogeneous texture coordinates `(u·q, v·q, q)` per vertex.
    ///
    /// For meshes `q` is always `1`. For quads `q` is the projective weight, so interpolating
    /// all three linearly across a triangle and dividing in the fragment shader gives exact
    /// perspective-correct sampling.
    pub texcoords: Vec<[f64; 3]>,
    /// Triangle list indices.
    pub indices: Vec<u32>,
}

impl WarpMesh {
    /// Number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// True when every vertex is finite and every triangle has positive (counter-clockwise in
    /// a y-down space) or zero area, i.e. the mesh never folds over itself.
    pub fn is_well_formed(&self) -> bool {
        if self.positions.len() != self.texcoords.len() {
            return false;
        }
        if !self.positions.iter().all(|p| p.is_finite()) {
            return false;
        }
        if !self
            .texcoords
            .iter()
            .all(|t| t.iter().all(|v| v.is_finite()))
        {
            return false;
        }
        self.indices.chunks_exact(3).all(|t| {
            let (a, b, c) = (
                self.positions[t[0] as usize],
                self.positions[t[1] as usize],
                self.positions[t[2] as usize],
            );
            (b - a).cross(c - a) >= -1e-12
        })
    }
}

/// Anything that can be tessellated into a [`WarpMesh`].
pub trait Warp {
    /// Tessellates with roughly `density` subdivisions per axis.
    fn tessellate(&self, density: u32) -> Result<WarpMesh, WarpError>;
}

/// Corner-pin warp. Corners are ordered top-left, top-right, bottom-right, bottom-left.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Quad {
    /// Destination corners in normalised output space.
    pub corners: [Vec2; 4],
}

impl Default for Quad {
    fn default() -> Self {
        Self::full_frame()
    }
}

impl Quad {
    /// A quad covering the whole output.
    pub fn full_frame() -> Self {
        Self {
            corners: [
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(1.0, 1.0),
                Vec2::new(0.0, 1.0),
            ],
        }
    }

    /// The homography this quad defines.
    pub fn homography(&self) -> Result<Homography, HomographyError> {
        Homography::unit_square_to(self.corners)
    }

    /// Moves one corner. Index order matches [`Quad::corners`].
    pub fn set_corner(&mut self, index: usize, p: Vec2) {
        if let Some(c) = self.corners.get_mut(index) {
            *c = p;
        }
    }

    /// Centroid of the four corners.
    pub fn centroid(&self) -> Vec2 {
        self.corners
            .iter()
            .fold(Vec2::ZERO, |acc, c| acc + *c)
            .scale(0.25)
    }
}

impl Warp for Quad {
    /// A quad needs only two triangles because the homogeneous texture coordinates carry the
    /// perspective; `density` is ignored.
    fn tessellate(&self, _density: u32) -> Result<WarpMesh, WarpError> {
        let h = self.homography()?;
        let inv = h.inverse_matrix();
        // Texture coordinate at each destination corner, expressed homogeneously so the GPU can
        // interpolate 1/w. We take the inverse map of each corner, which yields (u, v, 1) scaled
        // by the projective weight at that corner.
        // H⁻¹·c = H⁻¹·(H·s)/w = s/w, so the inverse map of each destination corner is exactly
        // (u/w, v/w, 1/w): the triple a rasteriser interpolates linearly for perspective-correct
        // sampling. All four weights share a sign for a convex quad; flip the lot if negative.
        let mut texcoords: Vec<[f64; 3]> = self
            .corners
            .iter()
            .map(|c| inv.apply_homogeneous(*c))
            .collect();
        let sign = texcoords[0][2].signum();
        for t in &mut texcoords {
            if t[2].abs() < 1e-12 || !t[2].is_finite() || t[2] * sign <= 0.0 {
                return Err(WarpError::Homography(HomographyError::Degenerate));
            }
            for v in t.iter_mut() {
                *v *= sign;
            }
        }
        Ok(WarpMesh {
            positions: self.corners.to_vec(),
            texcoords,
            indices: vec![0, 1, 2, 0, 2, 3],
        })
    }
}

/// A grid of bicubic Bezier patches.
///
/// `cols × rows` patches have `(3·cols + 1) × (3·rows + 1)` control points, stored row-major
/// from the top-left. Control points on patch boundaries are shared, so moving one moves the
/// edge of both neighbours and the surface stays C0 continuous.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BezierMesh {
    /// Patches across.
    pub cols: u32,
    /// Patches down.
    pub rows: u32,
    /// Control points in normalised output space, row-major.
    pub control_points: Vec<Vec2>,
}

impl BezierMesh {
    /// Maximum patches per axis; keeps the control net draggable and the tessellation bounded.
    pub const MAX_PATCHES: u32 = 16;

    /// Builds a flat, full-frame mesh of `cols × rows` patches.
    pub fn flat(cols: u32, rows: u32) -> Result<Self, WarpError> {
        Self::validate_dims(cols, rows)?;
        let (w, h) = (3 * cols + 1, 3 * rows + 1);
        let mut control_points = Vec::with_capacity((w * h) as usize);
        for j in 0..h {
            for i in 0..w {
                control_points.push(Vec2::new(
                    f64::from(i) / f64::from(w - 1),
                    f64::from(j) / f64::from(h - 1),
                ));
            }
        }
        Ok(Self {
            cols,
            rows,
            control_points,
        })
    }

    fn validate_dims(cols: u32, rows: u32) -> Result<(), WarpError> {
        if cols == 0 || rows == 0 {
            return Err(WarpError::InvalidMesh(
                "cols and rows must be at least 1".into(),
            ));
        }
        if cols > Self::MAX_PATCHES || rows > Self::MAX_PATCHES {
            return Err(WarpError::InvalidMesh(format!(
                "cols and rows must be at most {}",
                Self::MAX_PATCHES
            )));
        }
        Ok(())
    }

    /// Checks dimensions and control-point count agree and all points are finite.
    pub fn validate(&self) -> Result<(), WarpError> {
        Self::validate_dims(self.cols, self.rows)?;
        let expected = ((3 * self.cols + 1) * (3 * self.rows + 1)) as usize;
        if self.control_points.len() != expected {
            return Err(WarpError::InvalidMesh(format!(
                "expected {expected} control points for {}x{} patches, found {}",
                self.cols,
                self.rows,
                self.control_points.len()
            )));
        }
        if !self.control_points.iter().all(|p| p.is_finite()) {
            return Err(WarpError::InvalidMesh("control point is not finite".into()));
        }
        Ok(())
    }

    /// Control points per row.
    pub fn width(&self) -> u32 {
        3 * self.cols + 1
    }

    /// Control points per column.
    pub fn height(&self) -> u32 {
        3 * self.rows + 1
    }

    /// Control point at grid coordinates.
    pub fn point(&self, i: u32, j: u32) -> Option<Vec2> {
        if i >= self.width() || j >= self.height() {
            return None;
        }
        self.control_points
            .get((j * self.width() + i) as usize)
            .copied()
    }

    /// Moves a control point. Out-of-range coordinates are ignored.
    pub fn set_point(&mut self, i: u32, j: u32, p: Vec2) {
        if i < self.width() && j < self.height() {
            let idx = (j * self.width() + i) as usize;
            if let Some(slot) = self.control_points.get_mut(idx) {
                *slot = p;
            }
        }
    }

    /// Evaluates the surface at `(u, v)` in `[0,1]²` of the whole mesh.
    pub fn evaluate(&self, u: f64, v: f64) -> Vec2 {
        let u = u.clamp(0.0, 1.0);
        let v = v.clamp(0.0, 1.0);
        let su = u * f64::from(self.cols);
        let sv = v * f64::from(self.rows);
        let pi = (su.floor() as u32).min(self.cols - 1);
        let pj = (sv.floor() as u32).min(self.rows - 1);
        let (tu, tv) = (su - f64::from(pi), sv - f64::from(pj));
        let bu = bernstein(tu);
        let bv = bernstein(tv);
        let mut out = Vec2::ZERO;
        for (b, dj) in bv.iter().zip(0..4u32) {
            for (a, di) in bu.iter().zip(0..4u32) {
                let p = self.point(3 * pi + di, 3 * pj + dj).unwrap_or_default();
                out = out + p.scale(a * b);
            }
        }
        out
    }
}

/// Cubic Bernstein basis at `t`.
fn bernstein(t: f64) -> [f64; 4] {
    let s = 1.0 - t;
    [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t]
}

impl Warp for BezierMesh {
    fn tessellate(&self, density: u32) -> Result<WarpMesh, WarpError> {
        self.validate()?;
        let density = density.clamp(1, 64);
        let nx = self.cols * density;
        let ny = self.rows * density;
        let mut positions = Vec::with_capacity(((nx + 1) * (ny + 1)) as usize);
        let mut texcoords = Vec::with_capacity(positions.capacity());
        for j in 0..=ny {
            for i in 0..=nx {
                let u = f64::from(i) / f64::from(nx);
                let v = f64::from(j) / f64::from(ny);
                positions.push(self.evaluate(u, v));
                texcoords.push([u, v, 1.0]);
            }
        }
        let mut indices = Vec::with_capacity((nx * ny * 6) as usize);
        for j in 0..ny {
            for i in 0..nx {
                let a = j * (nx + 1) + i;
                let b = a + 1;
                let c = a + nx + 1;
                let d = c + 1;
                indices.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
        Ok(WarpMesh {
            positions,
            texcoords,
            indices,
        })
    }
}

/// Either warp kind, as stored in a project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WarpKind {
    /// Corner-pin.
    Quad(Quad),
    /// Bezier mesh.
    Bezier(BezierMesh),
}

impl Default for WarpKind {
    fn default() -> Self {
        Self::Quad(Quad::full_frame())
    }
}

impl Warp for WarpKind {
    fn tessellate(&self, density: u32) -> Result<WarpMesh, WarpError> {
        match self {
            Self::Quad(q) => q.tessellate(density),
            Self::Bezier(b) => b.tessellate(density),
        }
    }
}

impl WarpKind {
    /// Converts a quad into an equivalent flat Bezier mesh of the given size, or returns the
    /// mesh unchanged. Useful when a user upgrades a surface from corner-pin to mesh.
    pub fn to_bezier(&self, cols: u32, rows: u32) -> Result<BezierMesh, WarpError> {
        match self {
            Self::Bezier(b) => Ok(b.clone()),
            Self::Quad(q) => {
                let h = q.homography()?;
                let mut mesh = BezierMesh::flat(cols, rows)?;
                for p in &mut mesh.control_points {
                    *p = h.map(*p).ok_or(HomographyError::Degenerate)?;
                }
                Ok(mesh)
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn full_frame_quad_is_identity_mesh() {
        let m = Quad::full_frame().tessellate(8).unwrap();
        assert_eq!(m.triangle_count(), 2);
        assert!(m.is_well_formed());
        for (p, t) in m.positions.iter().zip(m.texcoords.iter()) {
            assert!((t[0] / t[2] - p.x).abs() < 1e-9);
            assert!((t[1] / t[2] - p.y).abs() < 1e-9);
        }
        assert_eq!(Quad::default(), Quad::full_frame());
        assert_eq!(Quad::full_frame().centroid(), Vec2::new(0.5, 0.5));
    }

    #[test]
    fn quad_texcoords_divide_back_to_unit_corners() {
        let mut q = Quad::full_frame();
        q.set_corner(2, Vec2::new(0.7, 0.9));
        q.set_corner(9, Vec2::new(9.0, 9.0)); // ignored
        let m = q.tessellate(1).unwrap();
        let expected = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        for (t, (u, v)) in m.texcoords.iter().zip(expected) {
            assert!((t[0] / t[2] - u).abs() < 1e-9, "{t:?}");
            assert!((t[1] / t[2] - v).abs() < 1e-9, "{t:?}");
            assert!(t[2] > 0.0, "weights must be positive for the GPU divide");
        }
    }

    #[test]
    fn quad_degenerate_errors_propagate() {
        let mut q = Quad::full_frame();
        q.set_corner(1, Vec2::new(0.0, 0.0));
        let err = q.tessellate(1).unwrap_err();
        assert!(matches!(
            err,
            WarpError::Homography(HomographyError::Degenerate)
        ));
        assert!(err.to_string().contains("degenerate"));
    }

    #[test]
    fn flat_bezier_is_identity() {
        let b = BezierMesh::flat(2, 3).unwrap();
        assert_eq!(b.width(), 7);
        assert_eq!(b.height(), 10);
        b.validate().unwrap();
        for (u, v) in [(0.0, 0.0), (1.0, 1.0), (0.37, 0.81), (0.5, 0.0)] {
            let p = b.evaluate(u, v);
            assert!(
                (p.x - u).abs() < 1e-9 && (p.y - v).abs() < 1e-9,
                "{u},{v} -> {p:?}"
            );
        }
        let m = b.tessellate(4).unwrap();
        assert_eq!(m.positions.len(), 9 * 13);
        assert_eq!(m.triangle_count(), 8 * 12 * 2);
        assert!(m.is_well_formed());
        for (p, t) in m.positions.iter().zip(&m.texcoords) {
            assert!((p.x - t[0]).abs() < 1e-9 && (p.y - t[1]).abs() < 1e-9);
            assert_eq!(t[2], 1.0);
        }
    }

    #[test]
    fn bezier_point_access_bounds() {
        let mut b = BezierMesh::flat(1, 1).unwrap();
        assert_eq!(b.point(4, 0), None);
        assert_eq!(b.point(0, 4), None);
        b.set_point(4, 4, Vec2::new(5.0, 5.0));
        assert!(b.control_points.iter().all(|p| p.x <= 1.0));
        b.set_point(3, 3, Vec2::new(1.2, 1.3));
        assert_eq!(b.point(3, 3), Some(Vec2::new(1.2, 1.3)));
        assert_eq!(b.evaluate(1.0, 1.0), Vec2::new(1.2, 1.3));
        assert_eq!(
            b.evaluate(2.0, -1.0),
            b.evaluate(1.0, 0.0),
            "evaluate clamps"
        );
    }

    #[test]
    fn bezier_validation_errors() {
        assert!(BezierMesh::flat(0, 1).is_err());
        assert!(BezierMesh::flat(1, BezierMesh::MAX_PATCHES + 1).is_err());
        let mut b = BezierMesh::flat(1, 1).unwrap();
        b.control_points.pop();
        let err = b.validate().unwrap_err();
        assert!(err.to_string().contains("expected 16 control points"));
        let mut b = BezierMesh::flat(1, 1).unwrap();
        b.control_points[0] = Vec2::new(f64::INFINITY, 0.0);
        assert!(b.validate().is_err());
        assert!(b.tessellate(2).is_err());
    }

    #[test]
    fn tessellation_density_is_clamped() {
        let b = BezierMesh::flat(1, 1).unwrap();
        assert_eq!(b.tessellate(0).unwrap().triangle_count(), 2);
        assert_eq!(b.tessellate(1000).unwrap().triangle_count(), 64 * 64 * 2);
    }

    #[test]
    fn warp_kind_dispatch_and_conversion() {
        assert!(matches!(WarpKind::default(), WarpKind::Quad(_)));
        let q = WarpKind::Quad(Quad {
            corners: [
                Vec2::new(0.1, 0.1),
                Vec2::new(0.9, 0.2),
                Vec2::new(0.8, 0.9),
                Vec2::new(0.2, 0.8),
            ],
        });
        let qm = q.tessellate(1).unwrap();
        assert_eq!(qm.triangle_count(), 2);
        let b = q.to_bezier(2, 2).unwrap();
        let bk = WarpKind::Bezier(b.clone());
        assert_eq!(bk.to_bezier(1, 1).unwrap(), b);
        // The converted mesh must reproduce the quad's homography at interior points.
        let h = Quad {
            corners: qm.positions.clone().try_into().unwrap(),
        }
        .homography()
        .unwrap();
        let mesh = bk.tessellate(6).unwrap();
        assert!(mesh.is_well_formed());
        // Bezier interpolation of a projective map is not exact, but corners are.
        for (u, v) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            let got = b.evaluate(u, v);
            let want = h.map(Vec2::new(u, v)).unwrap();
            assert!(got.distance(want) < 1e-9);
        }
        let mut bad = Quad::full_frame();
        bad.set_corner(0, Vec2::new(1.0, 0.0));
        assert!(WarpKind::Quad(bad).to_bezier(1, 1).is_err());
    }

    #[test]
    fn well_formed_detects_folds_and_mismatches() {
        let mut m = Quad::full_frame().tessellate(1).unwrap();
        assert!(m.is_well_formed());
        m.positions[1] = Vec2::new(-1.0, 1.5); // fold the first triangle
        assert!(!m.is_well_formed());
        let mut m = Quad::full_frame().tessellate(1).unwrap();
        m.texcoords.pop();
        assert!(!m.is_well_formed());
        let mut m = Quad::full_frame().tessellate(1).unwrap();
        m.texcoords[0][2] = f64::NAN;
        assert!(!m.is_well_formed());
        let mut m = Quad::full_frame().tessellate(1).unwrap();
        m.positions[0] = Vec2::new(f64::NAN, 0.0);
        assert!(!m.is_well_formed());
    }

    fn small_perturbation() -> impl Strategy<Value = Vec<Vec2>> {
        prop::collection::vec((-0.03f64..0.03, -0.03f64..0.03), 16)
            .prop_map(|v| v.into_iter().map(|(x, y)| Vec2::new(x, y)).collect())
    }

    proptest! {
        #[test]
        fn perturbed_bezier_never_folds(d in small_perturbation(), density in 1u32..8) {
            let mut b = BezierMesh::flat(1, 1).unwrap();
            for (p, delta) in b.control_points.iter_mut().zip(d) {
                *p = *p + delta;
            }
            let m = b.tessellate(density).unwrap();
            prop_assert!(m.is_well_formed());
            prop_assert_eq!(m.triangle_count() as u32, density * density * 2);
        }

        #[test]
        fn bezier_corners_are_interpolated(d in small_perturbation()) {
            let mut b = BezierMesh::flat(1, 1).unwrap();
            for (p, delta) in b.control_points.iter_mut().zip(d) {
                *p = *p + delta;
            }
            prop_assert_eq!(b.evaluate(0.0, 0.0), b.point(0, 0).unwrap());
            prop_assert_eq!(b.evaluate(1.0, 0.0), b.point(3, 0).unwrap());
            prop_assert_eq!(b.evaluate(1.0, 1.0), b.point(3, 3).unwrap());
            prop_assert_eq!(b.evaluate(0.0, 1.0), b.point(0, 3).unwrap());
        }
    }
}
