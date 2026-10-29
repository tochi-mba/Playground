//! The project file: everything a show needs, as plain, diffable TOML.
//!
//! A project is a set of **sources** (where pixels come from), **outputs** (physical
//! projectors with their blend and colour settings) and **surfaces** (a warp that places a
//! source onto an output). Files carry a `schema_version`; [`Project::load`] migrates older
//! versions forward so a show saved years ago still opens.

use crate::blend::EdgeBlend;
use crate::mask::Mask;
use crate::params::{ParamAddress, ParamPatch, ParamValue};
use crate::undo::KeyedValues;
use crate::warp::{Warp, WarpKind};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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

    /// Checks referential integrity and geometry. Empty when the project is sound.
    pub fn validate(&self) -> Vec<Issue> {
        let mut issues = Vec::new();
        if self.schema_version > SCHEMA_VERSION {
            issues.push(Issue::err(format!(
                "schema_version {} is newer than {SCHEMA_VERSION}",
                self.schema_version
            )));
        }
        let mut seen = HashSet::new();
        for (what, ids) in [
            (
                "source",
                self.sources
                    .iter()
                    .map(|s| s.id.as_str())
                    .collect::<Vec<_>>(),
            ),
            (
                "output",
                self.outputs.iter().map(|o| o.id.as_str()).collect(),
            ),
            (
                "surface",
                self.surfaces.iter().map(|s| s.id.as_str()).collect(),
            ),
        ] {
            seen.clear();
            for id in ids {
                if !is_valid_id(id) {
                    issues.push(Issue::err(format!(
                        "{what} id {id:?} must match [A-Za-z0-9_-]+"
                    )));
                }
                if !seen.insert(id) {
                    issues.push(Issue::err(format!("duplicate {what} id {id:?}")));
                }
            }
        }
        for o in &self.outputs {
            if o.width == 0 || o.height == 0 {
                issues.push(Issue::err(format!(
                    "output {:?} has a zero dimension",
                    o.id
                )));
            }
            if o.width > 16384 || o.height > 16384 {
                issues.push(Issue::warn(format!(
                    "output {:?} is larger than 16384px",
                    o.id
                )));
            }
        }
        for s in &self.surfaces {
            if self.output(&s.output).is_none() {
                issues.push(Issue::err(format!(
                    "surface {:?} references unknown output {:?}",
                    s.id, s.output
                )));
            }
            if self.source(&s.source).is_none() {
                issues.push(Issue::err(format!(
                    "surface {:?} references unknown source {:?}",
                    s.id, s.source
                )));
            }
            if let Err(e) = s.warp.tessellate(1) {
                issues.push(Issue::err(format!(
                    "surface {:?} warp is invalid: {e}",
                    s.id
                )));
            }
            if !(0.0..=1.0).contains(&s.opacity) {
                issues.push(Issue::warn(format!(
                    "surface {:?} opacity {} is outside [0,1]",
                    s.id, s.opacity
                )));
            }
            for (i, m) in s.masks.iter().enumerate() {
                if !m.is_valid() {
                    issues.push(Issue::err(format!(
                        "surface {:?} mask {i} needs at least three finite points",
                        s.id
                    )));
                }
            }
        }
        if self.outputs.is_empty() {
            issues.push(Issue::warn("project has no outputs"));
        }
        issues
    }

    /// True when [`Project::validate`] reports no errors.
    pub fn is_valid(&self) -> bool {
        self.validate().iter().all(|i| !i.error)
    }

    /// Reads the value at a parameter address, if it exists.
    pub fn get_param(&self, address: &ParamAddress) -> Option<ParamValue> {
        use ParamValue::{Bool, Number};
        let segs: Vec<&str> = address.segments().iter().map(String::as_str).collect();
        match segs.as_slice() {
            ["show", "blackout"] => Some(Bool(self.blackout)),
            ["show", "master"] => Some(Number(self.master)),
            ["surface", id, rest @ ..] => {
                let s = self.surface(id)?;
                match rest {
                    ["opacity"] => Some(Number(s.opacity)),
                    ["visible"] => Some(Bool(s.visible)),
                    ["z"] => Some(Number(f64::from(s.z))),
                    ["corner", n, axis] => {
                        let WarpKind::Quad(q) = &s.warp else {
                            return None;
                        };
                        let c = q.corners.get(n.parse::<usize>().ok()?)?;
                        match *axis {
                            "x" => Some(Number(c.x)),
                            "y" => Some(Number(c.y)),
                            _ => None,
                        }
                    }
                    _ => None,
                }
            }
            ["source", id, rest @ ..] => {
                let s = self.source(id)?;
                match (rest, &s.kind) {
                    (["speed"], SourceKind::Generator { speed, .. }) => {
                        Some(Number(f64::from(*speed)))
                    }
                    (["play"], k) => k.playback().map(|p| Bool(!p.paused)),
                    (["loop"], k) => k.playback().map(|p| Bool(p.looping)),
                    (["speed"], k) => k.playback().map(|p| Number(p.speed)),
                    (["restart"], k) => k.playback().map(|_| Bool(false)),
                    _ => None,
                }
            }
            ["output", id, rest @ ..] => {
                let o = self.output(id)?;
                match rest {
                    ["enabled"] => Some(Bool(o.enabled)),
                    ["brightness"] => Some(Number(o.color.brightness)),
                    ["contrast"] => Some(Number(o.color.contrast)),
                    ["gamma"] => Some(Number(o.color.gamma)),
                    ["blend", "gamma"] => Some(Number(o.blend.gamma)),
                    ["blend", edge, "width"] => {
                        let e = match *edge {
                            "left" => o.blend.left,
                            "right" => o.blend.right,
                            "top" => o.blend.top,
                            "bottom" => o.blend.bottom,
                            _ => return None,
                        };
                        Some(Number(e.width))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Applies a parameter change.
    pub fn apply_patch(&mut self, patch: &ParamPatch) -> Result<(), ProjectError> {
        let addr = patch.address.to_string();
        let unknown = || ProjectError::UnknownParam(addr.clone());
        let bad = || ProjectError::BadValue(addr.clone(), patch.value.clone());
        let num = || patch.value.as_f64().ok_or_else(bad);
        let boolean = || patch.value.as_bool().ok_or_else(bad);
        let segs: Vec<&str> = patch
            .address
            .segments()
            .iter()
            .map(String::as_str)
            .collect();
        match segs.as_slice() {
            ["show", "blackout"] => self.blackout = boolean()?,
            ["show", "master"] => self.master = num()?.clamp(0.0, 1.0),
            ["surface", id, rest @ ..] => {
                let s = self.surface_mut(id).ok_or_else(unknown)?;
                match rest {
                    ["opacity"] => s.opacity = num()?.clamp(0.0, 1.0),
                    ["visible"] => s.visible = boolean()?,
                    ["z"] => s.z = num()? as i32,
                    ["corner", n, axis] => {
                        let v = num()?;
                        let WarpKind::Quad(q) = &mut s.warp else {
                            return Err(unknown());
                        };
                        let i = n.parse::<usize>().map_err(|_| unknown())?;
                        let c = q.corners.get_mut(i).ok_or_else(unknown)?;
                        match *axis {
                            "x" => c.x = v,
                            "y" => c.y = v,
                            _ => return Err(unknown()),
                        }
                    }
                    _ => return Err(unknown()),
                }
            }
            ["source", id, rest @ ..] => {
                let s = self
                    .sources
                    .iter_mut()
                    .find(|s| s.id == *id)
                    .ok_or_else(unknown)?;
                if let (["speed"], SourceKind::Generator { speed, .. }) = (rest, &mut s.kind) {
                    *speed = num()?.clamp(0.0, 10.0) as f32;
                    return Ok(());
                }
                let pb = s.kind.playback_mut().ok_or_else(unknown)?;
                match rest {
                    ["play"] => pb.paused = !boolean()?,
                    ["loop"] => pb.looping = boolean()?,
                    ["speed"] => pb.speed = num()?.clamp(0.0, 16.0),
                    ["restart"] => pb.restart_token = pb.restart_token.wrapping_add(1),
                    _ => return Err(unknown()),
                }
            }
            ["output", id, rest @ ..] => {
                let o = self.output_mut(id).ok_or_else(unknown)?;
                match rest {
                    ["enabled"] => o.enabled = boolean()?,
                    ["brightness"] => o.color.brightness = num()?.clamp(0.0, 4.0),
                    ["contrast"] => o.color.contrast = num()?.clamp(0.0, 4.0),
                    ["gamma"] => o.color.gamma = num()?.clamp(0.2, 5.0),
                    ["blend", "gamma"] => o.blend.gamma = num()?.clamp(0.5, 4.0),
                    ["blend", edge, "width"] => {
                        let v = num()?.clamp(0.0, 1.0);
                        let e = match *edge {
                            "left" => &mut o.blend.left,
                            "right" => &mut o.blend.right,
                            "top" => &mut o.blend.top,
                            "bottom" => &mut o.blend.bottom,
                            _ => return Err(unknown()),
                        };
                        e.width = v;
                    }
                    _ => return Err(unknown()),
                }
            }
            _ => return Err(unknown()),
        }
        Ok(())
    }
}

impl KeyedValues<ParamValue> for Project {
    fn set_keyed(&mut self, key: &str, value: &ParamValue) {
        if let Ok(address) = ParamAddress::parse(key) {
            // Undo commands are built from values this project produced, so a failure here
            // means the target was deleted since; ignoring it is the right thing for undo.
            let _ = self.apply_patch(&ParamPatch {
                address,
                value: value.clone(),
            });
        }
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
/// the old arms alone so every historical file keeps loading.
pub fn migrate(doc: &mut toml::Table) -> Result<(), ProjectError> {
    loop {
        let version = match doc.get("schema_version") {
            None => 0,
            Some(toml::Value::Integer(v)) if *v >= 0 => *v as u32,
            Some(other) => {
                return Err(ProjectError::Parse(format!(
                    "schema_version must be a non-negative integer, got {other}"
                )))
            }
        };
        match version {
            0 => migrate_0_to_1(doc),
            v if v == SCHEMA_VERSION => return Ok(()),
            v if v > SCHEMA_VERSION => {
                return Err(ProjectError::TooNew {
                    found: v,
                    supported: SCHEMA_VERSION,
                })
            }
            // Every version below SCHEMA_VERSION has an arm above; this is unreachable by
            // construction but keeps the match exhaustive when a version is added.
            v => return Err(ProjectError::Parse(format!("no migration from schema {v}"))),
        }
    }
}

/// Schema 0 was the pre-release format: outputs stored `size = [w, h]` and surfaces used
/// `alpha` instead of `opacity`.
fn migrate_0_to_1(doc: &mut toml::Table) {
    if let Some(toml::Value::Array(outputs)) = doc.get_mut("outputs") {
        for o in outputs.iter_mut().filter_map(toml::Value::as_table_mut) {
            if let Some(toml::Value::Array(size)) = o.remove("size") {
                if let [w, h] = size.as_slice() {
                    o.insert("width".into(), w.clone());
                    o.insert("height".into(), h.clone());
                }
            }
        }
    }
    if let Some(toml::Value::Array(surfaces)) = doc.get_mut("surfaces") {
        for s in surfaces.iter_mut().filter_map(toml::Value::as_table_mut) {
            if let Some(a) = s.remove("alpha") {
                s.entry("opacity").or_insert(a);
            }
        }
    }
    doc.insert("schema_version".into(), toml::Value::Integer(1));
}

/// A small two-projector demo used by the examples, the CLI tests and the golden images.
#[allow(clippy::too_many_lines)]
pub fn demo_project() -> Project {
    use crate::blend::{BlendCurve, BlendEdge};
    use crate::geom::Vec2;
    use crate::warp::Quad;
    let blend = |left: f64, right: f64| EdgeBlend {
        left: BlendEdge {
            width: left,
            curve: BlendCurve::Smooth,
        },
        right: BlendEdge {
            width: right,
            curve: BlendCurve::Smooth,
        },
        gamma: 2.2,
        ..EdgeBlend::default()
    };
    Project {
        name: "Two projector demo".into(),
        sources: vec![
            Source {
                id: "grid".into(),
                kind: SourceKind::TestPattern {
                    pattern: TestPattern::Grid,
                },
            },
            Source {
                id: "bars".into(),
                kind: SourceKind::TestPattern {
                    pattern: TestPattern::Bars,
                },
            },
        ],
        outputs: vec![
            Output {
                id: "left".into(),
                width: 1920,
                height: 1080,
                enabled: true,
                blend: blend(0.0, 0.2),
                color: ColorAdjust::default(),
                placement: None,
            },
            Output {
                id: "right".into(),
                width: 1920,
                height: 1080,
                enabled: true,
                blend: blend(0.2, 0.0),
                color: ColorAdjust::default(),
                placement: None,
            },
        ],
        surfaces: vec![
            Surface {
                id: "wall-left".into(),
                output: "left".into(),
                source: "grid".into(),
                z: 0,
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                crop: Crop {
                    x: 0.0,
                    y: 0.0,
                    w: 0.6,
                    h: 1.0,
                },
                warp: WarpKind::Quad(Quad {
                    corners: [
                        Vec2::new(0.05, 0.08),
                        Vec2::new(0.98, 0.04),
                        Vec2::new(0.96, 0.95),
                        Vec2::new(0.03, 0.9),
                    ],
                }),
                masks: Vec::new(),
                locked: false,
            },
            Surface {
                id: "wall-right".into(),
                output: "right".into(),
                source: "grid".into(),
                z: 0,
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                crop: Crop {
                    x: 0.4,
                    y: 0.0,
                    w: 0.6,
                    h: 1.0,
                },
                warp: WarpKind::default(),
                masks: Vec::new(),
                locked: false,
            },
            Surface {
                id: "logo".into(),
                output: "right".into(),
                source: "bars".into(),
                z: 1,
                visible: true,
                opacity: 0.8,
                blend_mode: BlendMode::Screen,
                crop: Crop::default(),
                warp: WarpKind::Quad(Quad {
                    corners: [
                        Vec2::new(0.6, 0.6),
                        Vec2::new(0.9, 0.62),
                        Vec2::new(0.88, 0.9),
                        Vec2::new(0.62, 0.88),
                    ],
                }),
                masks: vec![Mask {
                    points: vec![
                        Vec2::new(0.4, 0.4),
                        Vec2::new(0.6, 0.4),
                        Vec2::new(0.5, 0.6),
                    ],
                    feather: 0.05,
                    subtract: true,
                }],
                locked: false,
            },
        ],
        ..Project::default()
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used, clippy::too_many_lines)]
mod tests {
    use super::*;
    use crate::geom::Vec2;
    use proptest::prelude::*;

    #[test]
    fn demo_project_is_valid_and_round_trips() {
        let p = demo_project();
        assert_eq!(p.validate(), Vec::<Issue>::new());
        assert!(p.is_valid());
        let text = p.to_toml().unwrap();
        assert!(text.starts_with("schema_version = 1"), "{text}");
        let back = Project::from_toml(&text).unwrap();
        assert_eq!(back, p);
        assert_eq!(
            p.surfaces_for_output("right")
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            ["wall-right", "logo"]
        );
        assert!(p.surfaces_for_output("nope").is_empty());
        assert_eq!(p.source("grid").map(|s| s.id.as_str()), Some("grid"));
        assert!(p.output("missing").is_none());
    }

    #[test]
    fn save_and_load_are_atomic_and_create_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("show.lumen.toml");
        let p = demo_project();
        p.save(&path).unwrap();
        assert_eq!(Project::load(&path).unwrap(), p);
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, ["show.lumen.toml"], "no temp file left behind");
        // Overwrite keeps working.
        let mut p2 = p.clone();
        p2.name = "renamed".into();
        p2.save(&path).unwrap();
        assert_eq!(Project::load(&path).unwrap().name, "renamed");
        // Saving into a path whose parent is a file fails cleanly.
        let bad = path.join("child.toml");
        assert!(matches!(p.save(&bad), Err(ProjectError::Io(_))));
        assert!(matches!(
            Project::load(&dir.path().join("none.toml")),
            Err(ProjectError::Io(_))
        ));
        // A bare file name saves into the current directory.
        let cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        p.save(Path::new("bare.toml")).unwrap();
        std::env::set_current_dir(cwd).unwrap();
        assert!(dir.path().join("bare.toml").exists());
    }

    #[test]
    fn validate_reports_every_problem() {
        let mut p = demo_project();
        p.surfaces[0].output = "ghost".into();
        p.surfaces[0].source = "ghost".into();
        p.surfaces[0].opacity = 2.0;
        p.surfaces[1].masks.push(Mask::hole(vec![Vec2::ZERO]));
        p.outputs[0].width = 0;
        p.outputs[1].height = 20000;
        p.outputs.push(p.outputs[1].clone());
        p.sources[0].id = "bad id".into();
        if let WarpKind::Quad(q) = &mut p.surfaces[2].warp {
            q.corners[1] = q.corners[0];
        }
        p.schema_version = 99;
        let issues = p.validate();
        let msgs: Vec<String> = issues.iter().map(ToString::to_string).collect();
        let expect = [
            "error: schema_version 99 is newer than 1",
            "error: source id \"bad id\" must match",
            "error: duplicate output id \"right\"",
            "error: output \"left\" has a zero dimension",
            "warning: output \"right\" is larger than 16384px",
            "error: surface \"wall-left\" references unknown output \"ghost\"",
            "error: surface \"wall-left\" references unknown source \"ghost\"",
            "warning: surface \"wall-left\" opacity 2 is outside [0,1]",
            "error: surface \"wall-right\" mask 0 needs at least three finite points",
            "error: surface \"logo\" warp is invalid",
        ];
        for e in expect {
            assert!(
                msgs.iter().any(|m| m.starts_with(e)),
                "missing {e:?} in {msgs:#?}"
            );
        }
        assert!(!p.is_valid());
        let empty = Project::default();
        assert_eq!(
            empty.validate(),
            vec![Issue::warn("project has no outputs")]
        );
        assert!(empty.is_valid());
    }

    #[test]
    fn migrates_schema_zero() {
        let legacy = r#"
name = "old show"
[[outputs]]
id = "a"
size = [1280, 720]
[[surfaces]]
id = "s"
output = "a"
source = "x"
alpha = 0.5
[[sources]]
id = "x"
kind = "solid"
color = [1.0, 0.0, 0.0, 1.0]
"#;
        let p = Project::from_toml(legacy).unwrap();
        assert_eq!(p.schema_version, 1);
        assert_eq!((p.outputs[0].width, p.outputs[0].height), (1280, 720));
        assert_eq!(p.surfaces[0].opacity, 0.5);
        assert!(p.is_valid());
        // Migration applies defaults for everything else.
        assert_eq!(p.outputs[0].blend, EdgeBlend::none());
        assert!(p.outputs[0].enabled);
        // A malformed size is dropped and then the schema rejects the missing width.
        let bad = "[[outputs]]\nid = \"a\"\nsize = [1]\n";
        assert!(matches!(
            Project::from_toml(bad),
            Err(ProjectError::Parse(_))
        ));
        // alpha never overrides an explicit opacity.
        let both = "[[surfaces]]\nid=\"s\"\noutput=\"a\"\nsource=\"x\"\nalpha=0.5\nopacity=0.9\n";
        assert_eq!(Project::from_toml(both).unwrap().surfaces[0].opacity, 0.9);
    }

    #[test]
    fn rejects_newer_and_malformed_versions() {
        let err = Project::from_toml("schema_version = 2").unwrap_err();
        assert!(matches!(
            err,
            ProjectError::TooNew {
                found: 2,
                supported: 1
            }
        ));
        assert!(err.to_string().contains("newer"));
        let err = Project::from_toml("schema_version = \"one\"").unwrap_err();
        assert!(matches!(err, ProjectError::Parse(_)));
        assert!(Project::from_toml("schema_version = -1").is_err());
        assert!(Project::from_toml("this is not toml [").is_err());
        assert!(Project::from_toml("schema_version = 1\n[[outputs]]\nid = 5\n").is_err());
    }

    #[test]
    fn params_get_and_set() {
        let mut p = demo_project();
        let set = |p: &mut Project, a: &str, v: ParamValue| {
            p.apply_patch(&ParamPatch {
                address: ParamAddress::parse(a).unwrap(),
                value: v,
            })
        };
        let get = |p: &Project, a: &str| p.get_param(&ParamAddress::parse(a).unwrap());

        set(&mut p, "/show/blackout", ParamValue::Bool(true)).unwrap();
        assert_eq!(get(&p, "/show/blackout"), Some(ParamValue::Bool(true)));
        set(&mut p, "/show/master", ParamValue::Number(7.0)).unwrap();
        assert_eq!(
            get(&p, "/show/master"),
            Some(ParamValue::Number(1.0)),
            "clamped"
        );

        set(
            &mut p,
            "/surface/logo/opacity",
            ParamValue::Text("0.25".into()),
        )
        .unwrap();
        assert_eq!(
            get(&p, "/surface/logo/opacity"),
            Some(ParamValue::Number(0.25))
        );
        set(&mut p, "/surface/logo/visible", ParamValue::Number(0.0)).unwrap();
        assert_eq!(
            get(&p, "/surface/logo/visible"),
            Some(ParamValue::Bool(false))
        );
        set(&mut p, "/surface/logo/z", ParamValue::Number(-3.0)).unwrap();
        assert_eq!(get(&p, "/surface/logo/z"), Some(ParamValue::Number(-3.0)));
        set(&mut p, "/surface/logo/corner/2/x", ParamValue::Number(0.77)).unwrap();
        assert_eq!(
            get(&p, "/surface/logo/corner/2/x"),
            Some(ParamValue::Number(0.77))
        );
        set(&mut p, "/surface/logo/corner/2/y", ParamValue::Number(0.66)).unwrap();
        assert_eq!(
            get(&p, "/surface/logo/corner/2/y"),
            Some(ParamValue::Number(0.66))
        );

        set(&mut p, "/output/left/enabled", ParamValue::Bool(false)).unwrap();
        assert_eq!(
            get(&p, "/output/left/enabled"),
            Some(ParamValue::Bool(false))
        );
        for (a, v, want) in [
            ("/output/left/brightness", 9.0, 4.0),
            ("/output/left/contrast", -1.0, 0.0),
            ("/output/left/gamma", 0.0, 0.2),
            ("/output/left/blend/gamma", 10.0, 4.0),
            ("/output/left/blend/left/width", 0.3, 0.3),
            ("/output/left/blend/right/width", 2.0, 1.0),
            ("/output/left/blend/top/width", 0.1, 0.1),
            ("/output/left/blend/bottom/width", 0.2, 0.2),
        ] {
            set(&mut p, a, ParamValue::Number(v)).unwrap();
            assert_eq!(get(&p, a), Some(ParamValue::Number(want)), "{a}");
        }

        // Unknown and bad.
        for a in [
            "/nope",
            "/show/nope",
            "/surface/ghost/opacity",
            "/surface/logo/nope",
            "/surface/logo/corner/9/x",
            "/surface/logo/corner/x/x",
            "/surface/logo/corner/1/z",
            "/output/ghost/enabled",
            "/output/left/nope",
            "/output/left/blend/diagonal/width",
        ] {
            assert!(
                matches!(
                    set(&mut p, a, ParamValue::Number(1.0)),
                    Err(ProjectError::UnknownParam(_))
                ),
                "{a}"
            );
            assert_eq!(get(&p, a), None, "{a}");
        }
        let err = set(
            &mut p,
            "/surface/logo/opacity",
            ParamValue::Text("loud".into()),
        )
        .unwrap_err();
        assert!(matches!(err, ProjectError::BadValue(..)));
        assert!(err.to_string().contains("cannot take value"));
        assert!(set(&mut p, "/show/blackout", ParamValue::Text("maybe".into())).is_err());

        // Corner params only exist on quads.
        p.surfaces[2].warp = WarpKind::Bezier(crate::warp::BezierMesh::flat(1, 1).unwrap());
        assert_eq!(get(&p, "/surface/logo/corner/0/x"), None);
        assert!(set(&mut p, "/surface/logo/corner/0/x", ParamValue::Number(0.0)).is_err());
    }

    #[test]
    fn source_transport_params() {
        let mut p = demo_project();
        p.sources.push(Source {
            id: "clip".into(),
            kind: SourceKind::Video {
                path: "clip.mp4".into(),
                playback: Playback::default(),
            },
        });
        p.sources.push(Source {
            id: "gen".into(),
            kind: SourceKind::Generator {
                pattern: GeneratorKind::Plasma,
                speed: 1.0,
                color: [1.0; 4],
            },
        });
        let set = |p: &mut Project, a: &str, v: ParamValue| {
            p.apply_patch(&ParamPatch {
                address: ParamAddress::parse(a).unwrap(),
                value: v,
            })
        };
        let get = |p: &Project, a: &str| p.get_param(&ParamAddress::parse(a).unwrap());
        assert_eq!(get(&p, "/source/clip/play"), Some(ParamValue::Bool(true)));
        set(&mut p, "/source/clip/play", ParamValue::Bool(false)).unwrap();
        assert!(p.source("clip").unwrap().kind.playback().unwrap().paused);
        set(&mut p, "/source/clip/loop", ParamValue::Bool(false)).unwrap();
        assert_eq!(get(&p, "/source/clip/loop"), Some(ParamValue::Bool(false)));
        set(&mut p, "/source/clip/speed", ParamValue::Number(99.0)).unwrap();
        assert_eq!(
            get(&p, "/source/clip/speed"),
            Some(ParamValue::Number(16.0))
        );
        assert_eq!(
            get(&p, "/source/clip/restart"),
            Some(ParamValue::Bool(false))
        );
        set(&mut p, "/source/clip/restart", ParamValue::Bool(true)).unwrap();
        assert_eq!(
            p.source("clip")
                .unwrap()
                .kind
                .playback()
                .unwrap()
                .restart_token,
            1
        );
        assert!(set(&mut p, "/source/clip/nope", ParamValue::Bool(true)).is_err());
        assert!(
            set(&mut p, "/source/grid/play", ParamValue::Bool(true)).is_err(),
            "patterns have no transport"
        );
        assert_eq!(get(&p, "/source/grid/play"), None);
        assert!(set(&mut p, "/source/ghost/play", ParamValue::Bool(true)).is_err());
        set(&mut p, "/source/gen/speed", ParamValue::Number(2.5)).unwrap();
        assert_eq!(get(&p, "/source/gen/speed"), Some(ParamValue::Number(2.5)));
        assert!(set(&mut p, "/source/gen/play", ParamValue::Bool(true)).is_err());
        assert!(set(&mut p, "/source/clip/speed", ParamValue::Text("x".into())).is_err());
        let text = p.to_toml().unwrap();
        assert!(
            !text.contains("restart_token"),
            "restart token is never saved"
        );
        let back = Project::from_toml(&text).unwrap();
        assert_eq!(
            back.source("clip").unwrap().kind.playback().unwrap().speed,
            16.0
        );
        let kinds = [
            SourceKind::Video {
                path: "v".into(),
                playback: Playback::default(),
            },
            SourceKind::Sequence {
                path: "d".into(),
                fps: 25.0,
                playback: Playback::default(),
            },
            SourceKind::Generator {
                pattern: GeneratorKind::Noise,
                speed: 1.0,
                color: [1.0; 4],
            },
            SourceKind::Image { path: "i".into() },
            SourceKind::Spout {
                sender: String::new(),
            },
        ];
        assert_eq!(
            kinds.iter().map(SourceKind::label).collect::<Vec<_>>(),
            ["video", "sequence", "generator", "image", "spout"]
        );
        assert_eq!(kinds[1].path(), Some("d"));
        assert_eq!(kinds[2].path(), None);
        assert!(kinds[2].playback().is_none());
        let seq: Source =
            toml::from_str("id = \"s\"\nkind = \"sequence\"\npath = \"frames\"").unwrap();
        assert_eq!(
            seq.kind,
            SourceKind::Sequence {
                path: "frames".into(),
                fps: 25.0,
                playback: Playback::default()
            }
        );
        let g: Source =
            toml::from_str("id = \"g\"\nkind = \"generator\"\npattern = \"radar\"\n").unwrap();
        assert_eq!(
            g.kind,
            SourceKind::Generator {
                pattern: GeneratorKind::Radar,
                speed: 1.0,
                color: [1.0; 4]
            }
        );
        assert_eq!(GeneratorKind::ALL.len(), 6);
    }

    #[test]
    fn keyed_values_feed_undo() {
        use crate::undo::{History, SetValue};
        let mut p = demo_project();
        let mut h: History<Project> = History::default();
        let key = "/surface/logo/opacity";
        let before = p.get_param(&ParamAddress::parse(key).unwrap()).unwrap();
        h.push(
            &mut p,
            Box::new(SetValue {
                key: key.into(),
                before: before.clone(),
                after: ParamValue::Number(0.1),
            }),
        );
        assert_eq!(p.surfaces[2].opacity, 0.1);
        h.undo(&mut p);
        assert_eq!(p.surfaces[2].opacity, 0.8);
        // Keys that no longer resolve are ignored rather than panicking.
        p.set_keyed("/surface/gone/opacity", &ParamValue::Number(0.5));
        p.set_keyed("not an address", &ParamValue::Number(0.5));
        assert_eq!(p, demo_project());
    }

    #[test]
    fn rename_rewrites_references_and_validates() {
        let mut p = demo_project();
        p.rename_output("left", "stage-left").unwrap();
        assert_eq!(p.surfaces[0].output, "stage-left");
        assert!(p.output("stage-left").is_some());
        p.rename_source("grid", "pattern").unwrap();
        assert_eq!(
            p.surfaces.iter().filter(|s| s.source == "pattern").count(),
            2
        );
        p.rename_surface("logo", "badge").unwrap();
        assert!(p.surface("badge").is_some());
        assert!(p.is_valid());
        assert!(p.rename_surface("badge", "bad id").is_err());
        assert!(
            p.rename_surface("badge", "bars").is_err(),
            "taken by a source"
        );
        assert!(
            p.rename_surface("badge", "badge").is_ok(),
            "same id is fine"
        );
        assert!(p.rename_surface("ghost", "x").is_err());
        assert!(p.rename_output("ghost", "x").is_err());
        assert!(p.rename_source("ghost", "x").is_err());
        assert!(p.id_in_use("badge") && !p.id_in_use("free"));
    }

    #[test]
    fn fresh_ids_and_duplicates() {
        let mut p = demo_project();
        assert_eq!(p.fresh_id("new"), "new");
        assert_eq!(p.fresh_id("logo"), "logo-2");
        assert_eq!(p.fresh_id("bad id!"), "badid");
        assert_eq!(p.fresh_id("!!"), "item");
        let copy = p.duplicate_surface("logo").unwrap();
        assert_eq!(copy, "logo-copy");
        let c = p.surface("logo-copy").unwrap();
        assert_eq!(c.z, 2);
        assert_eq!(c.source, "bars");
        assert_eq!(p.duplicate_surface("logo").unwrap(), "logo-copy-2");
        assert!(p.duplicate_surface("ghost").is_none());
        assert!(p.is_valid());
    }

    #[test]
    fn id_rules() {
        assert!(is_valid_id("wall-1_a"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("a/b"));
        assert!(!is_valid_id("é"));
    }

    #[test]
    fn serde_defaults_for_optional_fields() {
        let text = r#"
schema_version = 1
[[sources]]
id = "s"
kind = "spout"
[[outputs]]
id = "o"
width = 10
height = 10
[[surfaces]]
id = "a"
output = "o"
source = "s"
"#;
        let p = Project::from_toml(text).unwrap();
        assert_eq!(
            p.sources[0].kind,
            SourceKind::Spout {
                sender: String::new()
            }
        );
        let s = &p.surfaces[0];
        assert!(s.visible && s.opacity == 1.0 && s.z == 0 && s.masks.is_empty());
        assert_eq!(s.blend_mode, BlendMode::Normal);
        assert_eq!(s.crop, Crop::default());
        assert_eq!(s.warp, WarpKind::default());
        assert_eq!(p.outputs[0].color, ColorAdjust::default());
        assert_eq!(p.outputs[0].placement, None);
        assert_eq!(p.master, 1.0);
        assert!(!p.blackout);
        let img: Source = toml::from_str("id = \"i\"\nkind = \"image\"\npath = \"x.png\"").unwrap();
        assert_eq!(
            img.kind,
            SourceKind::Image {
                path: "x.png".into()
            }
        );
        let pl: Placement = toml::from_str("monitor = \"m\"").unwrap();
        assert_eq!(
            pl,
            Placement {
                monitor: "m".into(),
                x: 0,
                y: 0
            }
        );
        for pat in ["grid", "bars", "checker", "ramp"] {
            let s: Source = toml::from_str(&format!(
                "id = \"p\"\nkind = \"test_pattern\"\npattern = \"{pat}\""
            ))
            .unwrap();
            assert!(matches!(s.kind, SourceKind::TestPattern { .. }));
        }
    }

    proptest! {
        #[test]
        fn opacity_patches_always_land_in_range(v in -10.0f64..10.0) {
            let mut p = demo_project();
            p.apply_patch(&ParamPatch {
                address: ParamAddress::parse("/surface/logo/opacity").unwrap(),
                value: ParamValue::Number(v),
            }).unwrap();
            prop_assert!((0.0..=1.0).contains(&p.surfaces[2].opacity));
            prop_assert!(p.is_valid());
        }

        #[test]
        fn toml_round_trip_preserves_any_name(name in "[a-zA-Z0-9 _-]{0,20}") {
            let mut p = demo_project();
            p.name = name;
            let back = Project::from_toml(&p.to_toml().unwrap()).unwrap();
            prop_assert_eq!(back, p);
        }
    }
}
