//! Edits to outputs, surfaces, sources and masks.

use super::{source_kind_for, AlignOp, Editor, Selection};
use lumen_core::geom::Vec2;
use lumen_core::mask::Mask;
use lumen_core::params::ParamValue;
use lumen_core::project::{Crop, Output, Overlay, Source, SourceKind, Surface, TestPattern};
use lumen_core::warp::WarpKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

impl Editor {
    /// Sets or clears the calibration overlay of an output.
    pub fn set_overlay(&mut self, id: &str, overlay: Option<Overlay>) -> bool {
        let id = id.to_string();
        self.edit(move |p| {
            if let Some(o) = p.output_mut(&id) {
                o.overlay = overlay;
            }
        })
    }

    /// Sets blend widths from the overlap of placed outputs. Returns how many edges changed.
    pub fn suggest_blends(&mut self) -> usize {
        let mut n = 0;
        self.edit(|p| n = p.suggest_blends());
        self.status = if n == 0 {
            "no overlapping placed outputs".into()
        } else {
            format!("set {n} blend edges from the overlaps")
        };
        n
    }

    /// Nudges an output's blend gamma (the gamma match tool), clamped to `1..=3`.
    pub fn adjust_blend_gamma(&mut self, id: &str, delta: f64) -> bool {
        let address = format!("/output/{id}/blend/gamma");
        let Some(ParamValue::Number(g)) = self.get_param(&address) else {
            return false;
        };
        let next = ((g + delta).clamp(1.0, 3.0) * 100.0).round() / 100.0;
        let ok = self.set_param(&address, ParamValue::Number(next));
        self.commit();
        ok
    }

    /// Moves a placed output on the virtual desktop by whole pixels.
    pub fn move_placement(&mut self, id: &str, dx: i32, dy: i32) -> bool {
        if dx == 0 && dy == 0 {
            return false;
        }
        let id = id.to_string();
        self.edit(move |p| {
            if let Some(o) = p.output_mut(&id) {
                if let Some(pl) = &mut o.placement {
                    pl.x = pl.x.saturating_add(dx);
                    pl.y = pl.y.saturating_add(dy);
                }
            }
        })
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
                overlay: None,
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
        monitors: &[super::super::MonitorInfo],
        skip_primary: bool,
    ) -> Vec<String> {
        let mut ids = Vec::new();
        let chosen: Vec<&super::super::MonitorInfo> = monitors
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
                    overlay: None,
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
}
