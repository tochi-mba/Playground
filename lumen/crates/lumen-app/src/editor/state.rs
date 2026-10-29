//! Editor state and every edit operation, kept free of egui so it can be tested headless.

use crate::autosave::{self, Autosave};
use lumen_control::OscServer;
use lumen_core::geom::Vec2;
use lumen_core::mask::Mask;
use lumen_core::params::{ParamAddress, ParamValue};
use lumen_core::project::{
    Crop, Output, Playback, Project, Source, SourceKind, Surface, TestPattern,
};
use lumen_core::undo::{Command, History, SetValue};
use lumen_core::warp::WarpKind;
use lumen_render::scene::MaskCache;
use lumen_sources::SourceSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
        let mut applied = 0;
        for patch in patches {
            match self.project.apply_patch(&patch) {
                Ok(()) => {
                    applied += 1;
                    self.last_patch = Some(format!("{} = {:?}", patch.address, patch.value));
                }
                Err(e) => log::debug!("OSC: {e}"),
            }
        }
        if applied > 0 {
            self.dirty = true;
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

    /// Adds an output and selects it.
    pub fn add_output(&mut self) {
        let id = self.fresh_id("output");
        let new_id = id.clone();
        self.edit(|p| {
            p.outputs.push(Output {
                id: new_id,
                width: 1920,
                height: 1080,
                enabled: true,
                blend: lumen_core::blend::EdgeBlend::none(),
                color: lumen_core::project::ColorAdjust::default(),
                placement: None,
            });
        });
        self.selection = Selection::Output(id);
    }

    /// Adds a surface on `output` using the first source (or a grid pattern if none).
    pub fn add_surface(&mut self, output: &str) {
        if self.project.sources.is_empty() {
            self.add_source(SourceKind::TestPattern {
                pattern: TestPattern::Grid,
            });
        }
        let source = self.project.sources[0].id.clone();
        let id = self.fresh_id("surface");
        let new_id = id.clone();
        let output = output.to_string();
        self.edit(|p| {
            p.surfaces.push(Surface {
                id: new_id,
                output,
                source,
                z: p.surfaces.len() as i32,
                visible: true,
                opacity: 1.0,
                blend_mode: lumen_core::project::BlendMode::Normal,
                crop: Crop::default(),
                warp: WarpKind::default(),
                masks: Vec::new(),
                locked: false,
            });
        });
        self.selection = Selection::Surface { id, mask: None };
    }

    /// Adds a source.
    pub fn add_source(&mut self, kind: SourceKind) -> String {
        let id = self.fresh_id(kind.label());
        let new_id = id.clone();
        self.edit(|p| p.sources.push(Source { id: new_id, kind }));
        self.reload_sources();
        id
    }

    /// Removes whatever is selected (a mask, a surface or an output with its surfaces),
    /// plus every extra Shift-selected surface.
    pub fn delete_selection(&mut self) {
        if !self.extra_selected.is_empty() {
            let mut ids = self.extra_selected.clone();
            if let Selection::Surface { id, mask: None } = &self.selection {
                ids.push(id.clone());
            }
            self.edit(|p| p.surfaces.retain(|s| !ids.contains(&s.id)));
            self.extra_selected.clear();
            if matches!(&self.selection, Selection::Surface { mask: None, .. }) {
                self.selection = Selection::None;
            }
            return;
        }
        match self.selection.clone() {
            Selection::None => {}
            Selection::Source(id) => {
                if self.project.surfaces.iter().any(|s| s.source == id) {
                    self.status = format!("{id} is still used by a surface");
                    return;
                }
                self.edit(|p| p.sources.retain(|s| s.id != id));
                self.reload_sources();
                self.selection = Selection::None;
            }
            Selection::Output(id) => {
                self.edit(|p| {
                    p.outputs.retain(|o| o.id != id);
                    p.surfaces.retain(|s| s.output != id);
                });
                self.open_outputs.retain(|o| *o != id);
                self.selection = Selection::None;
            }
            Selection::Surface { id, mask: Some(i) } => {
                if let Some(mut masks) = self.project.surface(&id).map(|s| s.masks.clone()) {
                    if i < masks.len() {
                        masks.remove(i);
                        self.set_masks(&id, masks);
                        self.commit();
                    }
                }
                self.selection = Selection::Surface { id, mask: None };
            }
            Selection::Surface { id, mask: None } => {
                self.edit(|p| p.surfaces.retain(|s| s.id != id));
                self.selection = Selection::None;
            }
        }
    }

    /// Adds a rectangular hole mask to a surface and selects it.
    pub fn add_mask(&mut self, surface: &str) {
        let Some(mut masks) = self.project.surface(surface).map(|s| s.masks.clone()) else {
            return;
        };
        masks.push(Mask {
            points: vec![
                Vec2::new(0.3, 0.3),
                Vec2::new(0.7, 0.3),
                Vec2::new(0.7, 0.7),
                Vec2::new(0.3, 0.7),
            ],
            feather: 0.02,
            subtract: true,
        });
        let index = masks.len() - 1;
        self.set_masks(surface, masks);
        self.commit();
        self.selection = Selection::Surface {
            id: surface.into(),
            mask: Some(index),
        };
    }

    /// Surfaces that count as selected: the primary plus the Shift-selected extras.
    pub fn selected_surfaces(&self) -> Vec<String> {
        let mut ids = self.extra_selected.clone();
        if let Selection::Surface { id, .. } = &self.selection {
            if !ids.contains(id) {
                ids.insert(0, id.clone());
            }
        }
        ids
    }

    /// Selects a surface; with `extend` it joins the current selection instead.
    pub fn select_surface(&mut self, id: &str, extend: bool) {
        if extend {
            if let Selection::Surface { id: current, .. } = &self.selection {
                if current == id {
                    return;
                }
                let current = current.clone();
                if !self.extra_selected.contains(&current) {
                    self.extra_selected.push(current);
                }
            }
            self.extra_selected.retain(|s| s != id);
            self.selection = Selection::Surface {
                id: id.to_string(),
                mask: None,
            };
        } else {
            self.extra_selected.clear();
            self.selection = Selection::Surface {
                id: id.to_string(),
                mask: None,
            };
        }
    }

    /// Duplicates a surface and selects the copy.
    pub fn duplicate_surface(&mut self, id: &str) -> Option<String> {
        if self.show_mode || self.project.surface(id).is_none() {
            return None;
        }
        let mut new_id = None;
        let id = id.to_string();
        self.edit(|p| new_id = p.duplicate_surface(&id));
        if let Some(n) = &new_id {
            self.selection = Selection::Surface {
                id: n.clone(),
                mask: None,
            };
            self.extra_selected.clear();
        }
        new_id
    }

    /// Locks or unlocks a surface.
    pub fn set_locked(&mut self, id: &str, locked: bool) {
        let id = id.to_string();
        self.edit(|p| {
            if let Some(s) = p.surface_mut(&id) {
                s.locked = locked;
            }
        });
    }

    /// True when the surface exists and is locked.
    pub fn is_locked(&self, id: &str) -> bool {
        self.project.surface(id).is_some_and(|s| s.locked)
    }

    /// Renames the selected kind of item, keeping the selection on it.
    pub fn rename(&mut self, old: &str, new: &str) -> Result<(), String> {
        if self.show_mode {
            return Err("locked in show mode".into());
        }
        let new = new.trim().to_string();
        if new == old {
            return Ok(());
        }
        let before = self.project.clone();
        let result = if self.project.surface(old).is_some() {
            self.project.rename_surface(old, &new)
        } else if self.project.output(old).is_some() {
            self.project.rename_output(old, &new)
        } else if self.project.source(old).is_some() {
            self.project.rename_source(old, &new)
        } else {
            return Err(format!("nothing is called {old:?}"));
        };
        match result {
            Ok(()) => {
                let after = std::mem::replace(&mut self.project, before);
                self.edit(move |p| *p = after);
                self.selection = match self.selection.clone() {
                    Selection::Surface { id, mask } if id == old => Selection::Surface {
                        id: new.clone(),
                        mask,
                    },
                    Selection::Output(id) if id == old => Selection::Output(new.clone()),
                    Selection::Source(id) if id == old => Selection::Source(new.clone()),
                    other => other,
                };
                for o in &mut self.open_outputs {
                    if *o == old {
                        o.clone_from(&new);
                    }
                }
                self.reload_sources();
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Moves a surface one step up (`+1`) or down (`-1`) in draw order among its output's
    /// surfaces, renumbering `z` so the order is unambiguous.
    pub fn move_surface(&mut self, id: &str, delta: i32) {
        let Some(output) = self.project.surface(id).map(|s| s.output.clone()) else {
            return;
        };
        let mut ordered: Vec<String> = self
            .project
            .surfaces_for_output(&output)
            .iter()
            .map(|s| s.id.clone())
            .collect();
        let Some(i) = ordered.iter().position(|s| s == id) else {
            return;
        };
        let j = (i as i32 + delta).clamp(0, ordered.len() as i32 - 1) as usize;
        if i == j {
            return;
        }
        let moved = ordered.remove(i);
        ordered.insert(j, moved);
        self.edit(|p| {
            for (z, sid) in ordered.iter().enumerate() {
                if let Some(s) = p.surface_mut(sid) {
                    s.z = z as i32;
                }
            }
        });
    }

    /// Sends a surface to the top or bottom of its output's stack.
    pub fn send_surface(&mut self, id: &str, to_front: bool) {
        let n = self.project.surfaces.len() as i32;
        self.move_surface(id, if to_front { n } else { -n });
    }

    /// Applies an Align operation to a surface's warp.
    pub fn align(&mut self, id: &str, op: AlignOp) -> bool {
        let Some(surface) = self.project.surface(id).cloned() else {
            return false;
        };
        let Some(output) = self.project.output(&surface.output) else {
            return false;
        };
        let output_aspect = f64::from(output.width.max(1)) / f64::from(output.height.max(1));
        let warp = match op {
            AlignOp::FlipHorizontal => surface.warp.flip_horizontal(),
            AlignOp::FlipVertical => surface.warp.flip_vertical(),
            AlignOp::RotateQuarter => surface.warp.rotate_quarter(output_aspect),
            AlignOp::FitOutput => WarpKind::default(),
            AlignOp::Centre => surface.warp.centred(),
            AlignOp::MatchSourceAspect => {
                let source_aspect = self
                    .sources
                    .frame(&surface.source)
                    .map_or(output_aspect, |f| {
                        f64::from(f.width.max(1)) / f64::from(f.height.max(1))
                    });
                let crop_aspect =
                    source_aspect * surface.crop.w.max(1e-6) / surface.crop.h.max(1e-6);
                WarpKind::fit_aspect(crop_aspect, output_aspect)
            }
        };
        let ok = self.set_warp(id, warp);
        self.commit();
        ok
    }

    /// Flashes every output's identity for three seconds.
    pub fn identify(&mut self, now: Instant) {
        self.identify_until = Some(now + Duration::from_secs(3));
    }

    /// True while an identify flash is running (and clears it once it has expired).
    pub fn identify_active(&mut self, now: Instant) -> bool {
        match self.identify_until {
            Some(t) if now < t => true,
            Some(_) => {
                self.identify_until = None;
                false
            }
            None => false,
        }
    }

    /// Creates one full-size, placed output per monitor (skipping the first, which is
    /// usually the editor's screen, when `skip_primary`), each with a grid surface. Returns
    /// the new output ids.
    pub fn create_outputs_for_monitors(
        &mut self,
        monitors: &[super::MonitorInfo],
        skip_primary: bool,
    ) -> Vec<String> {
        let mut ids = Vec::new();
        let chosen: Vec<&super::MonitorInfo> = monitors
            .iter()
            .skip(usize::from(skip_primary && monitors.len() > 1))
            .collect();
        if chosen.is_empty() {
            return ids;
        }
        if self.project.sources.is_empty() {
            self.add_source(SourceKind::TestPattern {
                pattern: TestPattern::Grid,
            });
        }
        let source = self.project.sources[0].id.clone();
        for (i, m) in chosen.iter().enumerate() {
            let id = self.fresh_id(&format!("projector-{}", i + 1));
            let surface_id = self.fresh_id(&format!("{id}-surface"));
            let (name, x, y, w, h) = (m.name.clone(), m.x, m.y, m.width, m.height);
            let (oid, sid, src) = (id.clone(), surface_id, source.clone());
            self.edit(move |p| {
                p.outputs.push(Output {
                    id: oid.clone(),
                    width: w.max(1),
                    height: h.max(1),
                    enabled: true,
                    blend: lumen_core::blend::EdgeBlend::none(),
                    color: lumen_core::project::ColorAdjust::default(),
                    placement: Some(lumen_core::project::Placement {
                        monitor: name,
                        x,
                        y,
                    }),
                });
                p.surfaces.push(Surface {
                    id: sid,
                    output: oid,
                    source: src,
                    z: 0,
                    visible: true,
                    opacity: 1.0,
                    blend_mode: lumen_core::project::BlendMode::Normal,
                    crop: Crop::default(),
                    warp: WarpKind::default(),
                    masks: Vec::new(),
                    locked: false,
                });
            });
            self.open_outputs.push(id.clone());
            ids.push(id);
        }
        if let Some(first) = ids.first() {
            self.selection = Selection::Output(first.clone());
        }
        self.history.clear();
        self.dirty = true;
        ids
    }

    /// Adds sources for dropped or picked files. Images become image sources; a surface is
    /// added for each on the selected output (or the first output). Returns how many were
    /// added.
    pub fn add_files(&mut self, paths: &[PathBuf]) -> usize {
        let mut added = 0;
        let target = match &self.selection {
            Selection::Output(id) => Some(id.clone()),
            Selection::Surface { id, .. } => self.project.surface(id).map(|s| s.output.clone()),
            Selection::None | Selection::Source(_) => {
                self.project.outputs.first().map(|o| o.id.clone())
            }
        };
        for path in paths {
            let Some(kind) = source_kind_for(path) else {
                self.status = format!("{}: unsupported file type", path.display());
                continue;
            };
            let stem = path
                .file_stem()
                .map_or_else(|| "file".into(), |s| s.to_string_lossy().into_owned());
            let id = self.fresh_id(&stem);
            let relative = self.relative_path(path);
            let kind = match kind {
                SourceKind::Image { .. } => SourceKind::Image { path: relative },
                SourceKind::Video { playback, .. } => SourceKind::Video {
                    path: relative,
                    playback,
                },
                SourceKind::Sequence { fps, playback, .. } => SourceKind::Sequence {
                    path: relative,
                    fps,
                    playback,
                },
                other => other,
            };
            self.edit(|p| {
                p.sources.push(Source {
                    id: id.clone(),
                    kind,
                });
            });
            if let Some(output) = &target {
                let sid = self.fresh_id(&format!("{id}-surface"));
                let (oid, src, sid_for_edit) = (output.clone(), id.clone(), sid.clone());
                let z = self.project.surfaces_for_output(output).len() as i32;
                self.edit(move |p| {
                    p.surfaces.push(Surface {
                        id: sid_for_edit,
                        output: oid,
                        source: src,
                        z,
                        visible: true,
                        opacity: 1.0,
                        blend_mode: lumen_core::project::BlendMode::Normal,
                        crop: Crop::default(),
                        warp: WarpKind::default(),
                        masks: Vec::new(),
                        locked: false,
                    });
                });
                self.selection = Selection::Surface {
                    id: sid,
                    mask: None,
                };
            }
            added += 1;
        }
        if added > 0 {
            self.reload_sources();
            self.status = format!("added {added} source(s)");
        }
        added
    }

    /// Path to store in the project: relative to the project folder when possible.
    pub fn relative_path(&self, path: &Path) -> String {
        let base = self.base_dir();
        let base = std::fs::canonicalize(&base).unwrap_or(base);
        let full = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        full.strip_prefix(&base)
            .unwrap_or(&full)
            .to_string_lossy()
            .into_owned()
    }

    /// Asks to do `action`; returns true when it may proceed now, false when the user must
    /// first decide about unsaved changes (the action is parked in `pending`).
    pub fn request(&mut self, action: Pending) -> bool {
        if self.dirty {
            self.pending = Some(action);
            false
        } else {
            true
        }
    }

    /// Resolves a pending action: `discard` drops the changes, otherwise they were saved.
    /// Returns the action to carry out, if any.
    pub fn resolve_pending(&mut self, proceed: bool) -> Option<Pending> {
        let action = self.pending.take();
        if proceed {
            action
        } else {
            None
        }
    }

    /// Saves to the current path.
    pub fn save(&mut self) -> Result<(), String> {
        let Some(path) = self.path.clone() else {
            return Err("no file name; use Save As".into());
        };
        self.save_as(&path)
    }

    /// Saves to `path` and adopts it.
    pub fn save_as(&mut self, path: &Path) -> Result<(), String> {
        self.project.save(path).map_err(|e| e.to_string())?;
        if self.path.as_deref() != Some(path) {
            self.path = Some(path.to_path_buf());
            self.reload_sources();
        }
        autosave::discard(path);
        self.dirty = false;
        self.autosave.mark_saved(Instant::now());
        self.status = format!("saved {}", path.display());
        Ok(())
    }

    /// Time until the next autosave would be due (zero when due or not applicable).
    pub fn autosave_due_in(&self, now: Instant) -> Duration {
        self.autosave.due_in(now)
    }

    /// Writes the autosave file if one is due. Returns true when it wrote.
    pub fn maybe_autosave(&mut self, now: Instant) -> bool {
        if self.show_mode || !self.autosave.due(self.dirty, now) {
            return false;
        }
        let Some(path) = &self.path else { return false };
        let auto = autosave::autosave_path(path);
        match self.project.save(&auto) {
            Ok(()) => {
                self.autosave.mark_saved(now);
                true
            }
            Err(e) => {
                self.status = format!("autosave failed: {e}");
                false
            }
        }
    }

    /// Enters show mode: edits locked, every enabled output window opened.
    pub fn enter_show_mode(&mut self) {
        self.history.commit();
        self.show_mode = true;
        self.escape_armed = None;
        self.open_outputs = self
            .project
            .outputs
            .iter()
            .filter(|o| o.enabled)
            .map(|o| o.id.clone())
            .collect();
        self.status = "show mode: press Escape twice to leave".into();
    }

    /// Leaves show mode.
    pub fn exit_show_mode(&mut self) {
        self.show_mode = false;
        self.escape_armed = None;
        self.status = "editing".into();
    }

    /// Handles an Escape press; in show mode two presses within `window` exit. Returns true
    /// when show mode was left.
    pub fn press_escape(&mut self, now: Instant, window: Duration) -> bool {
        if !self.show_mode {
            self.selection = Selection::None;
            return false;
        }
        match self.escape_armed {
            Some(t) if now.duration_since(t) <= window => {
                self.exit_show_mode();
                true
            }
            _ => {
                self.escape_armed = Some(now);
                self.status = "press Escape again to leave show mode".into();
                false
            }
        }
    }

    /// Replaces the project with the autosave on offer. Returns false when none is pending.
    pub fn recover(&mut self) -> bool {
        let Some(auto) = self.recovery.take() else {
            return false;
        };
        match Project::load(&auto) {
            Ok(p) => {
                self.project = p;
                self.history.clear();
                self.dirty = true;
                self.reload_sources();
                self.status = "recovered autosave; save to keep it".into();
                true
            }
            Err(e) => {
                self.status = format!("cannot recover: {e}");
                false
            }
        }
    }

    /// Declines the autosave on offer and deletes it.
    pub fn discard_recovery(&mut self) {
        if self.recovery.take().is_some() {
            if let Some(p) = &self.path {
                autosave::discard(p);
            }
        }
    }

    /// Toggles an output window.
    pub fn toggle_output_window(&mut self, id: &str) {
        if let Some(i) = self.open_outputs.iter().position(|o| o == id) {
            self.open_outputs.remove(i);
        } else {
            self.open_outputs.push(id.to_string());
        }
    }

    /// Window title.
    pub fn title(&self) -> String {
        let name = self
            .path
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(|| "untitled".into(), |n| n.to_string_lossy().into_owned());
        format!("{}{name} - lumen", if self.dirty { "*" } else { "" })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;
    use lumen_core::project::demo_project;
    use lumen_core::warp::{BezierMesh, Quad};

    fn editor() -> Editor {
        Editor::new(demo_project(), None)
    }

    #[test]
    fn params_go_through_undo_with_coalescing() {
        let mut e = editor();
        assert!(e.set_param("/surface/logo/opacity", ParamValue::Number(0.5)));
        assert!(e.set_param("/surface/logo/opacity", ParamValue::Number(0.3)));
        assert_eq!(
            e.get_param("/surface/logo/opacity"),
            Some(ParamValue::Number(0.3))
        );
        assert_eq!(e.history.undo_len(), 1, "drag coalesced");
        assert!(e.dirty);
        e.commit();
        assert!(e.set_param("/surface/logo/opacity", ParamValue::Number(0.9)));
        assert_eq!(e.history.undo_len(), 2);
        assert!(e.undo());
        assert_eq!(
            e.get_param("/surface/logo/opacity"),
            Some(ParamValue::Number(0.3))
        );
        assert!(e.undo());
        assert_eq!(
            e.get_param("/surface/logo/opacity"),
            Some(ParamValue::Number(0.8))
        );
        assert!(!e.undo());
        assert!(e.redo());
        assert!(
            e.set_param("/surface/logo/opacity", ParamValue::Number(0.3)),
            "no-op when unchanged"
        );
        assert!(!e.set_param("/nope", ParamValue::Number(1.0)));
        assert!(!e.set_param("bad", ParamValue::Number(1.0)));
    }

    #[test]
    fn geometry_commands_coalesce_and_revert() {
        let mut e = editor();
        let mut q = Quad::full_frame();
        q.set_corner(0, Vec2::new(0.1, 0.1));
        assert!(e.set_warp("wall-right", WarpKind::Quad(q)));
        q.set_corner(0, Vec2::new(0.2, 0.2));
        assert!(e.set_warp("wall-right", WarpKind::Quad(q)));
        assert_eq!(e.history.undo_len(), 1);
        assert!(
            e.set_warp("wall-right", WarpKind::Quad(q)),
            "identical warp is a no-op"
        );
        assert!(!e.set_warp("ghost", WarpKind::default()));
        e.commit();
        let mesh = BezierMesh::flat(2, 2).unwrap();
        assert!(e.set_warp("wall-right", WarpKind::Bezier(mesh.clone())));
        assert_eq!(e.history.undo_len(), 2);
        e.undo();
        assert_eq!(
            e.project.surface("wall-right").unwrap().warp,
            WarpKind::Quad(q)
        );
        e.undo();
        assert_eq!(
            e.project.surface("wall-right").unwrap().warp,
            WarpKind::default()
        );

        // Masks are a separate coalescing stream from warps.
        let m = vec![Mask::hole(vec![
            Vec2::ZERO,
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
        ])];
        assert!(e.set_masks("wall-right", m.clone()));
        assert!(
            e.set_masks("wall-right", m.clone()),
            "identical masks are a no-op"
        );
        assert!(!e.set_masks("ghost", m.clone()));
        assert!(e.set_warp("wall-right", WarpKind::Bezier(mesh)));
        assert!(e.set_masks("wall-right", vec![]));
        assert_eq!(
            e.history.undo_len(),
            3,
            "warp between two mask edits splits them"
        );
        e.undo();
        assert_eq!(e.project.surface("wall-right").unwrap().masks, m);
    }

    #[test]
    fn structural_edits_are_single_snapshots() {
        let mut e = editor();
        e.add_output();
        assert_eq!(e.selection, Selection::Output("output".into()));
        assert_eq!(e.project.outputs.len(), 3);
        e.add_output();
        assert_eq!(e.project.outputs[3].id, "output-2");
        e.add_surface("output");
        assert_eq!(
            e.selection,
            Selection::Surface {
                id: "surface".into(),
                mask: None
            }
        );
        assert_eq!(e.project.surfaces.last().unwrap().output, "output");
        let n = e.history.undo_len();
        assert!(e.edit(|_| {}), "no change is still ok");
        assert_eq!(e.history.undo_len(), n);
        e.delete_selection();
        assert_eq!(e.project.surfaces.len(), 3);
        e.selection = Selection::Output("output".into());
        e.open_outputs.push("output".into());
        e.delete_selection();
        assert_eq!(e.project.outputs.len(), 3);
        assert!(e.open_outputs.is_empty());
        assert_eq!(e.selection, Selection::None);
        e.undo();
        assert_eq!(e.project.outputs.len(), 4);
        e.selection = Selection::None;
        e.delete_selection();
    }

    #[test]
    fn masks_add_and_delete() {
        let mut e = editor();
        e.add_mask("wall-left");
        assert_eq!(
            e.selection,
            Selection::Surface {
                id: "wall-left".into(),
                mask: Some(0)
            }
        );
        assert_eq!(e.project.surface("wall-left").unwrap().masks.len(), 1);
        e.add_mask("ghost");
        e.delete_selection();
        assert!(e.project.surface("wall-left").unwrap().masks.is_empty());
        assert_eq!(
            e.selection,
            Selection::Surface {
                id: "wall-left".into(),
                mask: None
            }
        );
        e.selection = Selection::Surface {
            id: "wall-left".into(),
            mask: Some(7),
        };
        e.delete_selection();
        assert_eq!(
            e.selection,
            Selection::Surface {
                id: "wall-left".into(),
                mask: None
            }
        );
    }

    #[test]
    fn sources_add_with_fresh_ids_and_reload() {
        let mut e = Editor::new(Project::default(), None);
        e.add_surface("main");
        assert_eq!(
            e.project.sources.len(),
            1,
            "a surface on an empty project gets a pattern"
        );
        let id = e.add_source(SourceKind::Solid {
            color: [1.0, 0.0, 0.0, 1.0],
        });
        assert_eq!(id, "solid");
        assert_eq!(
            e.add_source(SourceKind::Solid { color: [0.0; 4] }),
            "solid-2"
        );
        assert_eq!(
            e.add_source(SourceKind::Image {
                path: "missing.png".into()
            }),
            "image"
        );
        assert!(e.status.contains("failed to open"));
        assert_eq!(
            e.add_source(SourceKind::Spout {
                sender: String::new()
            }),
            "spout"
        );
        assert!(e.sources.frame("solid").is_some());
        assert_eq!(e.fresh_id("bad id"), "badid");
    }

    #[test]
    fn show_mode_locks_edits_and_double_escape_exits() {
        let mut e = editor();
        e.enter_show_mode();
        assert!(e.show_mode);
        assert_eq!(e.open_outputs, ["left", "right"]);
        assert!(!e.set_param("/surface/logo/opacity", ParamValue::Number(0.1)));
        assert!(!e.set_warp("logo", WarpKind::default()));
        assert!(!e.set_masks("logo", vec![]));
        assert!(!e.edit(|p| p.name.clear()));
        assert!(!e.undo() && !e.redo());
        let t = Instant::now();
        assert!(!e.press_escape(t, Duration::from_secs(2)));
        assert!(
            !e.press_escape(t + Duration::from_secs(3), Duration::from_secs(2)),
            "too late, re-armed"
        );
        assert!(e.press_escape(t + Duration::from_secs(4), Duration::from_secs(2)));
        assert!(!e.show_mode);
        e.selection = Selection::Output("left".into());
        assert!(!e.press_escape(t, Duration::from_secs(2)));
        assert_eq!(
            e.selection,
            Selection::None,
            "escape clears the selection when editing"
        );
    }

    #[test]
    fn save_load_autosave_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = editor();
        assert!(e.save().is_err(), "no path yet");
        let path = dir.path().join("show.lumen.toml");
        e.set_param("/show/master", ParamValue::Number(0.5));
        e.save_as(&path).unwrap();
        assert!(!e.dirty);
        assert_eq!(e.path.as_deref(), Some(path.as_path()));
        assert!(e.title().starts_with("show.lumen.toml"));
        let reopened = Editor::open(&path).unwrap();
        assert_eq!(reopened.project.master, 0.5);
        assert!(Editor::open(&dir.path().join("nope.toml")).is_err());

        // Autosave only when dirty and due.
        let now = Instant::now();
        assert!(!e.maybe_autosave(now + Duration::from_mins(1)), "clean");
        e.set_param("/show/master", ParamValue::Number(0.25));
        assert!(e.title().starts_with('*'));
        assert!(!e.maybe_autosave(now), "not due");
        assert!(e.maybe_autosave(now + Duration::from_mins(1)));
        let auto = autosave::autosave_path(&path);
        assert!(auto.exists());
        assert_eq!(Project::load(&auto).unwrap().master, 0.25);
        e.save().unwrap();
        assert!(!auto.exists(), "clean save discards the autosave");
        e.enter_show_mode();
        e.dirty = true;
        assert!(
            !e.maybe_autosave(now + Duration::from_mins(10)),
            "never in show mode"
        );
        let mut untitled = editor();
        untitled.dirty = true;
        assert!(
            !untitled.maybe_autosave(now + Duration::from_mins(10)),
            "no path"
        );
        let mut bad = editor();
        bad.path = Some(dir.path().join("missing-dir").join("x").join("y.toml"));
        std::fs::write(dir.path().join("missing-dir"), b"file").unwrap();
        bad.dirty = true;
        assert!(!bad.maybe_autosave(now + Duration::from_mins(10)));
        assert!(bad.status.contains("autosave failed"));
    }

    #[test]
    fn osc_patches_apply_live() {
        let mut e = editor();
        assert_eq!(e.poll_osc(), 0, "no server");
        e.set_osc_port(0);
        assert!(e.osc.is_none());
        e.set_osc_port(0);
        // Bind to an ephemeral port by asking for 0 through the server directly.
        e.osc = Some(OscServer::bind("127.0.0.1:0").unwrap());
        let addr = e.osc.as_ref().unwrap().local_addr();
        let client = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let msg = rosc_packet("/show/blackout", 1.0);
        client.send_to(&msg, addr).unwrap();
        client
            .send_to(&rosc_packet("/surface/ghost/opacity", 1.0), addr)
            .unwrap();
        let mut applied = 0;
        for _ in 0..50 {
            applied += e.poll_osc();
            if applied > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(applied, 1);
        assert!(e.project.blackout);
        assert!(e.dirty);
        assert!(e
            .last_patch
            .as_deref()
            .unwrap()
            .starts_with("/show/blackout"));
    }

    fn rosc_packet(addr: &str, v: f32) -> Vec<u8> {
        // Hand-encode a float OSC message so the test does not need the rosc crate.
        fn pad(mut s: Vec<u8>) -> Vec<u8> {
            s.push(0);
            while !s.len().is_multiple_of(4) {
                s.push(0);
            }
            s
        }
        let mut out = pad(addr.as_bytes().to_vec());
        out.extend(pad(b",f".to_vec()));
        out.extend(v.to_be_bytes());
        out
    }

    #[test]
    fn recovery_flow() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.lumen.toml");
        let mut e = editor();
        e.save_as(&path).unwrap();
        assert!(!e.recover(), "nothing pending");
        let auto = autosave::autosave_path(&path);
        let mut changed = demo_project();
        changed.master = 0.1;
        changed.save(&auto).unwrap();
        e.recovery = Some(auto.clone());
        assert!(e.recover());
        assert_eq!(e.project.master, 0.1);
        assert!(e.dirty);
        e.recovery = Some(auto.clone());
        e.discard_recovery();
        assert!(!auto.exists());
        e.recovery = Some(dir.path().join("missing.autosave"));
        assert!(!e.recover());
        assert!(e.status.contains("cannot recover"));
    }

    #[test]
    fn multi_select_and_duplicate_and_lock() {
        let mut e = editor();
        e.select_surface("logo", false);
        e.select_surface("wall-right", true);
        assert_eq!(e.selected_surfaces(), ["wall-right", "logo"]);
        e.select_surface("wall-right", true);
        assert_eq!(
            e.selected_surfaces(),
            ["wall-right", "logo"],
            "re-selecting the primary is a no-op"
        );
        e.select_surface("logo", true);
        assert_eq!(
            e.selected_surfaces(),
            ["logo", "wall-right"],
            "swapped roles, still two"
        );
        e.select_surface("wall-left", false);
        assert_eq!(e.selected_surfaces(), ["wall-left"]);
        e.select_surface("logo", true);
        e.delete_selection();
        assert!(e.project.surface("logo").is_none() && e.project.surface("wall-left").is_none());
        assert_eq!(e.selection, Selection::None);
        assert!(e.extra_selected.is_empty());
        e.undo();
        assert_eq!(e.project.surfaces.len(), 3);

        let copy = e.duplicate_surface("logo").unwrap();
        assert_eq!(copy, "logo-copy");
        assert_eq!(
            e.selection,
            Selection::Surface {
                id: copy.clone(),
                mask: None
            }
        );
        assert!(e.duplicate_surface("ghost").is_none());
        e.set_locked(&copy, true);
        assert!(e.is_locked(&copy) && !e.is_locked("logo") && !e.is_locked("ghost"));
        e.enter_show_mode();
        assert!(e.duplicate_surface("logo").is_none());
    }

    #[test]
    fn rename_updates_selection_and_windows() {
        let mut e = editor();
        e.selection = Selection::Surface {
            id: "logo".into(),
            mask: Some(0),
        };
        e.rename("logo", " badge ").unwrap();
        assert_eq!(
            e.selection,
            Selection::Surface {
                id: "badge".into(),
                mask: Some(0)
            }
        );
        assert!(e.project.surface("badge").is_some());
        e.open_outputs.push("left".into());
        e.selection = Selection::Output("left".into());
        e.rename("left", "L").unwrap();
        assert_eq!(e.selection, Selection::Output("L".into()));
        assert_eq!(e.open_outputs, ["L"]);
        assert_eq!(e.project.surfaces[0].output, "L");
        e.rename("grid", "g").unwrap();
        assert!(e.sources.frame("g").is_some());
        assert!(e.rename("g", "bad id").is_err());
        assert!(e.rename("ghost", "x").is_err());
        assert!(e.rename("g", "g").is_ok());
        assert!(e.undo(), "a rename is one undo step");
        assert!(e.project.source("grid").is_some());
        e.enter_show_mode();
        assert!(e.rename("grid", "x").is_err());
    }

    #[test]
    fn ordering_and_alignment() {
        let mut e = editor();
        let order = |e: &Editor| {
            e.project
                .surfaces_for_output("right")
                .iter()
                .map(|s| s.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(order(&e), ["wall-right", "logo"]);
        e.move_surface("logo", -1);
        assert_eq!(order(&e), ["logo", "wall-right"]);
        e.move_surface("logo", -1);
        assert_eq!(order(&e), ["logo", "wall-right"], "already at the bottom");
        e.send_surface("logo", true);
        assert_eq!(order(&e), ["wall-right", "logo"]);
        e.send_surface("logo", false);
        assert_eq!(order(&e), ["logo", "wall-right"]);
        e.move_surface("ghost", 1);

        for op in [
            AlignOp::FlipHorizontal,
            AlignOp::FlipVertical,
            AlignOp::RotateQuarter,
            AlignOp::Centre,
            AlignOp::MatchSourceAspect,
            AlignOp::FitOutput,
        ] {
            assert!(e.align("logo", op), "{op:?}");
            assert!(e.project.is_valid());
        }
        assert_eq!(e.project.surface("logo").unwrap().warp, WarpKind::default());
        assert!(!e.align("ghost", AlignOp::Centre));
        let mut orphan = editor();
        orphan.project.surfaces[2].output = "ghost".into();
        assert!(!orphan.align("logo", AlignOp::Centre));
        let mut no_frame = editor();
        no_frame.project.surfaces[2].source = "ghost".into();
        assert!(
            no_frame.align("logo", AlignOp::MatchSourceAspect),
            "falls back to the output aspect"
        );
    }

    #[test]
    fn identify_and_monitor_outputs() {
        let mut e = Editor::new(Project::default(), None);
        let t = Instant::now();
        assert!(!e.identify_active(t));
        e.identify(t);
        assert!(e.identify_active(t + Duration::from_secs(1)));
        assert!(!e.identify_active(t + Duration::from_secs(4)));
        assert!(e.identify_until.is_none());

        let monitors = vec![
            super::super::MonitorInfo {
                name: "a".into(),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 1.0,
            },
            super::super::MonitorInfo {
                name: "b".into(),
                x: 1920,
                y: 0,
                width: 1280,
                height: 720,
                scale: 1.0,
            },
            super::super::MonitorInfo {
                name: "c".into(),
                x: 3200,
                y: 0,
                width: 0,
                height: 0,
                scale: 1.0,
            },
        ];
        let ids = e.create_outputs_for_monitors(&monitors, true);
        assert_eq!(ids, ["projector-1", "projector-2"]);
        assert_eq!(
            e.project.outputs[0].placement.as_ref().unwrap().monitor,
            "b"
        );
        assert_eq!(
            (e.project.outputs[1].width, e.project.outputs[1].height),
            (1, 1),
            "zero sizes are clamped"
        );
        assert_eq!(e.project.surfaces.len(), 2);
        assert_eq!(e.open_outputs, ids);
        assert_eq!(e.selection, Selection::Output("projector-1".into()));
        assert!(e.dirty && !e.history.can_undo());
        assert!(e.project.is_valid());
        let more = e.create_outputs_for_monitors(&monitors[..1], true);
        assert_eq!(
            more,
            ["projector-1-2"],
            "a single monitor is used even when skipping the primary"
        );
        assert!(e.create_outputs_for_monitors(&[], false).is_empty());
    }

    #[test]
    fn files_become_sources_and_pending_actions() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("photo.PNG");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
            .save(&img)
            .unwrap();
        let mut e = editor();
        e.path = Some(dir.path().join("show.lumen.toml"));
        e.selection = Selection::Output("right".into());
        assert_eq!(e.add_files(&[img.clone(), dir.path().join("notes.txt")]), 1);
        assert!(e.status.contains("unsupported") || e.status.contains("added"));
        let src = e.project.source("photo").unwrap();
        assert_eq!(
            src.kind,
            SourceKind::Image {
                path: "photo.PNG".into()
            },
            "stored relative to the project"
        );
        assert_eq!(
            e.selection,
            Selection::Surface {
                id: "photo-surface".into(),
                mask: None
            }
        );
        assert_eq!(e.project.surface("photo-surface").unwrap().output, "right");
        assert!(e.sources.frame("photo").is_some());
        assert!(matches!(
            source_kind_for(Path::new("x.mov")),
            Some(SourceKind::Video { .. })
        ));
        assert!(matches!(
            source_kind_for(Path::new("x.GIF")),
            Some(SourceKind::Sequence { .. })
        ));
        assert!(
            matches!(
                source_kind_for(dir.path()),
                Some(SourceKind::Sequence { .. })
            ),
            "folders are sequences"
        );
        assert!(source_kind_for(Path::new("x.txt")).is_none());
        assert!(source_kind_for(Path::new("noext")).is_none());
        let mut empty = Editor::new(Project::default(), None);
        assert_eq!(empty.add_files(&[img]), 1, "no output: source only");
        assert!(empty.project.surfaces.is_empty());

        assert!(!e.request(Pending::Close), "add_files dirtied the project");
        e.pending = None;
        e.dirty = false;
        assert!(e.request(Pending::New));
        e.dirty = true;
        assert!(!e.request(Pending::Open(dir.path().join("o.toml"))));
        assert!(e.pending.is_some());
        assert_eq!(e.resolve_pending(false), None);
        assert!(e.pending.is_none());
        e.request(Pending::Close);
        assert_eq!(e.resolve_pending(true), Some(Pending::Close));
    }

    #[test]
    fn output_window_toggle_and_misc() {
        let mut e = editor();
        e.toggle_output_window("left");
        assert_eq!(e.open_outputs, ["left"]);
        e.toggle_output_window("left");
        assert!(e.open_outputs.is_empty());
        assert_eq!(e.base_dir(), PathBuf::from("."));
        e.exit_show_mode();
        assert_eq!(e.status, "editing");
        assert_eq!(e.title(), "untitled - lumen");
        let s = Snapshot {
            before: Project::default(),
            after: demo_project(),
        };
        assert!(Command::<Project>::coalesce_key(&s).is_none());
        let mut g = SetGeometry {
            surface: "a".into(),
            what: GeometryKind::Warp,
            before: Geometry::Warp(WarpKind::default()),
            after: Geometry::Warp(WarpKind::default()),
        };
        assert!(!Command::<Project>::merge(&mut g, &s));
        let other = SetGeometry {
            surface: "b".into(),
            ..g.clone()
        };
        assert!(!Command::<Project>::merge(&mut g, &other));
        // Geometry commands on a vanished surface are harmless.
        let mut p = Project::default();
        g.apply(&mut p);
        g.revert(&mut p);
        assert_eq!(p, Project::default());
    }
}
