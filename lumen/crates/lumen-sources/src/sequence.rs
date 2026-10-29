//! Image sequences: a folder of numbered frames, or an animated GIF.

use crate::clock::Transport;
use crate::{Frame, FrameSource, SourceError};
use image::AnimationDecoder;
use lumen_core::project::Playback;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Most frames loaded from a folder; the rest are ignored with a warning.
pub const MAX_FRAMES: usize = 2000;
/// Frames wider than this are scaled down on load.
pub const MAX_WIDTH: u32 = 1920;

/// Frames plus the time each one starts at.
#[derive(Debug, Clone)]
pub struct Clip {
    /// Frames in order.
    pub frames: Vec<Arc<Frame>>,
    /// Start time of each frame in seconds; same length as `frames`.
    pub starts: Vec<f64>,
    /// Total duration in seconds.
    pub duration: f64,
}

impl Clip {
    /// Frames at a constant rate.
    pub fn constant_rate(frames: Vec<Arc<Frame>>, fps: f64) -> Self {
        let fps = if fps.is_finite() && fps > 0.0 {
            fps
        } else {
            25.0
        };
        let starts = (0..frames.len()).map(|i| i as f64 / fps).collect();
        let duration = frames.len() as f64 / fps;
        Self {
            frames,
            starts,
            duration,
        }
    }

    /// Frames with explicit per-frame delays in seconds.
    pub fn with_delays(frames: Vec<Arc<Frame>>, delays: &[f64]) -> Self {
        let mut starts = Vec::with_capacity(frames.len());
        let mut t = 0.0;
        for (i, _) in frames.iter().enumerate() {
            starts.push(t);
            t += delays.get(i).copied().unwrap_or(0.1).max(0.001);
        }
        Self {
            frames,
            starts,
            duration: t,
        }
    }

    /// Index of the frame showing at `position` seconds.
    pub fn index_at(&self, position: f64) -> Option<usize> {
        if self.frames.is_empty() {
            return None;
        }
        let i = self.starts.partition_point(|s| *s <= position);
        Some(i.saturating_sub(1).min(self.frames.len() - 1))
    }

    /// Loads a GIF with its own frame timing.
    pub fn load_gif(path: &Path) -> Result<Self, SourceError> {
        let file = std::fs::File::open(path)
            .map_err(|e| SourceError::Image(path.to_path_buf(), e.to_string()))?;
        let decoder = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(file))
            .map_err(|e| SourceError::Image(path.to_path_buf(), e.to_string()))?;
        let mut frames = Vec::new();
        let mut delays = Vec::new();
        for f in decoder.into_frames() {
            let f = f.map_err(|e| SourceError::Image(path.to_path_buf(), e.to_string()))?;
            let (num, den) = f.delay().numer_denom_ms();
            delays.push(f64::from(num) / f64::from(den.max(1)) / 1000.0);
            let buf = f.into_buffer();
            frames.push(Arc::new(fit_width(Frame {
                width: buf.width(),
                height: buf.height(),
                rgba: buf.into_raw(),
            })));
            if frames.len() >= MAX_FRAMES {
                break;
            }
        }
        if frames.is_empty() {
            return Err(SourceError::Image(
                path.to_path_buf(),
                "GIF has no frames".into(),
            ));
        }
        Ok(Self::with_delays(frames, &delays))
    }

    /// Loads every PNG/JPEG in a folder, sorted by name, at a constant rate.
    pub fn load_folder(dir: &Path, fps: f64) -> Result<Self, SourceError> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| SourceError::Image(dir.to_path_buf(), e.to_string()))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|e| {
                    matches!(
                        e.to_string_lossy().to_ascii_lowercase().as_str(),
                        "png" | "jpg" | "jpeg"
                    )
                })
            })
            .collect();
        files.sort();
        if files.len() > MAX_FRAMES {
            log::warn!(
                "{}: only the first {MAX_FRAMES} of {} frames are used",
                dir.display(),
                files.len()
            );
            files.truncate(MAX_FRAMES);
        }
        let mut frames = Vec::with_capacity(files.len());
        for f in &files {
            frames.push(Arc::new(fit_width(Frame::load(f)?)));
        }
        if frames.is_empty() {
            return Err(SourceError::Image(
                dir.to_path_buf(),
                "folder has no PNG or JPEG frames".into(),
            ));
        }
        Ok(Self::constant_rate(frames, fps))
    }
}

/// Scales a frame down to at most [`MAX_WIDTH`] wide (nearest neighbour; frames this large
/// are already far above projector resolution).
pub fn fit_width(frame: Frame) -> Frame {
    if frame.width <= MAX_WIDTH {
        return frame;
    }
    let w = MAX_WIDTH;
    let h = (u64::from(frame.height) * u64::from(w) / u64::from(frame.width)).max(1) as u32;
    Frame::from_fn(w, h, |x, y| {
        let sx = i64::from(x) * i64::from(frame.width) / i64::from(w);
        let sy = i64::from(y) * i64::from(frame.height) / i64::from(h);
        frame.pixel(sx, sy)
    })
}

/// A clip played through a transport.
pub struct SequenceSource {
    clip: Clip,
    transport: Transport,
}

impl SequenceSource {
    /// Wraps a loaded clip.
    pub fn new(clip: Clip, playback: Playback) -> Self {
        Self {
            clip,
            transport: Transport::new(playback),
        }
    }

    /// Opens a folder or a `.gif`.
    pub fn open(path: &Path, fps: f64, playback: Playback) -> Result<Self, SourceError> {
        let clip = if path.is_dir() {
            Clip::load_folder(path, fps)?
        } else {
            Clip::load_gif(path)?
        };
        Ok(Self::new(clip, playback))
    }

    /// Current playhead in seconds.
    pub fn position(&self) -> f64 {
        self.transport.position
    }
}

impl FrameSource for SequenceSource {
    fn frame(&mut self, t: f64) -> Option<Arc<Frame>> {
        self.transport.advance(t);
        self.transport.wrap(self.clip.duration);
        self.clip
            .index_at(self.transport.position)
            .map(|i| Arc::clone(&self.clip.frames[i]))
    }
    fn is_live(&self) -> bool {
        self.clip.frames.len() > 1
    }
    fn set_playback(&mut self, playback: &Playback) {
        self.transport.set_playback(*playback);
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::field_reassign_with_default
)]
mod tests {
    use super::*;

    fn frame(v: u8) -> Arc<Frame> {
        Arc::new(Frame::solid(2, 2, [v, v, v, 255]))
    }

    #[test]
    fn clip_timing() {
        let c = Clip::constant_rate(vec![frame(0), frame(1), frame(2)], 10.0);
        assert_eq!(c.duration, 0.3);
        assert_eq!(c.index_at(0.0), Some(0));
        assert_eq!(c.index_at(0.15), Some(1));
        assert_eq!(c.index_at(0.29), Some(2));
        assert_eq!(c.index_at(5.0), Some(2));
        assert_eq!(
            Clip::constant_rate(vec![frame(0)], 0.0).duration,
            1.0 / 25.0,
            "bad fps falls back"
        );
        assert_eq!(Clip::constant_rate(vec![], 10.0).index_at(0.0), None);
        let d = Clip::with_delays(vec![frame(0), frame(1)], &[0.5]);
        assert_eq!(d.starts, [0.0, 0.5]);
        assert!(
            (d.duration - 0.6).abs() < 1e-9,
            "missing delays default to 0.1 s"
        );
        assert_eq!(d.index_at(0.49), Some(0));
        assert_eq!(d.index_at(0.5), Some(1));
    }

    #[test]
    fn folder_and_gif_sources() {
        let dir = tempfile::tempdir().unwrap();
        let seq = dir.path().join("seq");
        std::fs::create_dir(&seq).unwrap();
        for i in 0..3 {
            image::RgbaImage::from_pixel(4, 2, image::Rgba([i * 50, 0, 0, 255]))
                .save(seq.join(format!("frame_{i:03}.png")))
                .unwrap();
        }
        std::fs::write(seq.join("notes.txt"), "ignored").unwrap();
        let mut s = SequenceSource::open(&seq, 10.0, Playback::default()).unwrap();
        assert!(s.is_live());
        assert_eq!(s.frame(0.0).unwrap().pixel(0, 0)[0], 0);
        assert_eq!(s.frame(0.15).unwrap().pixel(0, 0)[0], 50);
        assert_eq!(s.frame(0.35).unwrap().pixel(0, 0)[0], 0, "looped");
        let mut once = Playback::default();
        once.looping = false;
        s.set_playback(&once);
        assert_eq!(
            s.frame(5.0).unwrap().pixel(0, 0)[0],
            100,
            "holds the last frame"
        );
        once.restart_token = 1;
        s.set_playback(&once);
        assert_eq!(s.position(), 0.0);

        // GIF with two frames.
        let gif = dir.path().join("anim.gif");
        {
            let file = std::fs::File::create(&gif).unwrap();
            let mut enc = image::codecs::gif::GifEncoder::new(file);
            enc.set_repeat(image::codecs::gif::Repeat::Infinite)
                .unwrap();
            for v in [10u8, 200u8] {
                let img = image::RgbaImage::from_pixel(3, 3, image::Rgba([v, v, v, 255]));
                let f =
                    image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(200, 1));
                enc.encode_frame(f).unwrap();
            }
        }
        let mut g = SequenceSource::open(&gif, 25.0, Playback::default()).unwrap();
        assert_eq!(g.frame(0.0).unwrap().pixel(0, 0)[0], 10);
        assert_eq!(g.frame(0.25).unwrap().pixel(0, 0)[0], 200);

        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert!(SequenceSource::open(&empty, 25.0, Playback::default()).is_err());
        assert!(
            SequenceSource::open(&dir.path().join("missing"), 25.0, Playback::default()).is_err()
        );
        std::fs::write(dir.path().join("bad.gif"), b"GIF89a junk").unwrap();
        assert!(
            SequenceSource::open(&dir.path().join("bad.gif"), 25.0, Playback::default()).is_err()
        );
        let single = SequenceSource::new(
            Clip::constant_rate(vec![frame(1)], 1.0),
            Playback::default(),
        );
        assert!(!single.is_live());
    }

    #[test]
    fn oversized_frames_are_scaled_down() {
        let big = Frame::from_fn(4000, 1000, |x, _| [(x % 256) as u8, 0, 0, 255]);
        let small = fit_width(big);
        assert_eq!((small.width, small.height), (1920, 480));
        assert_eq!(fit_width(Frame::solid(10, 10, [0; 4])).width, 10);
    }
}
