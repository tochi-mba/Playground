//! A render-ready description of one output.

use lumen_core::blend::EdgeBlend;
use lumen_core::geom::Vec2;
use lumen_core::mask::combined_coverage;
use lumen_core::project::{BlendMode, ColorAdjust, Crop, Overlay, Project};
use lumen_core::warp::{Quad, Warp, WarpError, WarpMesh};
use lumen_sources::{Frame, SourceSet};
use std::sync::Arc;

/// Side length of the rasterised mask texture, in texels.
pub const MASK_RESOLUTION: u32 = 256;

/// Default subdivision per Bezier patch.
pub const DEFAULT_MESH_DENSITY: u32 = 16;

/// Errors from building a scene.
#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    /// The output id does not exist.
    #[error("unknown output {0:?}")]
    UnknownOutput(String),
    /// A surface's warp could not be tessellated.
    #[error("surface {0:?}: {1}")]
    Warp(String, WarpError),
}

/// Coverage of a surface's masks, rasterised in surface space so both renderers sample the
/// same texels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaskRaster {
    /// Side length.
    pub size: u32,
    /// Row-major coverage, `255` is fully visible.
    pub data: Vec<u8>,
}

impl MaskRaster {
    /// Fully visible.
    pub fn full(size: u32) -> Self {
        Self {
            size,
            data: vec![255; (size * size) as usize],
        }
    }

    /// Rasterises `masks` at `size × size` texel centres.
    pub fn from_masks(masks: &[lumen_core::mask::Mask], size: u32) -> Self {
        if masks.is_empty() {
            return Self::full(size);
        }
        let mut data = Vec::with_capacity((size * size) as usize);
        for j in 0..size {
            for i in 0..size {
                let p = Vec2::new(
                    (f64::from(i) + 0.5) / f64::from(size),
                    (f64::from(j) + 0.5) / f64::from(size),
                );
                data.push((combined_coverage(masks, p).clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        Self { size, data }
    }

    /// Bilinear sample at normalised `(u, v)`, in `[0,1]`.
    pub fn sample(&self, u: f64, v: f64) -> f64 {
        let n = f64::from(self.size);
        let fx = u.clamp(0.0, 1.0) * n - 0.5;
        let fy = v.clamp(0.0, 1.0) * n - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let at = |x: f64, y: f64| {
            let xi = (x.max(0.0) as u32).min(self.size - 1);
            let yi = (y.max(0.0) as u32).min(self.size - 1);
            f64::from(self.data[(yi * self.size + xi) as usize]) / 255.0
        };
        let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1.0, y0) * tx;
        let bottom = at(x0, y0 + 1.0) * (1.0 - tx) + at(x0 + 1.0, y0 + 1.0) * tx;
        top * (1.0 - ty) + bottom * ty
    }

    /// True when no texel hides anything.
    pub fn is_full(&self) -> bool {
        self.data.iter().all(|&v| v == 255)
    }
}

/// One surface, ready to draw.
#[derive(Debug, Clone)]
pub struct SurfaceDraw {
    /// Surface id, for diagnostics.
    pub id: String,
    /// Tessellated warp.
    pub mesh: WarpMesh,
    /// Source pixels.
    pub frame: Arc<Frame>,
    /// Source id, so a renderer can cache textures per source.
    pub source_id: String,
    /// Opacity in `[0,1]`.
    pub opacity: f64,
    /// Compositing mode.
    pub blend_mode: BlendMode,
    /// Source crop.
    pub crop: Crop,
    /// Mask coverage.
    pub mask: MaskRaster,
}

/// Post-processing applied to the composited output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Post {
    /// Edge blend.
    pub blend: EdgeBlend,
    /// Colour adjustments.
    pub color: ColorAdjust,
    /// Master brightness.
    pub master: f64,
    /// Blackout.
    pub blackout: bool,
}

impl Post {
    /// Final multiplier and transfer applied to a composited colour at normalised `(x, y)`.
    ///
    /// Order: contrast, brightness, output gamma, edge blend, master, blackout. Both renderers
    /// implement exactly this sequence.
    pub fn apply(&self, rgb: [f64; 3], x: f64, y: f64) -> [f64; 3] {
        if self.blackout {
            return [0.0; 3];
        }
        let factor = self.blend.encoded_factor(x, y) * self.master.clamp(0.0, 1.0);
        let c = self.color;
        rgb.map(|v| {
            let v = ((v - 0.5) * c.contrast + 0.5).clamp(0.0, 1.0);
            let v = (v * c.brightness).clamp(0.0, 1.0);
            let v = if (c.gamma - 1.0).abs() < 1e-9 {
                v
            } else {
                v.powf(1.0 / c.gamma.max(1e-3))
            };
            (v * factor).clamp(0.0, 1.0)
        })
    }
}

/// Remembers rasterised masks per surface (and generated overlays per size) so an unchanged
/// mask or overlay costs nothing per frame.
#[derive(Debug, Default)]
pub struct MaskCache {
    entries: std::collections::HashMap<String, (Vec<lumen_core::mask::Mask>, MaskRaster)>,
    overlays: std::collections::HashMap<(Overlay, u32, u32), Arc<Frame>>,
}

impl MaskCache {
    /// Returns the overlay frame at `width × height`, generating it once.
    pub fn overlay(&mut self, kind: Overlay, width: u32, height: u32) -> Arc<Frame> {
        Arc::clone(
            self.overlays
                .entry((kind, width, height))
                .or_insert_with(|| Arc::new(lumen_sources::overlay::generate(kind, width, height))),
        )
    }

    /// Returns the raster for `masks`, rasterising only when they changed since last time.
    pub fn get(&mut self, surface_id: &str, masks: &[lumen_core::mask::Mask]) -> MaskRaster {
        if let Some((cached_masks, raster)) = self.entries.get(surface_id) {
            if cached_masks.as_slice() == masks {
                return raster.clone();
            }
        }
        let raster = MaskRaster::from_masks(masks, MASK_RESOLUTION);
        self.entries
            .insert(surface_id.to_string(), (masks.to_vec(), raster.clone()));
        raster
    }

    /// Number of surfaces remembered.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Everything needed to render one output.
#[derive(Debug, Clone)]
pub struct Scene {
    /// Output id.
    pub output_id: String,
    /// Output pixel size.
    pub width: u32,
    /// Output pixel size.
    pub height: u32,
    /// Surfaces in draw order.
    pub draws: Vec<SurfaceDraw>,
    /// Post-processing.
    pub post: Post,
}

impl Scene {
    /// Builds the scene for `output_id`. Surfaces whose source has not produced a frame yet
    /// are skipped; invisible surfaces are skipped; a broken warp is an error.
    pub fn build(
        project: &Project,
        output_id: &str,
        sources: &SourceSet,
        mesh_density: u32,
    ) -> Result<Self, SceneError> {
        Self::build_cached(
            project,
            output_id,
            sources,
            mesh_density,
            &mut MaskCache::default(),
        )
    }

    /// Like [`Scene::build`], reusing rasterised masks from `cache`.
    pub fn build_cached(
        project: &Project,
        output_id: &str,
        sources: &SourceSet,
        mesh_density: u32,
        cache: &mut MaskCache,
    ) -> Result<Self, SceneError> {
        Self::build_with(project, output_id, sources, mesh_density, cache, true)
    }

    /// Like [`Scene::build_cached`]; `overlays = false` leaves calibration overlays out, as
    /// show mode does.
    pub fn build_with(
        project: &Project,
        output_id: &str,
        sources: &SourceSet,
        mesh_density: u32,
        cache: &mut MaskCache,
        overlays: bool,
    ) -> Result<Self, SceneError> {
        let output = project
            .output(output_id)
            .ok_or_else(|| SceneError::UnknownOutput(output_id.to_string()))?;
        let mut draws = Vec::new();
        for s in project.surfaces_for_output(output_id) {
            if !s.visible || s.opacity <= 0.0 {
                continue;
            }
            let Some(frame) = sources.frame(&s.source) else {
                continue;
            };
            let mesh = s
                .warp
                .tessellate(mesh_density)
                .map_err(|e| SceneError::Warp(s.id.clone(), e))?;
            draws.push(SurfaceDraw {
                id: s.id.clone(),
                mesh,
                frame: Arc::clone(frame),
                source_id: s.source.clone(),
                opacity: s.opacity.clamp(0.0, 1.0),
                blend_mode: s.blend_mode,
                crop: s.crop,
                mask: cache.get(&s.id, &s.masks),
            });
        }
        let (width, height) = (output.width.max(1), output.height.max(1));
        if let Some(kind) = output.overlay.filter(|_| overlays) {
            draws.push(SurfaceDraw {
                id: format!("overlay:{kind:?}"),
                mesh: Quad::full_frame()
                    .tessellate(1)
                    .map_err(|e| SceneError::Warp("overlay".into(), e))?,
                frame: cache.overlay(kind, width, height),
                source_id: format!("overlay:{kind:?}:{width}x{height}"),
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                crop: Crop::default(),
                mask: MaskRaster::full(1),
            });
        }
        Ok(Self {
            output_id: output_id.to_string(),
            width,
            height,
            draws,
            post: Post {
                blend: output.blend,
                color: output.color,
                master: project.master,
                blackout: project.blackout,
            },
        })
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used)]
mod tests {
    use super::*;
    use lumen_core::mask::Mask;
    use lumen_core::project::demo_project;
    use std::path::Path;

    fn demo_sources(p: &Project) -> SourceSet {
        let (mut s, errs) = SourceSet::open(p, Path::new("."));
        assert!(errs.is_empty());
        s.poll();
        s
    }

    #[test]
    fn overlays_are_appended_cached_and_skipped_in_show_mode() {
        let mut p = demo_project();
        p.outputs[0].overlay = Some(Overlay::Grid);
        let s = demo_sources(&p);
        let mut cache = MaskCache::default();
        let scene = Scene::build_cached(&p, "left", &s, 4, &mut cache).unwrap();
        let last = scene.draws.last().unwrap();
        assert_eq!(last.id, "overlay:Grid");
        assert_eq!(last.source_id, "overlay:Grid:1920x1080");
        assert_eq!((last.frame.width, last.frame.height), (1920, 1080));
        assert!(last.mask.is_full());
        let again = Scene::build_cached(&p, "left", &s, 4, &mut cache).unwrap();
        assert!(
            Arc::ptr_eq(&again.draws.last().unwrap().frame, &last.frame),
            "the overlay frame is generated once per size"
        );
        let show = Scene::build_with(&p, "left", &s, 4, &mut cache, false).unwrap();
        assert!(show.draws.iter().all(|d| !d.id.starts_with("overlay:")));
        p.outputs[0].overlay = None;
        let plain = Scene::build_cached(&p, "left", &s, 4, &mut cache).unwrap();
        assert_eq!(plain.draws.len(), show.draws.len());
    }

    #[test]
    fn builds_demo_scene_in_draw_order() {
        let p = demo_project();
        let s = demo_sources(&p);
        let scene = Scene::build(&p, "right", &s, 4).unwrap();
        assert_eq!((scene.width, scene.height), (1920, 1080));
        assert_eq!(
            scene
                .draws
                .iter()
                .map(|d| d.id.as_str())
                .collect::<Vec<_>>(),
            ["wall-right", "logo"]
        );
        assert_eq!(scene.draws[1].blend_mode, BlendMode::Screen);
        assert!(!scene.draws[1].mask.is_full());
        assert!(scene.draws[0].mask.is_full());
        assert_eq!(scene.draws[0].source_id, "grid");
        assert!(matches!(
            Scene::build(&p, "nope", &s, 4),
            Err(SceneError::UnknownOutput(_))
        ));
    }

    #[test]
    fn skips_hidden_and_sourceless_surfaces_and_reports_bad_warps() {
        let mut p = demo_project();
        p.surfaces[1].visible = false;
        p.surfaces[2].source = "ghost".into();
        let s = demo_sources(&p);
        assert!(Scene::build(&p, "right", &s, 4).unwrap().draws.is_empty());
        p.surfaces[1].visible = true;
        p.surfaces[1].opacity = 0.0;
        assert!(Scene::build(&p, "right", &s, 4).unwrap().draws.is_empty());
        if let lumen_core::warp::WarpKind::Quad(q) = &mut p.surfaces[0].warp {
            q.corners[1] = q.corners[0];
        }
        let err = Scene::build(&p, "left", &s, 4).unwrap_err();
        assert!(err.to_string().starts_with("surface \"wall-left\""));
        p.outputs[0].width = 0;
        p.surfaces[0].warp = lumen_core::warp::WarpKind::default();
        assert_eq!(Scene::build(&p, "left", &s, 4).unwrap().width, 1);
    }

    #[test]
    fn mask_cache_reuses_unchanged_masks() {
        let mut cache = MaskCache::default();
        assert!(cache.is_empty());
        let hole = Mask::hole(vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(0.5, 0.0),
            Vec2::new(0.5, 1.0),
        ]);
        let a = cache.get("s", std::slice::from_ref(&hole));
        let b = cache.get("s", std::slice::from_ref(&hole));
        assert_eq!(a, b);
        assert_eq!(cache.len(), 1);
        let c = cache.get("s", &[]);
        assert!(c.is_full());
        assert_eq!(cache.len(), 1, "same surface id replaces the entry");
        let p = demo_project();
        let s = demo_sources(&p);
        let one = Scene::build(&p, "right", &s, 2).unwrap();
        let two = Scene::build_cached(&p, "right", &s, 2, &mut cache).unwrap();
        assert_eq!(one.draws[1].mask, two.draws[1].mask);
    }

    #[test]
    fn mask_raster_matches_coverage_and_samples_bilinearly() {
        let full = MaskRaster::full(4);
        assert!(full.is_full());
        assert_eq!(full.sample(0.3, 0.9), 1.0);
        let hole = Mask::hole(vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(0.5, 0.0),
            Vec2::new(0.5, 1.0),
            Vec2::new(0.0, 1.0),
        ]);
        let r = MaskRaster::from_masks(std::slice::from_ref(&hole), 64);
        assert_eq!(r.sample(0.25, 0.5), 0.0);
        assert_eq!(r.sample(0.75, 0.5), 1.0);
        let edge = r.sample(0.5, 0.5);
        assert!(
            edge > 0.2 && edge < 0.8,
            "bilinear across the hard edge: {edge}"
        );
        assert_eq!(r.sample(-1.0, 2.0), 0.0, "clamped");
    }

    #[test]
    fn post_order_and_clamping() {
        let mut post = Post {
            blend: EdgeBlend::none(),
            color: ColorAdjust::default(),
            master: 1.0,
            blackout: false,
        };
        assert_eq!(post.apply([0.25, 0.5, 2.0], 0.5, 0.5), [0.25, 0.5, 1.0]);
        post.master = 0.5;
        assert_eq!(post.apply([1.0, 1.0, 1.0], 0.5, 0.5), [0.5, 0.5, 0.5]);
        post.master = 1.0;
        post.color.contrast = 2.0;
        assert_eq!(post.apply([0.25, 0.75, 0.5], 0.5, 0.5), [0.0, 1.0, 0.5]);
        post.color = ColorAdjust {
            brightness: 2.0,
            contrast: 1.0,
            gamma: 1.0,
        };
        assert_eq!(post.apply([0.25, 0.75, 0.5], 0.5, 0.5), [0.5, 1.0, 1.0]);
        post.color = ColorAdjust {
            brightness: 1.0,
            contrast: 1.0,
            gamma: 2.0,
        };
        assert!((post.apply([0.25, 0.0, 1.0], 0.5, 0.5)[0] - 0.5).abs() < 1e-12);
        post.color = ColorAdjust::default();
        post.blend.right.width = 0.5;
        post.blend.gamma = 1.0;
        assert!((post.apply([1.0; 3], 0.75, 0.5)[0] - 0.5).abs() < 1e-12);
        post.blackout = true;
        assert_eq!(post.apply([1.0; 3], 0.5, 0.5), [0.0; 3]);
    }
}
