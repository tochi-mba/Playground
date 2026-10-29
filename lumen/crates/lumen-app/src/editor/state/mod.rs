//! Editor state and every edit operation, kept free of egui so it can be tested headless.

use crate::autosave::{self, Autosave};
use lumen_control::feedback::OscFeedback;
use lumen_control::midi::MidiListener;
use lumen_control::web::RemoteServer;
use lumen_control::OscServer;
use lumen_core::cues::CueRunner;
use lumen_core::mask::Mask;
use lumen_core::midimap::{MidiEvent, MidiMode};
use lumen_core::params::{ParamAddress, ParamPatch, ParamValue};
use lumen_core::project::{Crop, Playback, Project, SourceKind};
use lumen_core::undo::{Command, History, SetValue};
use lumen_core::warp::WarpKind;
use lumen_render::scene::MaskCache;
use lumen_sources::SourceSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// What the inspector shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Selection {
    /// Nothing; the show panel is shown.
    #[default]
    None,
    /// An output.
    Output(String),
    /// A surface, and optionally one of its masks.
    Surface {
        /// Surface id.
        id: String,
        /// Selected mask index.
        mask: Option<usize>,
    },
    /// A source.
    Source(String),
}

/// Replaces a surface's whole warp or mask list; coalesces while a handle is dragged.
#[derive(Debug, Clone)]
pub struct SetGeometry {
    /// Surface id.
    pub surface: String,
    /// Which geometry.
    pub what: GeometryKind,
    /// Before.
    pub before: Geometry,
    /// After.
    pub after: Geometry,
}

/// Which part of a surface a [`SetGeometry`] edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryKind {
    /// The warp.
    Warp,
    /// The masks.
    Masks,
    /// The source crop.
    Crop,
}

/// Payload of a [`SetGeometry`].
#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    /// A warp.
    Warp(WarpKind),
    /// A mask list.
    Masks(Vec<Mask>),
    /// A crop rectangle.
    Crop(Crop),
}

impl Command<Project> for SetGeometry {
    fn apply(&self, p: &mut Project) {
        if let Some(s) = p.surface_mut(&self.surface) {
            match &self.after {
                Geometry::Warp(w) => s.warp.clone_from(w),
                Geometry::Masks(m) => s.masks.clone_from(m),
                Geometry::Crop(c) => s.crop = *c,
            }
        }
    }
    fn revert(&self, p: &mut Project) {
        if let Some(s) = p.surface_mut(&self.surface) {
            match &self.before {
                Geometry::Warp(w) => s.warp.clone_from(w),
                Geometry::Masks(m) => s.masks.clone_from(m),
                Geometry::Crop(c) => s.crop = *c,
            }
        }
    }
    fn coalesce_key(&self) -> Option<String> {
        Some(format!("geometry:{}:{:?}", self.surface, self.what))
    }
    fn merge(&mut self, later: &dyn Command<Project>) -> bool {
        match later.as_any().downcast_ref::<Self>() {
            Some(l) if l.surface == self.surface && l.what == self.what => {
                self.after = l.after.clone();
                true
            }
            _ => false,
        }
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Snapshot command for structural edits (add/remove things): stores whole projects.
#[derive(Debug, Clone)]
pub struct Snapshot {
    before: Project,
    after: Project,
}

impl Command<Project> for Snapshot {
    fn apply(&self, p: &mut Project) {
        *p = self.after.clone();
    }
    fn revert(&self, p: &mut Project) {
        *p = self.before.clone();
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Source kind for a file or folder picked or dropped, by extension.
pub fn source_kind_for(path: &Path) -> Option<SourceKind> {
    let text = path.to_string_lossy().into_owned();
    if path.is_dir() {
        return Some(SourceKind::Sequence {
            path: text,
            fps: 25.0,
            playback: Playback::default(),
        });
    }
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" => Some(SourceKind::Image { path: text }),
        "gif" => Some(SourceKind::Sequence {
            path: text,
            fps: 25.0,
            playback: Playback::default(),
        }),
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" | "mpg" | "mpeg" | "wmv" => {
            Some(SourceKind::Video {
                path: text,
                playback: Playback::default(),
            })
        }
        _ => None,
    }
}

/// File extensions the media dialog offers.
pub const MEDIA_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "mp4", "mov", "mkv", "webm", "avi", "m4v", "mpg", "mpeg", "wmv",
];

/// The editor's model.
pub struct Editor {
    /// Current project.
    pub project: Project,
    /// Where it lives, once saved.
    pub path: Option<PathBuf>,
    /// Undo history.
    pub history: History<Project>,
    /// Inspector selection.
    pub selection: Selection,
    /// Unsaved changes.
    pub dirty: bool,
    /// Locked show mode.
    pub show_mode: bool,
    /// Autosave timer.
    pub autosave: Autosave,
    /// OSC server when enabled.
    pub osc: Option<OscServer>,
    /// Last status line.
    pub status: String,
    /// Opened sources.
    pub sources: SourceSet,
    /// Mask raster cache for scene building.
    pub mask_cache: MaskCache,
    /// Output windows the user has opened (ids).
    pub open_outputs: Vec<String>,
    /// Time of the last Escape press in show mode (double-tap exits).
    pub escape_armed: Option<Instant>,
    /// Patches received from OSC since the last frame, for display.
    pub last_patch: Option<String>,
    /// An autosave newer than the project, offered for recovery.
    pub recovery: Option<PathBuf>,
    /// Last known outer position (points) of each open output window.
    pub window_positions: std::collections::HashMap<String, (f32, f32)>,
    /// Outputs flash their id and colour until this instant.
    pub identify_until: Option<Instant>,
    /// Additional surfaces selected with Shift-click (the primary is in `selection`).
    pub extra_selected: Vec<String>,
    /// An action waiting for the user to decide about unsaved changes.
    pub pending: Option<Pending>,
    /// Handles are drawn on the output windows while editing.
    pub edit_on_wall: bool,
    /// Cue playback.
    pub cues: CueRunner,
    /// Selected cue in the cue list.
    pub selected_cue: Option<usize>,
    /// Open MIDI port.
    pub midi: Option<MidiListener>,
    /// Address (and mode) waiting for the next MIDI message to map it.
    pub midi_learn: Option<(String, MidiMode)>,
    /// Last MIDI message seen, for display.
    pub last_midi: Option<MidiEvent>,
    /// OSC feedback destination.
    pub feedback: Option<OscFeedback>,
    /// Phone remote server.
    pub remote: Option<RemoteServer>,
}

/// Something the user asked for that would lose unsaved changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    /// Quit the application.
    Close,
    /// Start a new project.
    New,
    /// Open this project.
    Open(PathBuf),
}

/// Geometry operations offered by the Align menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignOp {
    /// Mirror left/right.
    FlipHorizontal,
    /// Mirror top/bottom.
    FlipVertical,
    /// Quarter turn clockwise.
    RotateQuarter,
    /// Cover the whole output.
    FitOutput,
    /// Move to the output centre.
    Centre,
    /// Largest centred rectangle with the source's aspect ratio.
    MatchSourceAspect,
}

impl Editor {
    /// Creates an editor around a project.
    pub fn new(project: Project, path: Option<PathBuf>) -> Self {
        let base = path
            .as_deref()
            .and_then(Path::parent)
            .unwrap_or(Path::new("."))
            .to_path_buf();
        let (mut sources, errors) = SourceSet::open(&project, &base);
        sources.poll();
        let status = if errors.is_empty() {
            String::from("ready")
        } else {
            format!("{} source(s) failed to open", errors.len())
        };
        Self {
            project,
            path,
            history: History::default(),
            selection: Selection::None,
            dirty: false,
            show_mode: false,
            autosave: Autosave::new(autosave::DEFAULT_INTERVAL),
            osc: None,
            status,
            sources: SourceSet::open(&Project::default(), &base).0,
            mask_cache: MaskCache::default(),
            open_outputs: Vec::new(),
            escape_armed: None,
            last_patch: None,
            recovery: None,
            window_positions: std::collections::HashMap::new(),
            identify_until: None,
            extra_selected: Vec::new(),
            pending: None,
            edit_on_wall: true,
            cues: CueRunner::default(),
            selected_cue: None,
            midi: None,
            midi_learn: None,
            last_midi: None,
            feedback: None,
            remote: None,
        }
        .with_sources(sources)
    }

    fn with_sources(mut self, sources: SourceSet) -> Self {
        self.sources = sources;
        self
    }

    /// Opens a project file.
    pub fn open(path: &Path) -> Result<Self, lumen_core::project::ProjectError> {
        let project = Project::load(path)?;
        Ok(Self::new(project, Some(path.to_path_buf())))
    }

    /// Folder relative paths resolve against.
    pub fn base_dir(&self) -> PathBuf {
        self.path
            .as_deref()
            .and_then(Path::parent)
            .unwrap_or(Path::new("."))
            .to_path_buf()
    }

    /// Re-opens every source (after sources were added, removed or edited).
    pub fn reload_sources(&mut self) {
        let (mut sources, errors) = SourceSet::open(&self.project, &self.base_dir());
        sources.poll();
        self.sources = sources;
        if !errors.is_empty() {
            self.status = format!("{} source(s) failed to open", errors.len());
        }
    }

    /// Starts (or restarts) the OSC server on `port`; `0` disables.
    pub fn set_osc_port(&mut self, port: u16) {
        self.osc = None;
        if port == 0 {
            return;
        }
        match OscServer::bind(("0.0.0.0", port)) {
            Ok(s) => {
                self.status = format!("OSC on port {}", s.local_addr().port());
                self.osc = Some(s);
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    /// Applies pending OSC patches directly (no undo: live control is not an edit).
    pub fn poll_osc(&mut self) -> usize {
        let Some(osc) = &self.osc else { return 0 };
        let patches = osc.drain();
        let now = self.sources.time();
        let mut applied = 0;
        for patch in patches {
            if self.handle_control(&patch, now) {
                applied += 1;
            }
        }
        applied
    }

    /// Sets a parameter through the undo history, coalescing drags until [`Editor::commit`].
    pub fn set_param(&mut self, address: &str, value: ParamValue) -> bool {
        if self.show_mode {
            return false;
        }
        let Ok(addr) = ParamAddress::parse(address) else {
            return false;
        };
        let Some(before) = self.project.get_param(&addr) else {
            return false;
        };
        if before == value {
            return true;
        }
        if let Some(fb) = &self.feedback {
            fb.send(&ParamPatch {
                address: addr,
                value: value.clone(),
            });
        }
        self.history.push(
            &mut self.project,
            Box::new(SetValue {
                key: address.to_string(),
                before,
                after: value,
            }),
        );
        self.dirty = true;
        true
    }

    /// Reads a parameter.
    pub fn get_param(&self, address: &str) -> Option<ParamValue> {
        ParamAddress::parse(address)
            .ok()
            .and_then(|a| self.project.get_param(&a))
    }

    /// Replaces a surface's warp through the history.
    pub fn set_warp(&mut self, surface: &str, warp: WarpKind) -> bool {
        if self.show_mode {
            return false;
        }
        let Some(before) = self.project.surface(surface).map(|s| s.warp.clone()) else {
            return false;
        };
        if before == warp {
            return true;
        }
        self.history.push(
            &mut self.project,
            Box::new(SetGeometry {
                surface: surface.into(),
                what: GeometryKind::Warp,
                before: Geometry::Warp(before),
                after: Geometry::Warp(warp),
            }),
        );
        self.dirty = true;
        true
    }

    /// Replaces a surface's crop through the history (coalesces while dragging).
    pub fn set_crop(&mut self, surface: &str, crop: Crop) -> bool {
        if self.show_mode {
            return false;
        }
        let Some(before) = self.project.surface(surface).map(|s| s.crop) else {
            return false;
        };
        if before == crop {
            return true;
        }
        self.history.push(
            &mut self.project,
            Box::new(SetGeometry {
                surface: surface.into(),
                what: GeometryKind::Crop,
                before: Geometry::Crop(before),
                after: Geometry::Crop(crop),
            }),
        );
        self.dirty = true;
        true
    }

    /// Edits a source's kind in place (transport, generator settings, paths) as one undo step
    /// and pushes the change into the live source without reopening files where possible.
    pub fn edit_source(&mut self, id: &str, f: impl FnOnce(&mut SourceKind)) -> bool {
        if self.show_mode || self.project.source(id).is_none() {
            return false;
        }
        let before = self.project.source(id).map(|s| s.kind.clone());
        let id_owned = id.to_string();
        let changed = self.edit(move |p| {
            if let Some(s) = p.sources.iter_mut().find(|s| s.id == id_owned) {
                f(&mut s.kind);
            }
        });
        let after = self.project.source(id).map(|s| s.kind.clone());
        if before != after {
            let reopen = match (&before, &after) {
                (Some(b), Some(a)) => {
                    b.path() != a.path() || std::mem::discriminant(b) != std::mem::discriminant(a)
                }
                _ => true,
            };
            if reopen {
                self.reload_sources();
            } else {
                self.sources.sync(&self.project);
            }
        }
        changed
    }

    /// Replaces a surface's masks through the history.
    pub fn set_masks(&mut self, surface: &str, masks: Vec<Mask>) -> bool {
        if self.show_mode {
            return false;
        }
        let Some(before) = self.project.surface(surface).map(|s| s.masks.clone()) else {
            return false;
        };
        if before == masks {
            return true;
        }
        self.history.push(
            &mut self.project,
            Box::new(SetGeometry {
                surface: surface.into(),
                what: GeometryKind::Masks,
                before: Geometry::Masks(before),
                after: Geometry::Masks(masks),
            }),
        );
        self.dirty = true;
        true
    }

    /// Applies a structural edit as one undo step.
    pub fn edit(&mut self, f: impl FnOnce(&mut Project)) -> bool {
        if self.show_mode {
            return false;
        }
        let before = self.project.clone();
        let mut after = before.clone();
        f(&mut after);
        if after == before {
            return true;
        }
        self.history.commit();
        self.history
            .push(&mut self.project, Box::new(Snapshot { before, after }));
        self.history.commit();
        self.dirty = true;
        true
    }

    /// Ends a drag so the next edit is a new undo step.
    pub fn commit(&mut self) {
        self.history.commit();
    }

    /// Undo.
    pub fn undo(&mut self) -> bool {
        if self.show_mode {
            return false;
        }
        let did = self.history.undo(&mut self.project);
        if did {
            self.dirty = true;
        }
        did
    }

    /// Redo.
    pub fn redo(&mut self) -> bool {
        if self.show_mode {
            return false;
        }
        let did = self.history.redo(&mut self.project);
        if did {
            self.dirty = true;
        }
        did
    }

    /// A fresh id with `prefix` that nothing in the project uses.
    pub fn fresh_id(&self, prefix: &str) -> String {
        self.project.fresh_id(prefix)
    }
}

mod control;
mod objects;
mod session;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod control_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests;
