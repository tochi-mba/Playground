//! Top bar, object tree, inspector, dialogs and the show-mode screen.

use super::{Editor, MonitorInfo, Selection};
use eframe::egui::{self, Color32, RichText};
use lumen_core::blend::BlendCurve;
use lumen_core::params::ParamValue;
use lumen_core::project::{BlendMode, Placement, SourceKind, TestPattern};
use lumen_core::warp::{BezierMesh, WarpKind};
use std::path::PathBuf;

/// Transient UI state for the panels.
#[derive(Debug, Default)]
pub struct PanelState {
    /// Open/Save-As path prompt, when shown.
    pub path_dialog: Option<PathDialog>,
    /// Text being typed into the path prompt.
    pub path_text: String,
    /// Text for a new image source path.
    pub image_path: String,
    /// Text for a new Spout sender name.
    pub spout_sender: String,
    /// Mesh size for "convert to Bezier".
    pub mesh_cols: u32,
    /// Mesh size for "convert to Bezier".
    pub mesh_rows: u32,
    /// Error from the last file operation.
    pub error: Option<String>,
}

/// Which file prompt is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathDialog {
    /// Open a project.
    Open,
    /// Save the project under a new name.
    SaveAs,
}

impl PanelState {
    /// Saves, or opens the Save As prompt when the project has no path.
    pub fn save(&mut self, editor: &mut Editor) {
        if editor.path.is_some() {
            if let Err(e) = editor.save() {
                self.error = Some(e);
            }
        } else {
            self.path_dialog = Some(PathDialog::SaveAs);
            self.path_text = "show.lumen.toml".into();
        }
    }
}

fn num(editor: &mut Editor, address: &str) -> f64 {
    editor
        .get_param(address)
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
}

/// A slider bound to a parameter address, routed through undo.
fn param_slider(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    label: &str,
    address: &str,
    range: std::ops::RangeInclusive<f64>,
) {
    let mut v = num(editor, address);
    let r = ui.add(egui::Slider::new(&mut v, range).text(label));
    if r.changed() {
        editor.set_param(address, ParamValue::Number(v));
    }
    if r.drag_stopped() || r.lost_focus() {
        editor.commit();
    }
}

fn param_toggle(ui: &mut egui::Ui, editor: &mut Editor, label: &str, address: &str) {
    let mut v = editor
        .get_param(address)
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if ui.checkbox(&mut v, label).changed() {
        editor.set_param(address, ParamValue::Bool(v));
        editor.commit();
    }
}

/// Menu and status strip.
pub fn top_bar(ui: &mut egui::Ui, editor: &mut Editor, state: &mut PanelState) {
    egui::Panel::top("lumen-top").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            if ui.button("New").clicked() {
                *editor = Editor::new(lumen_core::project::demo_project(), None);
            }
            if ui.button("Open…").clicked() {
                state.path_dialog = Some(PathDialog::Open);
                state.path_text = editor
                    .path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
            }
            if ui.button("Save").clicked() {
                state.save(editor);
            }
            if ui.button("Save As…").clicked() {
                state.path_dialog = Some(PathDialog::SaveAs);
                state.path_text = editor
                    .path
                    .as_ref()
                    .map_or_else(|| "show.lumen.toml".into(), |p| p.display().to_string());
            }
            ui.separator();
            if ui
                .add_enabled(editor.history.can_undo(), egui::Button::new("Undo"))
                .clicked()
            {
                editor.undo();
            }
            if ui
                .add_enabled(editor.history.can_redo(), egui::Button::new("Redo"))
                .clicked()
            {
                editor.redo();
            }
            ui.separator();
            if ui.button("Show mode (F10)").clicked() {
                editor.enter_show_mode();
            }
            param_toggle(ui, editor, "Blackout", "/show/blackout");
            param_slider(ui, editor, "Master", "/show/master", 0.0..=1.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(&editor.status).weak());
                if let Some(p) = &editor.last_patch {
                    ui.label(RichText::new(format!("OSC {p}")).weak());
                }
            });
        });
    });
}

/// Outputs, surfaces and sources tree.
pub fn left_panel(ui: &mut egui::Ui, editor: &mut Editor) {
    egui::Panel::left("lumen-left").default_size(230.0).show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Outputs");
                if ui.small_button("+ output").clicked() {
                    editor.add_output();
                }
            });
            let outputs: Vec<_> = editor.project.outputs.iter().map(|o| (o.id.clone(), o.enabled)).collect();
            for (id, _enabled) in outputs {
                let selected = editor.selection == Selection::Output(id.clone());
                ui.horizontal(|ui| {
                    let open = editor.open_outputs.contains(&id);
                    if ui.selectable_label(open, "win").on_hover_text("Toggle output window").clicked() {
                        editor.toggle_output_window(&id);
                    }
                    if ui.selectable_label(selected, &id).clicked() {
                        editor.selection = Selection::Output(id.clone());
                    }
                    if ui.small_button("+ surface").clicked() {
                        editor.add_surface(&id);
                    }
                });
                let surfaces: Vec<_> = editor.project.surfaces_for_output(&id).iter().map(|s| (s.id.clone(), s.visible)).collect();
                ui.indent(&id, |ui| {
                    for (sid, visible) in surfaces {
                        let selected = matches!(&editor.selection, Selection::Surface { id, .. } if *id == sid);
                        ui.horizontal(|ui| {
                            let mut v = visible;
                            if ui.checkbox(&mut v, "").on_hover_text("Show or hide").changed() {
                                editor.set_param(&format!("/surface/{sid}/visible"), ParamValue::Bool(v));
                                editor.commit();
                            }
                            if ui.selectable_label(selected, &sid).clicked() {
                                editor.selection = Selection::Surface { id: sid.clone(), mask: None };
                            }
                        });
                    }
                });
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.heading("Sources");
                ui.menu_button("+ source", |ui| {
                    for (label, pattern) in [("Grid", TestPattern::Grid), ("Bars", TestPattern::Bars), ("Checker", TestPattern::Checker), ("Ramp", TestPattern::Ramp)] {
                        if ui.button(format!("Pattern: {label}")).clicked() {
                            editor.add_source(SourceKind::TestPattern { pattern });
                            ui.close();
                        }
                    }
                    if ui.button("Solid white").clicked() {
                        editor.add_source(SourceKind::Solid { color: [1.0, 1.0, 1.0, 1.0] });
                        ui.close();
                    }
                });
            });
            let sources: Vec<_> = editor.project.sources.iter().map(|s| (s.id.clone(), describe(&s.kind))).collect();
            for (id, desc) in sources {
                ui.horizontal(|ui| {
                    ui.label(&id);
                    ui.label(RichText::new(desc).weak());
                    let used = editor.project.surfaces.iter().any(|s| s.source == id);
                    if ui.add_enabled(!used, egui::Button::new("×").small()).on_hover_text("Remove (unused sources only)").clicked() {
                        editor.edit(|p| p.sources.retain(|s| s.id != id));
                        editor.reload_sources();
                    }
                });
            }
        });
    });
}

fn describe(kind: &SourceKind) -> String {
    match kind {
        SourceKind::TestPattern { pattern } => format!("{pattern:?}").to_lowercase(),
        SourceKind::Solid { color } => {
            format!("solid {:.2} {:.2} {:.2}", color[0], color[1], color[2])
        }
        SourceKind::Image { path } => format!("image {path}"),
        SourceKind::Spout { sender } => format!(
            "spout {}",
            if sender.is_empty() {
                "(active)"
            } else {
                sender
            }
        ),
    }
}

/// Right-hand inspector for the selection.
pub fn inspector(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    monitors: &[MonitorInfo],
    state: &mut PanelState,
) {
    egui::Panel::right("lumen-right")
        .default_size(300.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| match editor.selection.clone() {
                Selection::None => show_inspector(ui, editor, state),
                Selection::Output(id) => output_inspector(ui, editor, &id, monitors),
                Selection::Surface { id, mask } => surface_inspector(ui, editor, &id, mask, state),
            });
        });
}

fn show_inspector(ui: &mut egui::Ui, editor: &mut Editor, state: &mut PanelState) {
    ui.heading("Show");
    let mut name = editor.project.name.clone();
    let label = ui.label("Name");
    if ui
        .text_edit_singleline(&mut name)
        .labelled_by(label.id)
        .lost_focus()
        && name != editor.project.name
    {
        editor.edit(|p| p.name = name);
    }
    param_toggle(ui, editor, "Blackout", "/show/blackout");
    param_slider(ui, editor, "Master", "/show/master", 0.0..=1.0);
    ui.separator();
    ui.label(match &editor.osc {
        Some(s) => format!("OSC listening on {}", s.local_addr()),
        None => "OSC off (start with --osc-port)".into(),
    });
    ui.label(RichText::new("Addresses: /show/blackout, /show/master, /surface/<id>/opacity|visible|z|corner/<n>/x|y, /output/<id>/enabled|brightness|contrast|gamma|blend/<edge>/width").weak().small());
    ui.separator();
    let label = ui.heading("Add image source");
    ui.text_edit_singleline(&mut state.image_path)
        .labelled_by(label.id);
    if ui.button("Add image").clicked() && !state.image_path.trim().is_empty() {
        editor.add_source(SourceKind::Image {
            path: state.image_path.trim().to_string(),
        });
        state.image_path.clear();
    }
    let label = ui.heading("Add Spout source");
    ui.text_edit_singleline(&mut state.spout_sender)
        .labelled_by(label.id);
    if ui.button("Add Spout receiver").clicked() {
        editor.add_source(SourceKind::Spout {
            sender: state.spout_sender.trim().to_string(),
        });
    }
    if let Some(e) = &state.error {
        ui.colored_label(Color32::LIGHT_RED, e);
    }
}

#[allow(clippy::too_many_lines)]
fn output_inspector(ui: &mut egui::Ui, editor: &mut Editor, id: &str, monitors: &[MonitorInfo]) {
    ui.heading(format!("Output {id}"));
    let Some(output) = editor.project.output(id).cloned() else {
        return;
    };
    ui.horizontal(|ui| {
        let (mut w, mut h) = (output.width, output.height);
        ui.label("Size");
        let rw = ui.add(egui::DragValue::new(&mut w).range(1..=16384));
        let rh = ui.add(egui::DragValue::new(&mut h).range(1..=16384));
        if (rw.lost_focus() || rh.lost_focus() || rw.drag_stopped() || rh.drag_stopped())
            && (w, h) != (output.width, output.height)
        {
            let id = id.to_string();
            editor.edit(move |p| {
                if let Some(o) = p.output_mut(&id) {
                    o.width = w;
                    o.height = h;
                }
            });
        }
    });
    param_toggle(
        ui,
        editor,
        "Enabled in show mode",
        &format!("/output/{id}/enabled"),
    );
    let open = editor.open_outputs.contains(&id.to_string());
    if ui
        .button(if open { "Close window" } else { "Open window" })
        .clicked()
    {
        editor.toggle_output_window(id);
    }
    ui.separator();
    ui.label("Placement");
    if let Some(p) = &output.placement {
        ui.label(format!(
            "{} at {}, {}",
            if p.monitor.is_empty() {
                "window"
            } else {
                &p.monitor
            },
            p.x,
            p.y
        ));
        if ui.button("Unplace (back to preview window)").clicked() {
            let id = id.to_string();
            editor.edit(move |p| {
                if let Some(o) = p.output_mut(&id) {
                    o.placement = None;
                }
            });
        }
    } else {
        ui.label(RichText::new("Not placed: the window opens as a draggable preview.").weak());
        if let Some((x, y)) = editor.window_positions.get(id).copied() {
            if ui.button("Use current window position").clicked() {
                let scale =
                    super::monitor_at(monitors, x as i32, y as i32).map_or(1.0, |m| m.scale);
                let placement = Placement {
                    monitor: String::new(),
                    x: (f64::from(x) * scale).round() as i32,
                    y: (f64::from(y) * scale).round() as i32,
                };
                let id = id.to_string();
                editor.edit(move |p| {
                    if let Some(o) = p.output_mut(&id) {
                        o.placement = Some(placement);
                    }
                });
            }
        }
    }
    for m in monitors {
        if ui
            .button(format!(
                "Fill monitor {} ({}x{} at {},{})",
                m.name, m.width, m.height, m.x, m.y
            ))
            .clicked()
        {
            let placement = Placement {
                monitor: m.name.clone(),
                x: m.x,
                y: m.y,
            };
            let (w, h) = (m.width, m.height);
            let id = id.to_string();
            editor.edit(move |p| {
                if let Some(o) = p.output_mut(&id) {
                    o.placement = Some(placement);
                    o.width = w;
                    o.height = h;
                }
            });
        }
    }
    ui.separator();
    ui.label("Colour");
    param_slider(
        ui,
        editor,
        "Brightness",
        &format!("/output/{id}/brightness"),
        0.0..=2.0,
    );
    param_slider(
        ui,
        editor,
        "Contrast",
        &format!("/output/{id}/contrast"),
        0.0..=2.0,
    );
    param_slider(
        ui,
        editor,
        "Gamma",
        &format!("/output/{id}/gamma"),
        0.5..=2.5,
    );
    ui.separator();
    ui.label("Edge blend");
    for edge in ["left", "right", "top", "bottom"] {
        ui.horizontal(|ui| {
            param_slider(
                ui,
                editor,
                edge,
                &format!("/output/{id}/blend/{edge}/width"),
                0.0..=0.5,
            );
            let current = match edge {
                "left" => output.blend.left.curve,
                "right" => output.blend.right.curve,
                "top" => output.blend.top.curve,
                _ => output.blend.bottom.curve,
            };
            let mut curve = current;
            egui::ComboBox::from_id_salt(("curve", id, edge))
                .selected_text(format!("{curve:?}"))
                .show_ui(ui, |ui| {
                    for c in [BlendCurve::Linear, BlendCurve::Smooth, BlendCurve::Cosine] {
                        ui.selectable_value(&mut curve, c, format!("{c:?}"));
                    }
                });
            if curve != current {
                let id = id.to_string();
                editor.edit(move |p| {
                    if let Some(o) = p.output_mut(&id) {
                        match edge {
                            "left" => o.blend.left.curve = curve,
                            "right" => o.blend.right.curve = curve,
                            "top" => o.blend.top.curve = curve,
                            _ => o.blend.bottom.curve = curve,
                        }
                    }
                });
            }
        });
    }
    param_slider(
        ui,
        editor,
        "Blend gamma",
        &format!("/output/{id}/blend/gamma"),
        1.0..=3.0,
    );
    ui.separator();
    if ui.button("Delete output").clicked() {
        editor.selection = Selection::Output(id.to_string());
        editor.delete_selection();
    }
}

#[allow(clippy::too_many_lines)]
fn surface_inspector(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    id: &str,
    mask: Option<usize>,
    state: &mut PanelState,
) {
    ui.heading(format!("Surface {id}"));
    let Some(surface) = editor.project.surface(id).cloned() else {
        return;
    };
    // Source.
    let mut source = surface.source.clone();
    egui::ComboBox::from_label("Source")
        .selected_text(&source)
        .show_ui(ui, |ui| {
            for s in &editor.project.sources {
                ui.selectable_value(&mut source, s.id.clone(), &s.id);
            }
        });
    if source != surface.source {
        let id = id.to_string();
        editor.edit(move |p| {
            if let Some(s) = p.surface_mut(&id) {
                s.source = source;
            }
        });
    }
    param_toggle(ui, editor, "Visible", &format!("/surface/{id}/visible"));
    param_slider(
        ui,
        editor,
        "Opacity",
        &format!("/surface/{id}/opacity"),
        0.0..=1.0,
    );
    param_slider(
        ui,
        editor,
        "Order (z)",
        &format!("/surface/{id}/z"),
        -10.0..=10.0,
    );
    let mut mode = surface.blend_mode;
    egui::ComboBox::from_label("Blend mode")
        .selected_text(format!("{mode:?}"))
        .show_ui(ui, |ui| {
            for m in [
                BlendMode::Normal,
                BlendMode::Add,
                BlendMode::Multiply,
                BlendMode::Screen,
            ] {
                ui.selectable_value(&mut mode, m, format!("{m:?}"));
            }
        });
    if mode != surface.blend_mode {
        let id = id.to_string();
        editor.edit(move |p| {
            if let Some(s) = p.surface_mut(&id) {
                s.blend_mode = mode;
            }
        });
    }
    ui.separator();
    ui.label("Crop (normalised source rect)");
    let mut crop = surface.crop;
    let mut changed = false;
    ui.horizontal(|ui| {
        for (label, v) in [
            ("x", &mut crop.x),
            ("y", &mut crop.y),
            ("w", &mut crop.w),
            ("h", &mut crop.h),
        ] {
            ui.label(label);
            let r = ui.add(egui::DragValue::new(v).speed(0.005).range(0.0..=1.0));
            changed |= r.drag_stopped() || r.lost_focus();
        }
    });
    if changed && crop != surface.crop {
        let id = id.to_string();
        editor.edit(move |p| {
            if let Some(s) = p.surface_mut(&id) {
                s.crop = crop;
            }
        });
    }
    ui.separator();
    ui.label("Warp");
    match &surface.warp {
        WarpKind::Quad(q) => {
            ui.label(
                RichText::new("Corner pin: drag the four handles. Shift-drag moves all.").weak(),
            );
            for (i, c) in q.corners.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("corner {i}"));
                    let (mut x, mut y) = (c.x, c.y);
                    let rx = ui.add(egui::DragValue::new(&mut x).speed(0.001).fixed_decimals(4));
                    let ry = ui.add(egui::DragValue::new(&mut y).speed(0.001).fixed_decimals(4));
                    if rx.changed() {
                        editor.set_param(
                            &format!("/surface/{id}/corner/{i}/x"),
                            ParamValue::Number(x),
                        );
                    }
                    if ry.changed() {
                        editor.set_param(
                            &format!("/surface/{id}/corner/{i}/y"),
                            ParamValue::Number(y),
                        );
                    }
                    if rx.drag_stopped() || ry.drag_stopped() || rx.lost_focus() || ry.lost_focus()
                    {
                        editor.commit();
                    }
                });
            }
            if state.mesh_cols == 0 {
                state.mesh_cols = 2;
                state.mesh_rows = 2;
            }
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut state.mesh_cols)
                        .range(1..=BezierMesh::MAX_PATCHES)
                        .prefix("cols "),
                );
                ui.add(
                    egui::DragValue::new(&mut state.mesh_rows)
                        .range(1..=BezierMesh::MAX_PATCHES)
                        .prefix("rows "),
                );
                if ui.button("Convert to Bezier mesh").clicked() {
                    match surface.warp.to_bezier(state.mesh_cols, state.mesh_rows) {
                        Ok(b) => {
                            editor.set_warp(id, WarpKind::Bezier(b));
                            editor.commit();
                        }
                        Err(e) => state.error = Some(e.to_string()),
                    }
                }
            });
        }
        WarpKind::Bezier(b) => {
            ui.label(
                RichText::new(format!(
                    "Bezier mesh {}×{} patches: drag control points.",
                    b.cols, b.rows
                ))
                .weak(),
            );
            if ui.button("Reset to flat mesh").clicked() {
                if let Ok(flat) = BezierMesh::flat(b.cols, b.rows) {
                    editor.set_warp(id, WarpKind::Bezier(flat));
                    editor.commit();
                }
            }
            if ui.button("Back to corner pin").clicked() {
                editor.set_warp(id, WarpKind::default());
                editor.commit();
            }
        }
    }
    if ui.button("Reset warp").clicked() {
        editor.set_warp(id, WarpKind::default());
        editor.commit();
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Masks");
        if ui.small_button("+ rectangle").clicked() {
            editor.add_mask(id);
        }
    });
    for (i, m) in surface.masks.iter().enumerate() {
        let selected = mask == Some(i);
        ui.horizontal(|ui| {
            if ui
                .selectable_label(
                    selected,
                    format!(
                        "mask {i} ({} pts, {})",
                        m.points.len(),
                        if m.subtract { "hole" } else { "window" }
                    ),
                )
                .clicked()
            {
                editor.selection = Selection::Surface {
                    id: id.to_string(),
                    mask: if selected { None } else { Some(i) },
                };
            }
        });
        if selected {
            let mut edited = m.clone();
            let mut changed = false;
            let r = ui.add(egui::Slider::new(&mut edited.feather, 0.0..=0.3).text("feather"));
            changed |= r.drag_stopped() || r.lost_focus();
            changed |= ui
                .checkbox(&mut edited.subtract, "hole (unchecked = window)")
                .changed();
            if ui.button("Add point").clicked() {
                let last = edited.points.last().copied().unwrap_or_default();
                let first = edited.points.first().copied().unwrap_or_default();
                edited.points.push(last.lerp(first, 0.5));
                changed = true;
            }
            if changed && edited != *m {
                let mut masks = surface.masks.clone();
                masks[i] = edited;
                editor.set_masks(id, masks);
                editor.commit();
            }
            if ui.button("Delete mask").clicked() {
                editor.selection = Selection::Surface {
                    id: id.to_string(),
                    mask: Some(i),
                };
                editor.delete_selection();
            }
        }
    }
    ui.separator();
    if ui.button("Delete surface").clicked() {
        editor.selection = Selection::Surface {
            id: id.to_string(),
            mask: None,
        };
        editor.delete_selection();
    }
    if let Some(e) = &state.error {
        ui.colored_label(Color32::LIGHT_RED, e);
    }
}

/// Modal prompts: file paths and autosave recovery.
pub fn dialogs(ctx: &egui::Context, editor: &mut Editor, state: &mut PanelState) {
    if let Some(auto) = editor.recovery.clone() {
        egui::Window::new("Recover autosave?")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!(
                    "An autosave newer than the project exists:\n{}",
                    auto.display()
                ));
                ui.horizontal(|ui| {
                    if ui.button("Recover").clicked() {
                        editor.recover();
                    }
                    if ui.button("Discard").clicked() {
                        editor.discard_recovery();
                    }
                });
            });
    }
    let Some(kind) = state.path_dialog else {
        return;
    };
    let title = match kind {
        PathDialog::Open => "Open project",
        PathDialog::SaveAs => "Save project as",
    };
    let mut close = false;
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            let label = ui.label("Path to a .lumen.toml file:");
            let r = ui
                .text_edit_singleline(&mut state.path_text)
                .labelled_by(label.id);
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                let action = match kind {
                    PathDialog::Open => "Open file",
                    PathDialog::SaveAs => "Save file",
                };
                if ui.button(action).clicked() || enter {
                    let path = PathBuf::from(state.path_text.trim());
                    let result = match kind {
                        PathDialog::Open => Editor::open(&path)
                            .map(|e| *editor = e)
                            .map_err(|e| e.to_string()),
                        PathDialog::SaveAs => editor.save_as(&path),
                    };
                    match result {
                        Ok(()) => {
                            state.error = None;
                            close = true;
                        }
                        Err(e) => state.error = Some(e),
                    }
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
            if let Some(e) = &state.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
        });
    if close {
        state.path_dialog = None;
    }
}

/// Full-window status shown while in show mode.
pub fn show_mode_screen(ui: &mut egui::Ui, editor: &Editor, fps: f32) {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(Color32::BLACK))
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.3);
                ui.label(
                    RichText::new("SHOW MODE")
                        .size(32.0)
                        .color(Color32::from_gray(200)),
                );
                ui.label(
                    RichText::new(&editor.project.name)
                        .size(18.0)
                        .color(Color32::from_gray(150)),
                );
                ui.add_space(12.0);
                ui.label(
                    RichText::new(format!(
                        "{} output window(s) · {fps:.0} fps",
                        editor.open_outputs.len()
                    ))
                    .color(Color32::from_gray(120)),
                );
                ui.label(
                    RichText::new(match &editor.osc {
                        Some(s) => format!("OSC on {}", s.local_addr()),
                        None => "OSC off".into(),
                    })
                    .color(Color32::from_gray(120)),
                );
                if editor.project.blackout {
                    ui.label(RichText::new("BLACKOUT").color(Color32::LIGHT_RED));
                }
                ui.add_space(12.0);
                ui.label(RichText::new(&editor.status).color(Color32::from_gray(100)));
            });
        });
}
