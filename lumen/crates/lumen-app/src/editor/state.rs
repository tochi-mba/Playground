//! Editor state and every edit operation, kept free of egui so it can be tested headless.

use crate::autosave::{self, Autosave};
use lumen_control::OscServer;
use lumen_core::geom::Vec2;
use lumen_core::mask::Mask;
use lumen_core::params::{ParamAddress, ParamValue};
use lumen_core::project::{is_valid_id, Output, Project, Source, SourceKind, Surface, TestPattern};
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
}

/// Payload of a [`SetGeometry`].
#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    /// A warp.
    Warp(WarpKind),
    /// A mask list.
    Masks(Vec<Mask>),
}

impl Command<Project> for SetGeometry {
    fn apply(&self, p: &mut Project) {
        if let Some(s) = p.surface_mut(&self.surface) {
            match &self.after {
                Geometry::Warp(w) => s.warp.clone_from(w),
                Geometry::Masks(m) => s.masks.clone_from(m),
            }
        }
    }
    fn revert(&self, p: &mut Project) {
        if let Some(s) = p.surface_mut(&self.surface) {
            match &self.before {
                Geometry::Warp(w) => s.warp.clone_from(w),
                Geometry::Masks(m) => s.masks.clone_from(m),
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

    /// A fresh id with `prefix` that no item of that kind uses.
    pub fn fresh_id(&self, prefix: &str) -> String {
        let taken = |id: &str| {
            self.project.sources.iter().any(|s| s.id == id)
                || self.project.outputs.iter().any(|o| o.id == id)
                || self.project.surfaces.iter().any(|s| s.id == id)
        };
        if !taken(prefix) && is_valid_id(prefix) {
            return prefix.to_string();
        }
        // Bounded: at most len+2 candidates can be taken.
        (2..self.project.sources.len()
            + self.project.outputs.len()
            + self.project.surfaces.len()
            + 3)
            .map(|n| format!("{prefix}-{n}"))
            .find(|id| !taken(id))
            .unwrap_or_else(|| prefix.to_string())
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
                crop: lumen_core::project::Crop::default(),
                warp: WarpKind::default(),
                masks: Vec::new(),
            });
        });
        self.selection = Selection::Surface { id, mask: None };
    }

    /// Adds a source.
    pub fn add_source(&mut self, kind: SourceKind) -> String {
        let prefix = match &kind {
            SourceKind::TestPattern { .. } => "pattern",
            SourceKind::Solid { .. } => "solid",
            SourceKind::Image { .. } => "image",
            SourceKind::Spout { .. } => "spout",
        };
        let id = self.fresh_id(prefix);
        let new_id = id.clone();
        self.edit(|p| p.sources.push(Source { id: new_id, kind }));
        self.reload_sources();
        id
    }

    /// Removes whatever is selected (a mask, a surface or an output with its surfaces).
    pub fn delete_selection(&mut self) {
        match self.selection.clone() {
            Selection::None => {}
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
        assert_eq!(e.fresh_id("bad id"), "bad id-2");
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
