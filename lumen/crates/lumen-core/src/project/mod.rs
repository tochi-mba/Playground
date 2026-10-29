//! The project file: everything a show needs, as plain, diffable TOML.
//!
//! A project is a set of **sources** (where pixels come from), **outputs** (physical
//! projectors with their blend and colour settings) and **surfaces** (a warp that places a
//! source onto an output). Files carry a `schema_version`; [`Project::load`] migrates older
//! versions forward so a show saved years ago still opens.

use crate::blend::EdgeBlend;
use crate::mask::Mask;
use crate::params::ParamValue;
use crate::warp::WarpKind;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

/// The schema version written by this build.
pub const SCHEMA_VERSION: u32 = 1;

/// Errors from loading, saving or editing a project.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    /// File system error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// TOML could not be parsed or did not match the schema.
    #[error("parse error: {0}")]
    Parse(String),
    /// The file was written by a newer lumen.
    #[error("project schema {found} is newer than this build supports ({supported})")]
    TooNew {
        /// Version in the file.
        found: u32,
        /// Version this build writes.
        supported: u32,
    },
    /// A parameter address did not resolve.
    #[error("unknown parameter {0}")]
    UnknownParam(String),
    /// A parameter received a value of the wrong type.
    #[error("parameter {0} cannot take value {1:?}")]
    BadValue(String, ParamValue),
}

/// Where a surface's pixels come from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceKind {
    /// Built-in calibration pattern.
    TestPattern {
        /// Pattern name.
        pattern: TestPattern,
    },
    /// Flat colour, RGBA in `[0,1]`.
    Solid {
        /// Colour.
        color: [f32; 4],
    },
    /// Still image file, relative to the project file when not absolute.
    Image {
        /// Path to a PNG or JPEG.
        path: String,
    },
    /// Live texture from a Spout sender (Windows only).
    Spout {
        /// Sender name; empty means the active sender.
        #[serde(default)]
        sender: String,
    },
    /// A video file decoded through `ffmpeg`.
    Video {
        /// Path, relative to the project file when not absolute.
        path: String,
        /// Transport settings.
        #[serde(default)]
        playback: Playback,
    },
    /// A folder of numbered image frames, or an animated GIF.
    Sequence {
        /// Folder or `.gif` path, relative to the project file when not absolute.
        path: String,
        /// Frames per second for folders (GIFs carry their own timing).
        #[serde(default = "default_fps")]
        fps: f64,
        /// Transport settings.
        #[serde(default)]
        playback: Playback,
    },
    /// A procedural animated pattern.
    Generator {
        /// Which pattern.
        pattern: GeneratorKind,
        /// Animation speed multiplier.
        #[serde(default = "default_one_f32")]
        speed: f32,
        /// Tint colour, RGBA in `[0,1]`.
        #[serde(default = "default_white")]
        color: [f32; 4],
    },
}

fn default_fps() -> f64 {
    25.0
}

fn default_one_f32() -> f32 {
    1.0
}

fn default_white() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}

/// Transport state of a playable source.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Playback {
    /// Start again at the end.
    pub looping: bool,
    /// Rate multiplier, `1` is normal speed.
    pub speed: f64,
    /// Frozen on the current frame.
    pub paused: bool,
    /// Bumped by a restart request; never saved.
    #[serde(skip)]
    pub restart_token: u32,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            looping: true,
            speed: 1.0,
            paused: false,
            restart_token: 0,
        }
    }
}

/// Procedural patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneratorKind {
    /// Classic sine plasma.
    Plasma,
    /// Smooth value noise.
    Noise,
    /// Hue sweep across the frame.
    ColorCycle,
    /// Full-frame flashes.
    Strobe,
    /// Rotating radar sweep.
    Radar,
    /// Grid whose lines pulse.
    GridPulse,
}

impl GeneratorKind {
    /// Every kind, for menus.
    pub const ALL: [Self; 6] = [
        Self::Plasma,
        Self::Noise,
        Self::ColorCycle,
        Self::Strobe,
        Self::Radar,
        Self::GridPulse,
    ];
}

impl SourceKind {
    /// Transport settings, for the kinds that play.
    pub fn playback(&self) -> Option<&Playback> {
        match self {
            Self::Video { playback, .. } | Self::Sequence { playback, .. } => Some(playback),
            _ => None,
        }
    }

    /// Mutable transport settings.
    pub fn playback_mut(&mut self) -> Option<&mut Playback> {
        match self {
            Self::Video { playback, .. } | Self::Sequence { playback, .. } => Some(playback),
            _ => None,
        }
    }

    /// The file or folder path, for file-backed kinds.
    pub fn path(&self) -> Option<&str> {
        match self {
            Self::Image { path } | Self::Video { path, .. } | Self::Sequence { path, .. } => {
                Some(path)
            }
            _ => None,
        }
    }

    /// Short human label for the kind.
    pub fn label(&self) -> &'static str {
        match self {
            Self::TestPattern { .. } => "pattern",
            Self::Solid { .. } => "solid",
            Self::Image { .. } => "image",
            Self::Spout { .. } => "spout",
            Self::Video { .. } => "video",
            Self::Sequence { .. } => "sequence",
            Self::Generator { .. } => "generator",
        }
    }
}

/// Built-in test patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestPattern {
    /// Alignment grid with a centre crosshair.
    Grid,
    /// SMPTE-style colour bars.
    Bars,
    /// Checkerboard.
    Checker,
    /// Linear grey ramp, left to right.
    Ramp,
}

/// A named pixel source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    /// Unique id, used by surfaces and parameter addresses.
    pub id: String,
    /// What it is.
    #[serde(flatten)]
    pub kind: SourceKind,
}

/// Per-output colour adjustments applied after blending.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColorAdjust {
    /// Multiplier, `1` is neutral.
    pub brightness: f64,
    /// Contrast around mid-grey, `1` is neutral.
    pub contrast: f64,
    /// Output gamma correction, `1` is neutral.
    pub gamma: f64,
}

impl Default for ColorAdjust {
    fn default() -> Self {
        Self {
            brightness: 1.0,
            contrast: 1.0,
            gamma: 1.0,
        }
    }
}

/// Where an output window should live.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    /// Monitor name as reported by the OS, e.g. `\\.\DISPLAY2`.
    #[serde(default)]
    pub monitor: String,
    /// Window origin in virtual-desktop pixels.
    #[serde(default)]
    pub x: i32,
    /// Window origin in virtual-desktop pixels.
    #[serde(default)]
    pub y: i32,
}

/// A physical projector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Output {
    /// Unique id.
    pub id: String,
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// Enabled outputs open a window in show mode.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Edge blend.
    #[serde(default = "EdgeBlend::none")]
    pub blend: EdgeBlend,
    /// Colour adjustments.
    #[serde(default)]
    pub color: ColorAdjust,
    /// Window placement; `None` means "ask at first run".
    #[serde(default)]
    pub placement: Option<Placement>,
    /// Calibration overlay drawn on top of the content (never in show mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay: Option<Overlay>,
}

/// A calibration pattern drawn over an output while aligning projectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Overlay {
    /// Fine alignment grid with a centre cross; content shows through the cells.
    Grid,
    /// Centre crosshair with diagonals, for aiming two projectors at one point.
    Crosshair,
    /// Colour bars, for colour matching.
    Bars,
    /// Eleven grey steps, for gamma matching across a seam.
    GreySteps,
    /// Flat white, so any visible seam is the blend's fault.
    BlendCheck,
}

impl Overlay {
    /// Every overlay, in menu order.
    pub const ALL: [Self; 5] = [
        Self::Grid,
        Self::Crosshair,
        Self::Bars,
        Self::GreySteps,
        Self::BlendCheck,
    ];

    /// Menu label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Grid => "Grid",
            Self::Crosshair => "Crosshair",
            Self::Bars => "Colour bars",
            Self::GreySteps => "Grey steps",
            Self::BlendCheck => "Blend check (white)",
        }
    }
}

fn default_true() -> bool {
    true
}

/// How a surface composites over what is already on its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    /// Source over.
    #[default]
    Normal,
    /// Additive.
    Add,
    /// Multiply.
    Multiply,
    /// Screen.
    Screen,
}

/// A region of the source to sample, in normalised source coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Crop {
    /// Left.
    pub x: f64,
    /// Top.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

impl Default for Crop {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        }
    }
}

/// A source placed on an output through a warp.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Surface {
    /// Unique id.
    pub id: String,
    /// Output this surface draws on.
    pub output: String,
    /// Source it samples.
    pub source: String,
    /// Draw order within the output; lower draws first.
    #[serde(default)]
    pub z: i32,
    /// Shown at all.
    #[serde(default = "default_true")]
    pub visible: bool,
    /// Opacity in `[0,1]`.
    #[serde(default = "default_one")]
    pub opacity: f64,
    /// Compositing mode.
    #[serde(default)]
    pub blend_mode: BlendMode,
    /// Which part of the source to use.
    #[serde(default)]
    pub crop: Crop,
    /// Geometry.
    #[serde(default)]
    pub warp: WarpKind,
    /// Masks in surface space.
    #[serde(default)]
    pub masks: Vec<Mask>,
    /// Locked surfaces show no handles and ignore drags, so a finished calibration stays put.
    #[serde(default)]
    pub locked: bool,
}

fn default_one() -> f64 {
    1.0
}

/// A complete show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// File format version.
    pub schema_version: u32,
    /// Human name.
    #[serde(default)]
    pub name: String,
    /// Global blackout: every output renders black when true.
    #[serde(default)]
    pub blackout: bool,
    /// Master brightness multiplier applied to all outputs.
    #[serde(default = "default_one")]
    pub master: f64,
    /// Sources.
    #[serde(default)]
    pub sources: Vec<Source>,
    /// Outputs.
    #[serde(default)]
    pub outputs: Vec<Output>,
    /// Surfaces.
    #[serde(default)]
    pub surfaces: Vec<Surface>,
    /// Cues, in GO order.
    #[serde(default)]
    pub cues: Vec<crate::cues::Cue>,
    /// MIDI controller mappings.
    #[serde(default)]
    pub midi: crate::midimap::MidiMap,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            name: String::new(),
            blackout: false,
            master: 1.0,
            sources: Vec::new(),
            outputs: Vec::new(),
            surfaces: Vec::new(),
            cues: Vec::new(),
            midi: crate::midimap::MidiMap::default(),
        }
    }
}

/// A validation finding. Errors make a project unrenderable; warnings are advisory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// True for errors.
    pub error: bool,
    /// Human description.
    pub message: String,
}

impl Issue {
    fn err(message: impl Into<String>) -> Self {
        Self {
            error: true,
            message: message.into(),
        }
    }
    fn warn(message: impl Into<String>) -> Self {
        Self {
            error: false,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {}",
            if self.error { "error" } else { "warning" },
            self.message
        )
    }
}

impl Project {
    /// Parses TOML text, migrating older schemas.
    pub fn from_toml(text: &str) -> Result<Self, ProjectError> {
        let mut doc: toml::Table =
            toml::from_str(text).map_err(|e| ProjectError::Parse(e.to_string()))?;
        migrate(&mut doc)?;
        let project: Self = doc
            .try_into()
            .map_err(|e: toml::de::Error| ProjectError::Parse(e.to_string()))?;
        Ok(project)
    }

    /// Serialises to TOML text with the current schema version.
    pub fn to_toml(&self) -> Result<String, ProjectError> {
        let mut copy = self.clone();
        copy.schema_version = SCHEMA_VERSION;
        toml::to_string_pretty(&copy).map_err(|e| ProjectError::Parse(e.to_string()))
    }

    /// Loads from a file.
    pub fn load(path: &Path) -> Result<Self, ProjectError> {
        let text = std::fs::read_to_string(path)?;
        Self::from_toml(&text)
    }

    /// Saves atomically: writes a sibling temp file, flushes, then renames over the target so a
    /// crash mid-write can never leave a truncated project behind.
    pub fn save(&self, path: &Path) -> Result<(), ProjectError> {
        let text = self.to_toml()?;
        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir)?;
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tmp = dir.join(format!(".{file_name}.tmp-{}", std::process::id()));
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
        }
        if let Err(e) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
        Ok(())
    }

    /// Finds a source by id.
    pub fn source(&self, id: &str) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }

    /// Finds an output by id.
    pub fn output(&self, id: &str) -> Option<&Output> {
        self.outputs.iter().find(|o| o.id == id)
    }

    /// Renames a surface. Fails when the new id is invalid or taken.
    pub fn rename_surface(&mut self, old: &str, new: &str) -> Result<(), ProjectError> {
        self.check_new_id(new, old)?;
        let s = self
            .surface_mut(old)
            .ok_or_else(|| ProjectError::UnknownParam(old.to_string()))?;
        s.id = new.to_string();
        Ok(())
    }

    /// Renames an output and every surface that references it.
    pub fn rename_output(&mut self, old: &str, new: &str) -> Result<(), ProjectError> {
        self.check_new_id(new, old)?;
        let o = self
            .output_mut(old)
            .ok_or_else(|| ProjectError::UnknownParam(old.to_string()))?;
        o.id = new.to_string();
        for s in &mut self.surfaces {
            if s.output == old {
                s.output = new.to_string();
            }
        }
        Ok(())
    }

    /// Renames a source and every surface that references it.
    pub fn rename_source(&mut self, old: &str, new: &str) -> Result<(), ProjectError> {
        self.check_new_id(new, old)?;
        let src = self
            .sources
            .iter_mut()
            .find(|s| s.id == old)
            .ok_or_else(|| ProjectError::UnknownParam(old.to_string()))?;
        src.id = new.to_string();
        for s in &mut self.surfaces {
            if s.source == old {
                s.source = new.to_string();
            }
        }
        Ok(())
    }

    fn check_new_id(&self, new: &str, old: &str) -> Result<(), ProjectError> {
        if !is_valid_id(new) {
            return Err(ProjectError::Parse(format!(
                "id {new:?} must match [A-Za-z0-9_-]+"
            )));
        }
        if new != old && self.id_in_use(new) {
            return Err(ProjectError::Parse(format!("id {new:?} is already used")));
        }
        Ok(())
    }

    /// True when any source, output or surface has this id.
    pub fn id_in_use(&self, id: &str) -> bool {
        self.sources.iter().any(|s| s.id == id)
            || self.outputs.iter().any(|o| o.id == id)
            || self.surfaces.iter().any(|s| s.id == id)
    }

    /// A free id derived from `prefix` (`prefix`, `prefix-2`, `prefix-3`, ...).
    pub fn fresh_id(&self, prefix: &str) -> String {
        let base: String = prefix
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        let base = if base.is_empty() {
            "item".to_string()
        } else {
            base
        };
        if !self.id_in_use(&base) {
            return base;
        }
        let limit = self.sources.len() + self.outputs.len() + self.surfaces.len() + 3;
        (2..limit)
            .map(|n| format!("{base}-{n}"))
            .find(|id| !self.id_in_use(id))
            .unwrap_or(base)
    }

    /// Copies a surface under a fresh id, placed just above the original in draw order.
    /// Returns the new id.
    pub fn duplicate_surface(&mut self, id: &str) -> Option<String> {
        let original = self.surface(id)?.clone();
        let new_id = self.fresh_id(&format!("{id}-copy"));
        let mut copy = original;
        copy.id.clone_from(&new_id);
        copy.z += 1;
        copy.locked = false;
        self.surfaces.push(copy);
        Some(new_id)
    }

    /// Finds a surface by id.
    pub fn surface(&self, id: &str) -> Option<&Surface> {
        self.surfaces.iter().find(|s| s.id == id)
    }

    /// Finds a surface mutably.
    pub fn surface_mut(&mut self, id: &str) -> Option<&mut Surface> {
        self.surfaces.iter_mut().find(|s| s.id == id)
    }

    /// Finds an output mutably.
    pub fn output_mut(&mut self, id: &str) -> Option<&mut Output> {
        self.outputs.iter_mut().find(|o| o.id == id)
    }

    /// Placed outputs as desktop rectangles, with their index in `outputs`.
    pub fn placed_rects(&self) -> Vec<(usize, crate::blend::PlacedRect)> {
        self.outputs
            .iter()
            .enumerate()
            .filter_map(|(i, o)| {
                let p = o.placement.as_ref()?;
                Some((
                    i,
                    crate::blend::PlacedRect {
                        x: i64::from(p.x),
                        y: i64::from(p.y),
                        width: i64::from(o.width),
                        height: i64::from(o.height),
                    },
                ))
            })
            .collect()
    }

    /// Sets blend widths from the overlap of placed outputs; returns how many edges changed.
    /// Outputs without a placement, or that overlap nothing, are left alone.
    pub fn suggest_blends(&mut self) -> usize {
        let mut n = 0;
        for (i, edge, width) in crate::blend::suggest_all(&self.placed_rects()) {
            let e = self.outputs[i].blend.edge_mut(edge);
            if (e.width - width).abs() > 1e-9 {
                e.width = width;
                n += 1;
            }
        }
        n
    }

    /// Surfaces drawn on `output`, in draw order.
    pub fn surfaces_for_output(&self, output: &str) -> Vec<&Surface> {
        let mut v: Vec<&Surface> = self
            .surfaces
            .iter()
            .filter(|s| s.output == output)
            .collect();
        v.sort_by_key(|s| s.z);
        v
    }
}

/// True for ids that are safe in parameter addresses and file names.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Brings a raw TOML document up to [`SCHEMA_VERSION`] in place.
///
/// Each step is a pure function on the table; add a new arm when the schema changes and leave
mod demo;
mod migrate;
mod params;
mod validate;

pub use demo::demo_project;
pub use migrate::migrate;

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used, clippy::too_many_lines)]
mod tests;
