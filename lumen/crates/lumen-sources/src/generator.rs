//! Procedural animated sources, rendered on the CPU so the oracle can test them.

use crate::{Frame, FrameSource};
use lumen_core::project::{GeneratorKind, Playback, SourceKind};
use std::sync::Arc;

/// Side length of generated frames.
pub const GENERATOR_SIZE: u32 = 256;

/// Renders a pattern at time `t` (seconds, already scaled by speed) tinted with `color`.
pub fn render(kind: GeneratorKind, t: f64, color: [f32; 4], size: u32) -> Frame {
    let n = size.max(2);
    let tint = |v: f64, x: u32, y: u32| -> [u8; 4] {
        let _ = (x, y);
        let v = v.clamp(0.0, 1.0);
        [
            (v * f64::from(color[0].clamp(0.0, 1.0)) * 255.0).round() as u8,
            (v * f64::from(color[1].clamp(0.0, 1.0)) * 255.0).round() as u8,
            (v * f64::from(color[2].clamp(0.0, 1.0)) * 255.0).round() as u8,
            (f64::from(color[3].clamp(0.0, 1.0)) * 255.0).round() as u8,
        ]
    };
    let fsize = f64::from(n);
    match kind {
        GeneratorKind::Plasma => Frame::from_fn(n, n, |x, y| {
            let (u, v) = (f64::from(x) / fsize, f64::from(y) / fsize);
            let s = (u * 8.0 + t).sin()
                + (v * 6.0 - t * 1.3).sin()
                + ((u + v) * 5.0 + t * 0.7).sin()
                + ((u * u + v * v).sqrt() * 9.0 - t * 2.0).sin();
            tint(s.mul_add(0.125, 0.5), x, y)
        }),
        GeneratorKind::Noise => Frame::from_fn(n, n, |x, y| {
            let (u, v) = (f64::from(x) / fsize * 6.0, f64::from(y) / fsize * 6.0);
            tint(value_noise(u, v, t * 0.8), x, y)
        }),
        GeneratorKind::ColorCycle => Frame::from_fn(n, n, |x, y| {
            let hue = (f64::from(x) / fsize + t * 0.1).rem_euclid(1.0);
            let [r, g, b] = hsv_to_rgb(hue, 1.0, 1.0);
            let a = (f64::from(color[3].clamp(0.0, 1.0)) * 255.0).round() as u8;
            let _ = y;
            [
                (r * f64::from(color[0].clamp(0.0, 1.0)) * 255.0).round() as u8,
                (g * f64::from(color[1].clamp(0.0, 1.0)) * 255.0).round() as u8,
                (b * f64::from(color[2].clamp(0.0, 1.0)) * 255.0).round() as u8,
                a,
            ]
        }),
        GeneratorKind::Strobe => {
            let on = (t * 4.0).fract() < 0.5;
            let px = tint(if on { 1.0 } else { 0.0 }, 0, 0);
            Frame::solid(2, 2, px)
        }
        GeneratorKind::Radar => Frame::from_fn(n, n, |x, y| {
            let (dx, dy) = (f64::from(x) / fsize - 0.5, f64::from(y) / fsize - 0.5);
            let angle = dy.atan2(dx).rem_euclid(std::f64::consts::TAU);
            let sweep = (t * 1.5).rem_euclid(std::f64::consts::TAU);
            let behind = (sweep - angle).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU;
            let r = (dx * dx + dy * dy).sqrt();
            let ring = if (r * 10.0).fract() < 0.08 { 0.25 } else { 0.0 };
            let v = if r > 0.5 {
                0.0
            } else {
                (1.0 - behind).powi(3) * 0.9 + ring
            };
            tint(v, x, y)
        }),
        GeneratorKind::GridPulse => Frame::from_fn(n, n, |x, y| {
            let cell = n / 8;
            let (cx, cy) = (x % cell.max(1), y % cell.max(1));
            let line = cx < 2 || cy < 2;
            let pulse = (t * 2.0).sin().mul_add(0.5, 0.5);
            tint(
                if line {
                    0.6 + 0.4 * pulse
                } else {
                    0.05 * pulse
                },
                x,
                y,
            )
        }),
    }
}

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> [f64; 3] {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match (i as i64).rem_euclid(6) {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

fn hash(x: i64, y: i64, z: i64) -> f64 {
    let mut h = (x.wrapping_mul(374_761_393)
        ^ y.wrapping_mul(668_265_263)
        ^ z.wrapping_mul(2_147_483_647)) as u64;
    h ^= h >> 13;
    h = h.wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h % 10_000) as f64 / 10_000.0
}

fn value_noise(x: f64, y: f64, z: f64) -> f64 {
    let (xi, yi, zi) = (x.floor(), y.floor(), z.floor());
    let (fx, fy, fz) = (x - xi, y - yi, z - zi);
    let smooth = |t: f64| t * t * (3.0 - 2.0 * t);
    let (sx, sy, sz) = (smooth(fx), smooth(fy), smooth(fz));
    let (xi, yi, zi) = (xi as i64, yi as i64, zi as i64);
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let c = |dx: i64, dy: i64, dz: i64| hash(xi + dx, yi + dy, zi + dz);
    let x0 = lerp(
        lerp(c(0, 0, 0), c(1, 0, 0), sx),
        lerp(c(0, 1, 0), c(1, 1, 0), sx),
        sy,
    );
    let x1 = lerp(
        lerp(c(0, 0, 1), c(1, 0, 1), sx),
        lerp(c(0, 1, 1), c(1, 1, 1), sx),
        sy,
    );
    lerp(x0, x1, sz)
}

/// A generator as a frame source.
pub struct GeneratorSource {
    kind: GeneratorKind,
    speed: f32,
    color: [f32; 4],
    size: u32,
    last_t: Option<f64>,
    frame: Option<Arc<Frame>>,
}

impl GeneratorSource {
    /// Creates a generator rendering at [`GENERATOR_SIZE`].
    pub fn new(kind: GeneratorKind, speed: f32, color: [f32; 4]) -> Self {
        Self {
            kind,
            speed,
            color,
            size: GENERATOR_SIZE,
            last_t: None,
            frame: None,
        }
    }

    /// Updates the parameters without resetting time.
    pub fn configure(&mut self, speed: f32, color: [f32; 4]) {
        if (speed, color) != (self.speed, self.color) {
            self.speed = speed;
            self.color = color;
            self.last_t = None;
        }
    }
}

impl FrameSource for GeneratorSource {
    fn frame(&mut self, t: f64) -> Option<Arc<Frame>> {
        if self.last_t != Some(t) || self.frame.is_none() {
            self.frame = Some(Arc::new(render(
                self.kind,
                t * f64::from(self.speed),
                self.color,
                self.size,
            )));
            self.last_t = Some(t);
        }
        self.frame.clone()
    }
    fn is_live(&self) -> bool {
        true
    }
    fn set_playback(&mut self, _playback: &Playback) {}
    fn configure(&mut self, kind: &SourceKind) {
        if let SourceKind::Generator {
            pattern,
            speed,
            color,
        } = kind
        {
            if *pattern != self.kind {
                self.kind = *pattern;
                self.last_t = None;
            }
            self.configure(*speed, *color);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn every_generator_is_deterministic_and_consistent() {
        for kind in GeneratorKind::ALL {
            let a = render(kind, 1.25, [1.0, 0.5, 0.25, 1.0], 32);
            let b = render(kind, 1.25, [1.0, 0.5, 0.25, 1.0], 32);
            assert_eq!(a, b, "{kind:?} is deterministic");
            assert!(a.is_consistent());
            let later = render(kind, 2.13, [1.0, 0.5, 0.25, 1.0], 32);
            assert_ne!(a, later, "{kind:?} animates");
            assert!(a.rgba.chunks_exact(4).all(|p| p[3] == 255));
            let tinted = render(kind, 1.25, [0.0, 1.0, 0.0, 0.5], 32);
            assert!(
                tinted
                    .rgba
                    .chunks_exact(4)
                    .all(|p| p[0] == 0 && p[3] == 128),
                "{kind:?} honours the tint and alpha"
            );
        }
        assert_eq!(
            render(GeneratorKind::Plasma, 0.0, [1.0; 4], 1).width,
            2,
            "size is clamped to 2"
        );
    }

    #[test]
    fn strobe_flashes_and_radar_sweeps() {
        let on = render(GeneratorKind::Strobe, 0.0, [1.0; 4], 4);
        let off = render(GeneratorKind::Strobe, 0.13, [1.0; 4], 4);
        assert_eq!(on.pixel(0, 0)[0], 255);
        assert_eq!(off.pixel(0, 0)[0], 0);
        let r = render(GeneratorKind::Radar, 0.0, [1.0; 4], 64);
        assert_eq!(r.pixel(0, 0), [0, 0, 0, 255], "outside the dish is black");
        assert!(r.pixel(32, 32)[0] > 0);
    }

    #[test]
    fn helpers() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), [1.0, 0.0, 0.0]);
        assert_eq!(hsv_to_rgb(1.0 / 3.0, 1.0, 1.0)[1], 1.0);
        assert_eq!(hsv_to_rgb(2.0 / 3.0, 1.0, 1.0)[2], 1.0);
        for h in [0.1, 0.3, 0.5, 0.7, 0.9] {
            let rgb = hsv_to_rgb(h, 1.0, 1.0);
            assert!(rgb.iter().all(|c| (0.0..=1.0).contains(c)));
        }
        let n = value_noise(0.5, 0.5, 0.5);
        assert!((0.0..=1.0).contains(&n));
        assert_eq!(
            value_noise(1.0, 2.0, 3.0),
            hash(1, 2, 3),
            "lattice points hit the hash"
        );
    }

    #[test]
    fn source_caches_per_time_and_reconfigures() {
        let mut g = GeneratorSource::new(GeneratorKind::Noise, 1.0, [1.0; 4]);
        assert!(g.is_live());
        let a = g.frame(1.0).unwrap();
        let b = g.frame(1.0).unwrap();
        assert!(Arc::ptr_eq(&a, &b), "same time, same frame");
        let c = g.frame(1.5).unwrap();
        assert!(!Arc::ptr_eq(&a, &c));
        g.configure(2.0, [1.0; 4]);
        let d = g.frame(1.5).unwrap();
        assert!(!Arc::ptr_eq(&c, &d), "reconfiguring re-renders");
        g.configure(2.0, [1.0; 4]);
        let e = g.frame(1.5).unwrap();
        assert!(Arc::ptr_eq(&d, &e));
        g.set_playback(&Playback::default());
        FrameSource::configure(
            &mut g,
            &SourceKind::Generator {
                pattern: GeneratorKind::Radar,
                speed: 2.0,
                color: [1.0; 4],
            },
        );
        let f = g.frame(1.5).unwrap();
        assert!(!Arc::ptr_eq(&e, &f), "a new pattern re-renders");
        FrameSource::configure(&mut g, &SourceKind::Solid { color: [0.0; 4] });
        assert!(
            Arc::ptr_eq(&f, &g.frame(1.5).unwrap()),
            "foreign kinds are ignored"
        );
    }
}
