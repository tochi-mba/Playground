//! Small egui textures of source frames for the tree and the inspectors.

use eframe::egui;
use lumen_sources::Frame;
use std::collections::HashMap;
use std::sync::Arc;

/// Longest side of a thumbnail texture.
pub const THUMB_SIZE: u32 = 160;

/// Caches one texture per source, re-uploaded only when the frame changes.
#[derive(Default)]
pub struct Thumbnails {
    textures: HashMap<String, (usize, egui::TextureHandle, (u32, u32))>,
}

impl Thumbnails {
    /// Returns the texture for `frame`, uploading when it is new for this source.
    pub fn get(
        &mut self,
        ctx: &egui::Context,
        id: &str,
        frame: &Arc<Frame>,
    ) -> (egui::TextureId, (u32, u32)) {
        let key = Arc::as_ptr(frame) as usize;
        if let Some((k, tex, size)) = self.textures.get(id) {
            if *k == key {
                return (tex.id(), *size);
            }
        }
        let image = downscale(frame, THUMB_SIZE);
        let size = (image.width() as u32, image.height() as u32);
        let tex = ctx.load_texture(format!("thumb:{id}"), image, egui::TextureOptions::LINEAR);
        let tid = tex.id();
        self.textures.insert(id.to_string(), (key, tex, size));
        (tid, size)
    }

    /// Drops textures for sources that no longer exist.
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.textures.retain(|id, _| keep(id));
    }

    /// Number of cached textures.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.textures.len()
    }

    /// True when nothing is cached.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }
}

/// Nearest-neighbour downscale so the longest side is at most `max`.
pub fn downscale(frame: &Frame, max: u32) -> egui::ColorImage {
    let scale = f64::from(max) / f64::from(frame.width.max(frame.height).max(1));
    let (w, h) = if scale >= 1.0 {
        (frame.width, frame.height)
    } else {
        (
            (f64::from(frame.width) * scale).round().max(1.0) as u32,
            (f64::from(frame.height) * scale).round().max(1.0) as u32,
        )
    };
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let sx = i64::from(x) * i64::from(frame.width) / i64::from(w);
            let sy = i64::from(y) * i64::from(frame.height) / i64::from(h);
            let p = frame.pixel(sx, sy);
            pixels.push(egui::Color32::from_rgba_unmultiplied(
                p[0], p[1], p[2], p[3],
            ));
        }
    }
    egui::ColorImage {
        size: [w as usize, h as usize],
        source_size: egui::Vec2::new(w as f32, h as f32),
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downscale_limits_the_longest_side() {
        let f = Frame::from_fn(400, 100, |x, _| [(x / 2) as u8, 0, 0, 255]);
        let img = downscale(&f, 100);
        assert_eq!(img.size, [100, 25]);
        let small = Frame::solid(10, 5, [1, 2, 3, 4]);
        let img = downscale(&small, 100);
        assert_eq!(img.size, [10, 5]);
        assert_eq!(
            img.pixels[0],
            egui::Color32::from_rgba_unmultiplied(1, 2, 3, 4)
        );
        let tall = Frame::solid(1, 1000, [0; 4]);
        assert_eq!(downscale(&tall, 50).size, [1, 50]);
    }

    #[test]
    fn cache_uploads_once_per_frame() {
        let ctx = egui::Context::default();
        let mut t = Thumbnails::default();
        assert!(t.is_empty());
        let a = Arc::new(Frame::solid(4, 4, [9; 4]));
        let (id1, size) = t.get(&ctx, "s", &a);
        assert_eq!(size, (4, 4));
        let (id2, _) = t.get(&ctx, "s", &a);
        assert_eq!(id1, id2, "same frame, same texture");
        let b = Arc::new(Frame::solid(8, 4, [1; 4]));
        let (id3, size3) = t.get(&ctx, "s", &b);
        assert_ne!(id1, id3);
        assert_eq!(size3, (8, 4));
        assert_eq!(t.len(), 1);
        t.get(&ctx, "other", &a);
        t.retain(|id| id == "s");
        assert_eq!(t.len(), 1);
    }
}
