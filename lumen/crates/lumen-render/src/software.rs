//! CPU reference renderer.
//!
//! Everything is computed in `f64` per pixel with no shortcuts: perspective-correct texture
//! lookup, bilinear sampling, straight-alpha compositing with the four blend modes, then the
//! same post pass as the GPU. It is the oracle: if the GPU disagrees with it, the GPU is wrong.

use crate::scene::{Scene, SurfaceDraw};
use image::RgbaImage;
use lumen_core::geom::Vec2;
use lumen_core::project::BlendMode;

/// Renders a scene at its own output size.
pub fn render(scene: &Scene) -> RgbaImage {
    render_at(scene, scene.width, scene.height)
}

/// Renders a scene at an arbitrary size (previews, goldens).
pub fn render_at(scene: &Scene, width: u32, height: u32) -> RgbaImage {
    let (w, h) = (width.max(1), height.max(1));
    let mut buf = vec![[0.0f64; 3]; (w * h) as usize];
    for draw in &scene.draws {
        draw_surface(draw, &mut buf, w, h);
    }
    let mut out = RgbaImage::new(w, h);
    for (i, px) in out.pixels_mut().enumerate() {
        let x = (i as u32 % w, i as u32 / w);
        let nx = (f64::from(x.0) + 0.5) / f64::from(w);
        let ny = (f64::from(x.1) + 0.5) / f64::from(h);
        let c = scene.post.apply(buf[i], nx, ny);
        *px = image::Rgba([to_u8(c[0]), to_u8(c[1]), to_u8(c[2]), 255]);
    }
    out
}

fn to_u8(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Composites one premultiplied source colour onto a destination.
pub fn blend_pixel(mode: BlendMode, dst: [f64; 3], src_premul: [f64; 3], alpha: f64) -> [f64; 3] {
    let mut out = [0.0; 3];
    for c in 0..3 {
        let (d, s) = (dst[c], src_premul[c]);
        out[c] = match mode {
            BlendMode::Normal => s + d * (1.0 - alpha),
            BlendMode::Add => s + d,
            BlendMode::Multiply => d * (1.0 - alpha) + d * s,
            BlendMode::Screen => s + d * (1.0 - s),
        }
        .clamp(0.0, 1.0);
    }
    out
}

fn draw_surface(draw: &SurfaceDraw, buf: &mut [[f64; 3]], w: u32, h: u32) {
    let m = &draw.mesh;
    let to_px = |p: Vec2| Vec2::new(p.x * f64::from(w), p.y * f64::from(h));
    for tri in m.indices.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let (p0, p1, p2) = (
            to_px(m.positions[i0]),
            to_px(m.positions[i1]),
            to_px(m.positions[i2]),
        );
        let (t0, t1, t2) = (m.texcoords[i0], m.texcoords[i1], m.texcoords[i2]);
        let area = (p1 - p0).cross(p2 - p0);
        if area.abs() < 1e-12 {
            continue;
        }
        let min_x = p0.x.min(p1.x).min(p2.x).floor().max(0.0) as u32;
        let max_x = (p0.x.max(p1.x).max(p2.x).ceil() as i64).clamp(0, i64::from(w)) as u32;
        let min_y = p0.y.min(p1.y).min(p2.y).floor().max(0.0) as u32;
        let max_y = (p0.y.max(p1.y).max(p2.y).ceil() as i64).clamp(0, i64::from(h)) as u32;
        for py in min_y..max_y {
            for px in min_x..max_x {
                let p = Vec2::new(f64::from(px) + 0.5, f64::from(py) + 0.5);
                let w0 = (p1 - p).cross(p2 - p) / area;
                let w1 = (p2 - p).cross(p0 - p) / area;
                let w2 = 1.0 - w0 - w1;
                if !covers(w0, p1, p2, area)
                    || !covers(w1, p2, p0, area)
                    || !covers(w2, p0, p1, area)
                {
                    continue;
                }
                let uq = w0 * t0[0] + w1 * t1[0] + w2 * t2[0];
                let vq = w0 * t0[1] + w1 * t1[1] + w2 * t2[1];
                let q = w0 * t0[2] + w1 * t1[2] + w2 * t2[2];
                if q.abs() < 1e-12 {
                    continue;
                }
                let (u, v) = (uq / q, vq / q);
                let (u, v) = (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
                let mask = draw.mask.sample(u, v);
                let su = draw.crop.x + u * draw.crop.w;
                let sv = draw.crop.y + v * draw.crop.h;
                let src = draw.frame.sample_bilinear(su, sv);
                let alpha = src[3] * draw.opacity * mask;
                if alpha <= 0.0 {
                    continue;
                }
                let premul = [src[0] * alpha, src[1] * alpha, src[2] * alpha];
                let idx = (py * w + px) as usize;
                buf[idx] = blend_pixel(draw.blend_mode, buf[idx], premul, alpha);
            }
        }
    }
}

/// Top-left fill rule so that pixels on a shared edge are drawn exactly once.
fn covers(weight: f64, a: Vec2, b: Vec2, area: f64) -> bool {
    if weight > 0.0 {
        return true;
    }
    if weight < 0.0 {
        return false;
    }
    // Edge a→b, oriented so the triangle interior is on the consistent side.
    let e = if area > 0.0 { b - a } else { a - b };
    // In y-down space a "top" edge is horizontal going left; a "left" edge goes up.
    (e.y == 0.0 && e.x < 0.0) || e.y < 0.0
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::scene::{MaskRaster, Post};
    use lumen_core::blend::EdgeBlend;
    use lumen_core::project::{ColorAdjust, Crop};
    use lumen_core::warp::{Quad, Warp};
    use lumen_sources::Frame;
    use std::sync::Arc;

    fn scene_with(draws: Vec<SurfaceDraw>) -> Scene {
        Scene {
            output_id: "o".into(),
            width: 16,
            height: 8,
            draws,
            post: Post {
                blend: EdgeBlend::none(),
                color: ColorAdjust::default(),
                master: 1.0,
                blackout: false,
            },
        }
    }

    fn draw(frame: Frame, quad: Quad, mode: BlendMode, opacity: f64) -> SurfaceDraw {
        SurfaceDraw {
            id: "s".into(),
            mesh: quad.tessellate(1).unwrap(),
            frame: Arc::new(frame),
            source_id: "f".into(),
            opacity,
            blend_mode: mode,
            crop: Crop::default(),
            mask: MaskRaster::full(4),
        }
    }

    #[test]
    fn empty_scene_is_black_and_sized() {
        let img = render(&scene_with(vec![]));
        assert_eq!(img.dimensions(), (16, 8));
        assert!(img.pixels().all(|p| p.0 == [0, 0, 0, 255]));
        assert_eq!(render_at(&scene_with(vec![]), 0, 0).dimensions(), (1, 1));
    }

    #[test]
    fn full_frame_quad_fills_every_pixel_once() {
        let d = draw(
            Frame::solid(2, 2, [100, 150, 200, 255]),
            Quad::full_frame(),
            BlendMode::Add,
            1.0,
        );
        let img = render(&scene_with(vec![d]));
        // With Add mode, a pixel drawn twice along the diagonal would be brighter.
        assert!(
            img.pixels().all(|p| p.0 == [100, 150, 200, 255]),
            "{:?}",
            img.get_pixel(8, 4)
        );
    }

    #[test]
    fn half_quad_covers_left_half_only() {
        let q = Quad {
            corners: [
                Vec2::new(0.0, 0.0),
                Vec2::new(0.5, 0.0),
                Vec2::new(0.5, 1.0),
                Vec2::new(0.0, 1.0),
            ],
        };
        let d = draw(
            Frame::solid(1, 1, [255, 255, 255, 255]),
            q,
            BlendMode::Normal,
            1.0,
        );
        let img = render(&scene_with(vec![d]));
        assert_eq!(img.get_pixel(7, 4).0, [255, 255, 255, 255]);
        assert_eq!(img.get_pixel(8, 4).0, [0, 0, 0, 255]);
    }

    #[test]
    fn blend_modes_and_opacity() {
        let d = [0.5, 0.5, 0.5];
        assert_eq!(
            blend_pixel(BlendMode::Normal, d, [0.5, 0.0, 0.0], 0.5),
            [0.75, 0.25, 0.25]
        );
        assert_eq!(
            blend_pixel(BlendMode::Add, d, [0.5, 0.0, 0.0], 0.5),
            [1.0, 0.5, 0.5]
        );
        assert_eq!(
            blend_pixel(BlendMode::Multiply, d, [0.5, 0.0, 0.0], 0.5),
            [0.5, 0.25, 0.25]
        );
        assert_eq!(
            blend_pixel(BlendMode::Screen, d, [0.5, 0.0, 0.0], 0.5),
            [0.75, 0.5, 0.5]
        );
        assert_eq!(
            blend_pixel(BlendMode::Add, [0.9; 3], [0.9; 3], 1.0),
            [1.0; 3],
            "clamped"
        );

        let base = draw(
            Frame::solid(1, 1, [128, 128, 128, 255]),
            Quad::full_frame(),
            BlendMode::Normal,
            1.0,
        );
        let over = draw(
            Frame::solid(1, 1, [255, 0, 0, 255]),
            Quad::full_frame(),
            BlendMode::Normal,
            0.5,
        );
        let img = render(&scene_with(vec![base.clone(), over]));
        let p = img.get_pixel(3, 3).0;
        assert!(
            (i32::from(p[0]) - 192).abs() <= 1 && (i32::from(p[1]) - 64).abs() <= 1,
            "{p:?}"
        );
        let zero = draw(
            Frame::solid(1, 1, [255, 0, 0, 0]),
            Quad::full_frame(),
            BlendMode::Normal,
            1.0,
        );
        let img = render(&scene_with(vec![base, zero]));
        assert_eq!(
            img.get_pixel(3, 3).0,
            [128, 128, 128, 255],
            "zero alpha source changes nothing"
        );
    }

    #[test]
    fn perspective_correct_sampling_of_a_trapezoid() {
        // Texture: left half black, right half white. Map it to a trapezoid whose right edge is
        // squashed; the u=0.5 boundary must land on the homography's image of x=0.5, not the
        // screen-space midpoint.
        let frame = Frame::from_fn(64, 4, |x, _| {
            if x < 32 {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        });
        let q = Quad {
            corners: [
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.4),
                Vec2::new(1.0, 0.6),
                Vec2::new(0.0, 1.0),
            ],
        };
        let h = q.homography().unwrap();
        let boundary = h.map(Vec2::new(0.5, 0.5)).unwrap().x;
        let d = draw(frame, q, BlendMode::Normal, 1.0);
        let mut scene = scene_with(vec![d]);
        scene.width = 200;
        scene.height = 100;
        let img = render(&scene);
        let row: Vec<u8> = (0..200).map(|x| img.get_pixel(x, 50).0[0]).collect();
        let first_white = row.iter().position(|&v| v > 127).unwrap();
        let expected = (boundary * 200.0).round() as usize;
        assert!(
            (first_white as i64 - expected as i64).abs() <= 1,
            "boundary at {first_white}, expected {expected}"
        );
        assert_ne!(
            expected, 100,
            "the test is only meaningful when perspective moves the boundary"
        );
    }

    #[test]
    fn crop_and_mask_apply() {
        let frame = Frame::from_fn(4, 1, |x, _| [x as u8 * 60, 0, 0, 255]);
        let mut d = draw(frame, Quad::full_frame(), BlendMode::Normal, 1.0);
        d.crop = Crop {
            x: 0.5,
            y: 0.0,
            w: 0.5,
            h: 1.0,
        };
        let img = render(&scene_with(vec![d.clone()]));
        // Cropped source starts half way: pixel 0 samples between texels 1 (60) and 2 (120).
        let left = img.get_pixel(0, 0).0[0];
        assert!(
            left > 80 && left < 120,
            "crop starts half way through the source: {left}"
        );
        assert_eq!(
            img.get_pixel(15, 0).0[0],
            180,
            "crop ends at the last texel"
        );
        let mut m = MaskRaster::full(2);
        m.data = vec![0, 255, 0, 255];
        d.mask = m;
        let img = render(&scene_with(vec![d]));
        assert_eq!(img.get_pixel(0, 0).0[0], 0, "masked out on the left");
        assert!(img.get_pixel(15, 7).0[0] > 100, "visible on the right");
    }

    #[test]
    fn degenerate_triangles_are_skipped() {
        let mut d = draw(
            Frame::solid(1, 1, [255; 4]),
            Quad::full_frame(),
            BlendMode::Normal,
            1.0,
        );
        // Collapse the first triangle (0,1,2): its half of the output stays black.
        d.mesh.positions[1] = d.mesh.positions[0];
        let img = render(&scene_with(vec![d.clone()]));
        assert_eq!(
            img.get_pixel(12, 1).0,
            [0, 0, 0, 255],
            "upper-right half is unfilled"
        );
        assert_eq!(
            img.get_pixel(2, 6).0,
            [255, 255, 255, 255],
            "lower-left half still draws"
        );
        // Zero projective weight on the remaining triangle skips its pixels too.
        for i in [0, 2, 3] {
            d.mesh.texcoords[i][2] = 0.0;
        }
        let img = render(&scene_with(vec![d]));
        assert_eq!(img.get_pixel(2, 6).0, [0, 0, 0, 255]);
    }

    #[test]
    fn fill_rule_edges() {
        let a = Vec2::new(0.0, 0.0);
        let b = Vec2::new(1.0, 0.0);
        assert!(covers(0.1, a, b, 1.0));
        assert!(!covers(-0.1, a, b, 1.0));
        // Horizontal edge going left is a top edge for positive area.
        assert!(covers(0.0, b, a, 1.0));
        assert!(!covers(0.0, a, b, 1.0));
        // Negative area flips orientation.
        assert!(covers(0.0, a, b, -1.0));
        let up = Vec2::new(0.0, -1.0);
        assert!(covers(0.0, a, up, 1.0), "left edge");
    }
}
