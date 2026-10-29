//! Frame sources: where the pixels a surface samples come from.
//!
//! A [`Frame`] is a plain RGBA8 bitmap. Sources produce frames; the renderers consume them.
//! Keeping this crate GPU-free means every source, including the Windows-only Spout receiver,
//! has a mockable, testable interface.

#![forbid(unsafe_code)]

pub mod pattern;
#[cfg(feature = "spout")]
pub mod spout;

use lumen_core::project::{Project, Source, SourceKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// An RGBA8 bitmap, straight (non-premultiplied) alpha, rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl Frame {
    /// Creates a frame filled with one colour.
    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let (w, h) = (width.max(1), height.max(1));
        Self {
            width: w,
            height: h,
            rgba: rgba.repeat((w * h) as usize),
        }
    }

    /// Builds a frame from a per-pixel function.
    pub fn from_fn(width: u32, height: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Self {
        let (w, h) = (width.max(1), height.max(1));
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&f(x, y));
            }
        }
        Self {
            width: w,
            height: h,
            rgba,
        }
    }

    /// Pixel at integer coordinates, clamped to the edge.
    pub fn pixel(&self, x: i64, y: i64) -> [u8; 4] {
        let x = x.clamp(0, i64::from(self.width) - 1) as usize;
        let y = y.clamp(0, i64::from(self.height) - 1) as usize;
        let i = (y * self.width as usize + x) * 4;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    /// Bilinear sample at normalised `(u, v)`, clamped to the edge, returning `[0,1]` floats.
    pub fn sample_bilinear(&self, u: f64, v: f64) -> [f64; 4] {
        let fx = u.clamp(0.0, 1.0) * f64::from(self.width) - 0.5;
        let fy = v.clamp(0.0, 1.0) * f64::from(self.height) - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = fx - x0;
        let ty = fy - y0;
        let (x0, y0) = (x0 as i64, y0 as i64);
        let p00 = self.pixel(x0, y0);
        let p10 = self.pixel(x0 + 1, y0);
        let p01 = self.pixel(x0, y0 + 1);
        let p11 = self.pixel(x0 + 1, y0 + 1);
        let mut out = [0.0; 4];
        for c in 0..4 {
            let top = f64::from(p00[c]) * (1.0 - tx) + f64::from(p10[c]) * tx;
            let bottom = f64::from(p01[c]) * (1.0 - tx) + f64::from(p11[c]) * tx;
            out[c] = (top * (1.0 - ty) + bottom * ty) / 255.0;
        }
        out
    }

    /// True when dimensions and buffer length agree.
    pub fn is_consistent(&self) -> bool {
        self.rgba.len() == (self.width as usize) * (self.height as usize) * 4
    }

    /// Loads a PNG or JPEG from disk.
    pub fn load(path: &Path) -> Result<Self, SourceError> {
        let img =
            image::open(path).map_err(|e| SourceError::Image(path.to_path_buf(), e.to_string()))?;
        let rgba = img.to_rgba8();
        Ok(Self {
            width: rgba.width(),
            height: rgba.height(),
            rgba: rgba.into_raw(),
        })
    }
}

/// Errors from resolving sources.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// An image could not be read or decoded.
    #[error("cannot load image {0}: {1}")]
    Image(PathBuf, String),
    /// The source kind is not available on this platform or build.
    #[error("source {0:?} needs the {1} feature, which this build lacks")]
    Unavailable(String, &'static str),
}

/// Anything that can produce frames over time.
pub trait FrameSource: Send {
    /// Latest frame, or `None` while nothing is available yet (a Spout sender that has not
    /// started, for instance).
    fn frame(&mut self) -> Option<Arc<Frame>>;
    /// True when the source changes between calls, so the renderer should re-upload.
    fn is_live(&self) -> bool {
        false
    }
}

/// A source that always returns the same frame.
pub struct StaticSource(pub Arc<Frame>);

impl FrameSource for StaticSource {
    fn frame(&mut self) -> Option<Arc<Frame>> {
        Some(Arc::clone(&self.0))
    }
}

/// Resolution used for generated patterns and solids.
pub const PATTERN_SIZE: (u32, u32) = (1024, 1024);

/// Builds the frame source for one project source.
///
/// `base_dir` resolves relative image paths; it should be the project file's folder.
pub fn open_source(source: &Source, base_dir: &Path) -> Result<Box<dyn FrameSource>, SourceError> {
    let (w, h) = PATTERN_SIZE;
    match &source.kind {
        SourceKind::TestPattern { pattern } => Ok(Box::new(StaticSource(Arc::new(
            pattern::generate(*pattern, w, h),
        )))),
        SourceKind::Solid { color } => {
            let c = color.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
            Ok(Box::new(StaticSource(Arc::new(Frame::solid(16, 16, c)))))
        }
        SourceKind::Image { path } => {
            let p = Path::new(path);
            let full = if p.is_absolute() {
                p.to_path_buf()
            } else {
                base_dir.join(p)
            };
            Ok(Box::new(StaticSource(Arc::new(Frame::load(&full)?))))
        }
        SourceKind::Spout { sender } => open_spout(&source.id, sender),
    }
}

#[cfg(feature = "spout")]
#[allow(clippy::unnecessary_wraps)]
fn open_spout(_id: &str, sender: &str) -> Result<Box<dyn FrameSource>, SourceError> {
    Ok(Box::new(spout::SpoutReceiver::new(sender)))
}

#[cfg(not(feature = "spout"))]
fn open_spout(id: &str, _sender: &str) -> Result<Box<dyn FrameSource>, SourceError> {
    Err(SourceError::Unavailable(id.to_string(), "spout"))
}

/// All of a project's sources, opened and ready to be polled each frame.
pub struct SourceSet {
    sources: HashMap<String, Box<dyn FrameSource>>,
    cache: HashMap<String, Arc<Frame>>,
}

impl SourceSet {
    /// Opens every source in the project. Sources that fail to open are reported and
    /// replaced with a magenta "missing" frame so the show keeps running.
    pub fn open(project: &Project, base_dir: &Path) -> (Self, Vec<SourceError>) {
        let mut sources: HashMap<String, Box<dyn FrameSource>> = HashMap::new();
        let mut errors = Vec::new();
        for s in &project.sources {
            match open_source(s, base_dir) {
                Ok(src) => {
                    sources.insert(s.id.clone(), src);
                }
                Err(e) => {
                    log::warn!("source {:?}: {e}", s.id);
                    errors.push(e);
                    sources.insert(
                        s.id.clone(),
                        Box::new(StaticSource(Arc::new(missing_frame()))),
                    );
                }
            }
        }
        (
            Self {
                sources,
                cache: HashMap::new(),
            },
            errors,
        )
    }

    /// Polls every source once; call at the start of each rendered frame.
    pub fn poll(&mut self) {
        for (id, src) in &mut self.sources {
            if let Some(f) = src.frame() {
                self.cache.insert(id.clone(), f);
            }
        }
    }

    /// Latest frame for a source id, if it has produced one.
    pub fn frame(&self, id: &str) -> Option<&Arc<Frame>> {
        self.cache.get(id)
    }

    /// Ids of sources that change over time.
    pub fn live_ids(&self) -> Vec<String> {
        self.sources
            .iter()
            .filter(|(_, s)| s.is_live())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Replaces or adds a source at runtime (used by the editor when a source is edited).
    pub fn insert(&mut self, id: String, source: Box<dyn FrameSource>) {
        self.cache.remove(&id);
        self.sources.insert(id, source);
    }
}

/// The frame shown for a source that failed to open: an unmistakable magenta/black checker.
pub fn missing_frame() -> Frame {
    Frame::from_fn(64, 64, |x, y| {
        if ((x / 8) + (y / 8)) % 2 == 0 {
            [255, 0, 255, 255]
        } else {
            [0, 0, 0, 255]
        }
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used)]
mod tests {
    use super::*;
    use lumen_core::project::TestPattern;

    #[test]
    fn solid_and_from_fn() {
        let f = Frame::solid(2, 2, [1, 2, 3, 4]);
        assert!(f.is_consistent());
        assert_eq!(f.pixel(1, 1), [1, 2, 3, 4]);
        assert_eq!(f.pixel(-5, 99), [1, 2, 3, 4], "clamped");
        let g = Frame::from_fn(3, 1, |x, _| [x as u8, 0, 0, 255]);
        assert_eq!(g.pixel(2, 0), [2, 0, 0, 255]);
        assert_eq!(Frame::solid(0, 0, [0; 4]).width, 1);
        let mut bad = Frame::solid(2, 2, [0; 4]);
        bad.rgba.pop();
        assert!(!bad.is_consistent());
    }

    #[test]
    fn bilinear_sampling_interpolates_and_clamps() {
        let f = Frame::from_fn(2, 1, |x, _| {
            if x == 0 {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        });
        let mid = f.sample_bilinear(0.5, 0.5);
        assert!((mid[0] - 0.5).abs() < 1e-9);
        assert_eq!(f.sample_bilinear(0.0, 0.5)[0], 0.0);
        assert_eq!(f.sample_bilinear(1.0, 0.5)[0], 1.0);
        assert_eq!(f.sample_bilinear(-3.0, 9.0)[0], 0.0);
        assert_eq!(
            f.sample_bilinear(0.25, 0.5)[0],
            0.0,
            "pixel centre of the first texel"
        );
        assert!((f.sample_bilinear(0.375, 0.5)[0] - 0.25).abs() < 1e-9);
    }

    #[test]
    fn image_round_trip_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("img.png");
        let img = image::RgbaImage::from_fn(4, 3, |x, y| {
            image::Rgba([x as u8 * 10, y as u8 * 10, 7, 255])
        });
        img.save(&path).unwrap();
        let f = Frame::load(&path).unwrap();
        assert_eq!((f.width, f.height), (4, 3));
        assert_eq!(f.pixel(3, 2), [30, 20, 7, 255]);
        let err = Frame::load(&dir.path().join("nope.png")).unwrap_err();
        assert!(err.to_string().contains("cannot load image"));
        std::fs::write(dir.path().join("junk.png"), b"not a png").unwrap();
        assert!(Frame::load(&dir.path().join("junk.png")).is_err());

        // open_source resolves relative paths against base_dir.
        let src = Source {
            id: "i".into(),
            kind: SourceKind::Image {
                path: "img.png".into(),
            },
        };
        let mut s = open_source(&src, dir.path()).unwrap();
        assert_eq!(s.frame().unwrap().width, 4);
        assert!(!s.is_live());
        let abs = Source {
            id: "i".into(),
            kind: SourceKind::Image {
                path: path.to_string_lossy().into(),
            },
        };
        assert!(open_source(&abs, Path::new("/elsewhere")).is_ok());
    }

    #[test]
    fn open_source_kinds() {
        let pat = Source {
            id: "p".into(),
            kind: SourceKind::TestPattern {
                pattern: TestPattern::Grid,
            },
        };
        let mut s = open_source(&pat, Path::new(".")).unwrap();
        assert_eq!(s.frame().unwrap().width, PATTERN_SIZE.0);
        let solid = Source {
            id: "s".into(),
            kind: SourceKind::Solid {
                color: [1.0, 0.5, 0.0, 2.0],
            },
        };
        let mut s = open_source(&solid, Path::new(".")).unwrap();
        assert_eq!(s.frame().unwrap().pixel(0, 0), [255, 128, 0, 255]);
        let spout = Source {
            id: "sp".into(),
            kind: SourceKind::Spout {
                sender: String::new(),
            },
        };
        let r = open_source(&spout, Path::new("."));
        if cfg!(feature = "spout") {
            assert!(r.is_ok());
        } else {
            let e = r.err().unwrap();
            assert!(matches!(e, SourceError::Unavailable(_, "spout")));
            assert!(e.to_string().contains("spout"));
        }
    }

    #[test]
    fn source_set_falls_back_and_polls() {
        let mut p = Project::default();
        p.sources.push(Source {
            id: "ok".into(),
            kind: SourceKind::Solid {
                color: [0.0, 0.0, 1.0, 1.0],
            },
        });
        p.sources.push(Source {
            id: "bad".into(),
            kind: SourceKind::Image {
                path: "missing.png".into(),
            },
        });
        let (mut set, errors) = SourceSet::open(&p, Path::new("."));
        assert_eq!(errors.len(), 1);
        assert!(set.frame("ok").is_none(), "nothing cached before poll");
        set.poll();
        assert_eq!(set.frame("ok").unwrap().pixel(0, 0), [0, 0, 255, 255]);
        assert_eq!(
            set.frame("bad").unwrap().pixel(0, 0),
            [255, 0, 255, 255],
            "magenta fallback"
        );
        assert!(set.frame("ghost").is_none());
        assert!(set.live_ids().is_empty());
        set.insert(
            "ok".into(),
            Box::new(StaticSource(Arc::new(Frame::solid(1, 1, [9; 4])))),
        );
        assert!(set.frame("ok").is_none(), "cache cleared on insert");
        set.poll();
        assert_eq!(set.frame("ok").unwrap().pixel(0, 0), [9; 4]);
        assert!(missing_frame().is_consistent());
    }
}
