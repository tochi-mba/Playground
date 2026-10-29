//! Files, pending actions, autosave, show mode and recovery.

use super::{Editor, Pending, Selection};
use crate::autosave::{self};
use lumen_core::project::Project;
use std::path::Path;
use std::time::{Duration, Instant};

impl Editor {
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
