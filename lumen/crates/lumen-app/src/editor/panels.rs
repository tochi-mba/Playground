//! Toolbar, status bar, object tree, inspector, dialogs and the show-mode screen.

use super::canvas::CanvasState;
use super::thumbs::Thumbnails;
use super::{AlignOp, Editor, MonitorInfo, Pending, Selection};
use crate::settings::Settings;
use eframe::egui::{self, Color32, RichText};
use lumen_core::blend::BlendCurve;
use lumen_core::params::ParamValue;
use lumen_core::project::{BlendMode, GeneratorKind, Placement, SourceKind, TestPattern};
use lumen_core::warp::{BezierMesh, WarpKind};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Native file dialogs, behind a trait so tests can script them.
pub trait FileDialogs {
    /// Choose a project to open.
    fn pick_project(&mut self) -> Option<PathBuf>;
    /// Choose where to save a project.
    fn save_project(&mut self, suggested: Option<&Path>) -> Option<PathBuf>;
    /// Choose media files to add as sources.
    fn pick_media(&mut self) -> Vec<PathBuf>;
    /// Choose a folder of image frames.
    fn pick_folder(&mut self) -> Option<PathBuf>;
}

/// The real dialogs.
#[derive(Debug, Default)]
pub struct NativeDialogs;

impl FileDialogs for NativeDialogs {
    fn pick_project(&mut self) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .add_filter("lumen project", &["toml"])
            .pick_file()
    }
    fn save_project(&mut self, suggested: Option<&Path>) -> Option<PathBuf> {
        let mut d = rfd::FileDialog::new().add_filter("lumen project", &["toml"]);
        if let Some(p) = suggested {
            if let Some(dir) = p.parent() {
                d = d.set_directory(dir);
            }
            if let Some(name) = p.file_name() {
                d = d.set_file_name(name.to_string_lossy());
            }
        } else {
            d = d.set_file_name("show.lumen.toml");
        }
        d.save_file()
    }
    fn pick_media(&mut self) -> Vec<PathBuf> {
        rfd::FileDialog::new()
            .add_filter("media", super::state::MEDIA_EXTENSIONS)
            .pick_files()
            .unwrap_or_default()
    }
    fn pick_folder(&mut self) -> Option<PathBuf> {
        rfd::FileDialog::new().pick_folder()
    }
}

/// Scripted dialogs for tests.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct FakeDialogs {
    /// Returned by the next `pick_project`.
    pub project: Option<PathBuf>,
    /// Returned by the next `save_project`.
    pub save: Option<PathBuf>,
    /// Returned by the next `pick_media`.
    pub media: Vec<PathBuf>,
    /// Returned by the next `pick_folder`.
    pub folder: Option<PathBuf>,
}

#[cfg(test)]
impl FileDialogs for FakeDialogs {
    fn pick_project(&mut self) -> Option<PathBuf> {
        self.project.take()
    }
    fn save_project(&mut self, _suggested: Option<&Path>) -> Option<PathBuf> {
        self.save.take()
    }
    fn pick_media(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.media)
    }
    fn pick_folder(&mut self) -> Option<PathBuf> {
        self.folder.take()
    }
}

/// Things the panels ask the app to do; the app runs them after the frame.
#[derive(Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct Actions {
    /// Save (or Save As when untitled).
    pub save: bool,
    /// Save As.
    pub save_as: bool,
    /// Open through the dialog.
    pub open: bool,
    /// Open this path.
    pub open_path: Option<PathBuf>,
    /// New project.
    pub new: bool,
    /// Add media through the dialog.
    pub add_media: bool,
    /// Add an image-sequence folder through the dialog.
    pub add_folder: bool,
    /// Create outputs for the detected monitors.
    pub create_outputs: bool,
    /// The unsaved-changes prompt was answered: proceed or cancel.
    pub resolve_pending: Option<bool>,
    /// Never show the welcome screen again.
    pub skip_welcome: bool,
    /// New ffmpeg path to remember and use.
    pub ffmpeg_path: Option<String>,
}

/// Transient UI state for the panels.
pub struct PanelState {
    /// File dialogs (native, or scripted in tests).
    pub dialogs: Box<dyn FileDialogs>,
    /// The welcome screen is up.
    pub welcome: bool,
    /// The keyboard cheat sheet is up.
    pub help: bool,
    /// Text for a new Spout sender name.
    pub spout_sender: String,
    /// Mesh size for "convert to Bezier".
    pub mesh_cols: u32,
    /// Mesh size for "convert to Bezier".
    pub mesh_rows: u32,
    /// Error from the last file operation.
    pub error: Option<String>,
    /// Id being edited in the rename box.
    pub rename: Option<(String, String)>,
    /// Thumbnail textures.
    pub thumbs: Thumbnails,
    /// Whether ffmpeg was found at startup.
    pub ffmpeg_ok: bool,
    /// ffmpeg path field.
    pub ffmpeg_path: String,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            dialogs: Box::new(NativeDialogs),
            welcome: false,
            help: false,
            spout_sender: String::new(),
            mesh_cols: 2,
            mesh_rows: 2,
            error: None,
            rename: None,
            thumbs: Thumbnails::default(),
            ffmpeg_ok: lumen_sources::video::Tools::from_env().available(),
            ffmpeg_path: std::env::var("LUMEN_FFMPEG").unwrap_or_default(),
        }
    }
}

impl std::fmt::Debug for PanelState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PanelState")
            .field("welcome", &self.welcome)
            .field("help", &self.help)
            .finish_non_exhaustive()
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

/// Toolbar.
#[allow(clippy::too_many_lines)]
pub fn top_bar(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut PanelState,
    canvas: &mut CanvasState,
    actions: &mut Actions,
) {
    egui::Panel::top("lumen-top").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New  (Ctrl+N)").clicked() {
                    actions.new = true;
                    ui.close();
                }
                if ui.button("Open…  (Ctrl+O)").clicked() {
                    actions.open = true;
                    ui.close();
                }
                if ui.button("Save  (Ctrl+S)").clicked() {
                    actions.save = true;
                    ui.close();
                }
                if ui.button("Save As…  (Ctrl+Shift+S)").clicked() {
                    actions.save_as = true;
                    ui.close();
                }
                ui.separator();
                if ui.button("Add media…").clicked() {
                    actions.add_media = true;
                    ui.close();
                }
                ui.separator();
                if ui.button("Welcome screen").clicked() {
                    state.welcome = true;
                    ui.close();
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui
                    .add_enabled(
                        editor.history.can_undo(),
                        egui::Button::new("Undo  (Ctrl+Z)"),
                    )
                    .clicked()
                {
                    editor.undo();
                    ui.close();
                }
                if ui
                    .add_enabled(
                        editor.history.can_redo(),
                        egui::Button::new("Redo  (Ctrl+Y)"),
                    )
                    .clicked()
                {
                    editor.redo();
                    ui.close();
                }
                ui.separator();
                let has_surface = matches!(editor.selection, Selection::Surface { .. });
                if ui
                    .add_enabled(
                        has_surface,
                        egui::Button::new("Duplicate surface  (Ctrl+D)"),
                    )
                    .clicked()
                {
                    if let Selection::Surface { id, .. } = editor.selection.clone() {
                        editor.duplicate_surface(&id);
                    }
                    ui.close();
                }
                if ui
                    .add_enabled(
                        editor.selection != Selection::None,
                        egui::Button::new("Delete  (Del)"),
                    )
                    .clicked()
                {
                    editor.delete_selection();
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut canvas.show_grid, "Grid  (G)");
                ui.checkbox(&mut canvas.handles.snap, "Snap  (S)");
                ui.checkbox(&mut editor.edit_on_wall, "Handles on output windows");
                ui.add(
                    egui::Slider::new(&mut canvas.grid_divisions, 2..=32).text("grid divisions"),
                );
                if ui.button("Fit view  (0)").clicked() {
                    canvas.reset_view();
                    ui.close();
                }
                if ui.button("Keyboard shortcuts  (?)").clicked() {
                    state.help = true;
                    ui.close();
                }
            });
            ui.separator();
            if ui
                .add_enabled(editor.history.can_undo(), egui::Button::new("Undo"))
                .on_hover_text("Ctrl+Z")
                .clicked()
            {
                editor.undo();
            }
            if ui
                .add_enabled(editor.history.can_redo(), egui::Button::new("Redo"))
                .on_hover_text("Ctrl+Y")
                .clicked()
            {
                editor.redo();
            }
            ui.separator();
            if ui
                .button("Identify")
                .on_hover_text("Flash each output's name on its projector (I)")
                .clicked()
            {
                editor.identify(Instant::now());
            }
            if ui
                .button("Show mode")
                .on_hover_text("Lock editing and open every output (F10)")
                .clicked()
            {
                editor.enter_show_mode();
            }
            let blackout = editor.project.blackout;
            if ui
                .add(
                    egui::Button::new(if blackout { "BLACKOUT" } else { "Blackout" }).fill(
                        if blackout {
                            Color32::from_rgb(120, 20, 20)
                        } else {
                            Color32::TRANSPARENT
                        },
                    ),
                )
                .on_hover_text("All outputs black (B)")
                .clicked()
            {
                editor.set_param("/show/blackout", ParamValue::Bool(!blackout));
                editor.commit();
            }
            param_slider(ui, editor, "Master", "/show/master", 0.0..=1.0);
        });
    });
}

/// Bottom status strip.
pub fn status_bar(
    ui: &mut egui::Ui,
    editor: &Editor,
    canvas: &CanvasState,
    fps: f32,
    now: Instant,
) {
    egui::Panel::bottom("lumen-status").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&editor.status).weak());
            let selected = editor.selected_surfaces();
            if selected.len() > 1 {
                ui.label(RichText::new(format!("{} surfaces selected", selected.len())).weak());
            }
            ui.separator();
            ui.label(
                RichText::new(match &editor.osc {
                    Some(s) => format!("OSC {}", s.local_addr().port()),
                    None => "OSC off".into(),
                })
                .weak(),
            );
            if let Some(p) = &editor.last_patch {
                ui.label(RichText::new(p).weak().small());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{fps:.0} fps")).weak());
                ui.separator();
                ui.label(RichText::new(format!("{:.0}%", canvas.zoom * 100.0)).weak())
                    .on_hover_text("Scroll to zoom, middle-drag to pan, 0 to fit");
                ui.separator();
                ui.label(
                    RichText::new(if canvas.handles.snap {
                        "snap on"
                    } else {
                        "snap off"
                    })
                    .weak(),
                );
                ui.separator();
                if editor.dirty {
                    let secs = editor.autosave_due_in(now).as_secs();
                    ui.label(
                        RichText::new(if editor.path.is_some() {
                            format!("unsaved · autosave in {secs}s")
                        } else {
                            "unsaved".into()
                        })
                        .weak(),
                    );
                } else {
                    ui.label(RichText::new("saved").weak());
                }
            });
        });
    });
}

/// Outputs, surfaces and sources tree.
#[allow(clippy::too_many_lines)]
pub fn left_panel(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut PanelState,
    actions: &mut Actions,
) {
    egui::Panel::left("lumen-left").default_size(240.0).show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Outputs");
                if ui.small_button("+ output").on_hover_text("Add a projector").clicked() {
                    editor.add_output();
                }
            });
            let outputs: Vec<_> = editor.project.outputs.iter().map(|o| o.id.clone()).collect();
            for (index, id) in outputs.iter().enumerate() {
                let selected = editor.selection == Selection::Output(id.clone());
                ui.horizontal(|ui| {
                    let open = editor.open_outputs.contains(id);
                    if ui.selectable_label(open, "win").on_hover_text("Toggle the output window").clicked() {
                        editor.toggle_output_window(id);
                    }
                    let color = super::handles::output_color(index);
                    let r = ui.selectable_label(selected, RichText::new(id).color(color));
                    if r.clicked() {
                        editor.selection = Selection::Output(id.clone());
                        editor.extra_selected.clear();
                    }
                    r.context_menu(|ui| {
                        if ui.button("Add surface").clicked() {
                            editor.add_surface(id);
                            ui.close();
                        }
                        if ui.button(if open { "Close window" } else { "Open window" }).clicked() {
                            editor.toggle_output_window(id);
                            ui.close();
                        }
                        if ui.button("Identify outputs").clicked() {
                            editor.identify(Instant::now());
                            ui.close();
                        }
                        if ui.button("Delete output").clicked() {
                            editor.selection = Selection::Output(id.clone());
                            editor.delete_selection();
                            ui.close();
                        }
                    });
                    if ui.small_button("+ surface").clicked() {
                        editor.add_surface(id);
                    }
                });
                let surfaces: Vec<_> = editor.project.surfaces_for_output(id).iter().map(|s| (s.id.clone(), s.visible, s.locked)).collect();
                ui.indent(id, |ui| {
                    for (sid, visible, locked) in surfaces {
                        let selected = matches!(&editor.selection, Selection::Surface { id, .. } if *id == sid) || editor.extra_selected.contains(&sid);
                        ui.horizontal(|ui| {
                            let mut v = visible;
                            if ui.checkbox(&mut v, "").on_hover_text("Show or hide").changed() {
                                editor.set_param(&format!("/surface/{sid}/visible"), ParamValue::Bool(v));
                                editor.commit();
                            }
                            let text = if locked { format!("{sid} [locked]") } else { sid.clone() };
                            let r = ui.selectable_label(selected, text);
                            if r.clicked() {
                                let shift = ui.input(|i| i.modifiers.shift);
                                editor.select_surface(&sid, shift);
                            }
                            r.context_menu(|ui| surface_context_menu(ui, editor, &sid, locked));
                        });
                    }
                });
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.heading("Sources");
                ui.menu_button("+ source", |ui| {
                    if ui.button("Media file…").clicked() {
                        actions.add_media = true;
                        ui.close();
                    }
                    if ui.button("Image sequence folder…").clicked() {
                        actions.add_folder = true;
                        ui.close();
                    }
                    ui.separator();
                    for g in GeneratorKind::ALL {
                        if ui.button(format!("Generator: {g:?}")).clicked() {
                            editor.add_source(SourceKind::Generator { pattern: g, speed: 1.0, color: [1.0; 4] });
                            ui.close();
                        }
                    }
                    ui.separator();
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
                let selected = editor.selection == Selection::Source(id.clone());
                ui.horizontal(|ui| {
                    if let Some(frame) = editor.sources.frame(&id).cloned() {
                        let (tex, size) = state.thumbs.get(ui.ctx(), &id, &frame);
                        let h = 28.0;
                        let w = (h * size.0 as f32 / size.1.max(1) as f32).clamp(8.0, 64.0);
                        ui.add(egui::Image::new((tex, egui::Vec2::new(w, h))));
                    }
                    if ui.selectable_label(selected, &id).clicked() {
                        editor.selection = Selection::Source(id.clone());
                        editor.extra_selected.clear();
                    }
                    ui.label(RichText::new(desc).weak().small());
                    let used = editor.project.surfaces.iter().any(|s| s.source == id);
                    if ui.add_enabled(!used, egui::Button::new("×").small()).on_hover_text("Remove (unused sources only)").clicked() {
                        editor.edit(|p| p.sources.retain(|s| s.id != id));
                        editor.reload_sources();
                    }
                });
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Drop images, videos, GIFs or frame folders anywhere to add them.").weak().small());
        });
    });
}

fn surface_context_menu(ui: &mut egui::Ui, editor: &mut Editor, sid: &str, locked: bool) {
    if ui.button("Duplicate  (Ctrl+D)").clicked() {
        editor.duplicate_surface(sid);
        ui.close();
    }
    if ui
        .button(if locked { "Unlock  (L)" } else { "Lock  (L)" })
        .clicked()
    {
        editor.set_locked(sid, !locked);
        ui.close();
    }
    if ui.button("Bring to front").clicked() {
        editor.send_surface(sid, true);
        ui.close();
    }
    if ui.button("Send to back").clicked() {
        editor.send_surface(sid, false);
        ui.close();
    }
    if ui.button("Reset warp to full frame").clicked() {
        editor.set_warp(sid, WarpKind::default());
        editor.commit();
        ui.close();
    }
    if ui.button("Delete").clicked() {
        editor.selection = Selection::Surface {
            id: sid.to_string(),
            mask: None,
        };
        editor.extra_selected.clear();
        editor.delete_selection();
        ui.close();
    }
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
        SourceKind::Video { path, playback } => format!(
            "video {path}{}",
            if playback.paused { " (paused)" } else { "" }
        ),
        SourceKind::Sequence { path, .. } => format!("sequence {path}"),
        SourceKind::Generator { pattern, .. } => format!("generator {pattern:?}").to_lowercase(),
    }
}

/// Inline id editor shared by the inspectors. Returns true when a rename happened.
fn rename_box(ui: &mut egui::Ui, editor: &mut Editor, state: &mut PanelState, id: &str) -> bool {
    let mut renamed = false;
    ui.horizontal(|ui| {
        let label = ui.label("Id");
        let editing = state.rename.as_ref().is_some_and(|(orig, _)| orig == id);
        let mut text = if editing {
            state
                .rename
                .as_ref()
                .map(|(_, t)| t.clone())
                .unwrap_or_default()
        } else {
            id.to_string()
        };
        let r = ui.text_edit_singleline(&mut text).labelled_by(label.id);
        if r.changed() {
            state.rename = Some((id.to_string(), text.clone()));
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (enter || (r.lost_focus() && editing)) && text != id {
            match editor.rename(id, &text) {
                Ok(()) => {
                    state.error = None;
                    renamed = true;
                }
                Err(e) => state.error = Some(e),
            }
            state.rename = None;
        }
    });
    renamed
}

/// Right-hand inspector for the selection.
pub fn inspector(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    monitors: &[MonitorInfo],
    state: &mut PanelState,
    actions: &mut Actions,
) {
    egui::Panel::right("lumen-right")
        .default_size(310.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| match editor.selection.clone() {
                Selection::None => show_inspector(ui, editor, state, actions),
                Selection::Output(id) => output_inspector(ui, editor, &id, monitors, state),
                Selection::Surface { id, mask } => surface_inspector(ui, editor, &id, mask, state),
                Selection::Source(id) => source_inspector(ui, editor, &id, state),
            });
        });
}

fn show_inspector(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut PanelState,
    actions: &mut Actions,
) {
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
    ui.horizontal(|ui| {
        if ui.button("Add media…").clicked() {
            actions.add_media = true;
        }
        if ui.button("Add frame folder…").clicked() {
            actions.add_folder = true;
        }
    });
    ui.separator();
    ui.label(if state.ffmpeg_ok {
        "ffmpeg found: video sources play."
    } else {
        "ffmpeg not found: install it or point lumen at it below."
    });
    ui.horizontal(|ui| {
        let label = ui.label("ffmpeg path");
        let r = ui
            .text_edit_singleline(&mut state.ffmpeg_path)
            .labelled_by(label.id);
        if r.lost_focus() {
            actions.ffmpeg_path = Some(state.ffmpeg_path.trim().to_string());
        }
    });
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
fn output_inspector(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    id: &str,
    monitors: &[MonitorInfo],
    state: &mut PanelState,
) {
    ui.heading(format!("Output {id}"));
    if rename_box(ui, editor, state, id) {
        return;
    }
    if ui.button("Identify on the projector").clicked() {
        editor.identify(Instant::now());
    }
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
    if rename_box(ui, editor, state, id) {
        return;
    }
    let Some(surface) = editor.project.surface(id).cloned() else {
        return;
    };
    ui.horizontal(|ui| {
        let mut locked = surface.locked;
        if ui
            .checkbox(&mut locked, "Locked")
            .on_hover_text("No handles, no accidental drags (L)")
            .changed()
        {
            editor.set_locked(id, locked);
        }
        if ui.button("Duplicate").on_hover_text("Ctrl+D").clicked() {
            editor.duplicate_surface(id);
        }
        if ui
            .button("Forward")
            .on_hover_text("Draw this surface later (on top)")
            .clicked()
        {
            editor.move_surface(id, 1);
        }
        if ui
            .button("Backward")
            .on_hover_text("Draw this surface earlier (underneath)")
            .clicked()
        {
            editor.move_surface(id, -1);
        }
    });
    ui.menu_button("Align", |ui| {
        for (label, op) in [
            ("Flip horizontal", AlignOp::FlipHorizontal),
            ("Flip vertical", AlignOp::FlipVertical),
            ("Rotate 90°", AlignOp::RotateQuarter),
            ("Fit output", AlignOp::FitOutput),
            ("Centre", AlignOp::Centre),
            ("Match source aspect", AlignOp::MatchSourceAspect),
        ] {
            if ui.button(label).clicked() {
                editor.align(id, op);
                ui.close();
            }
        }
    });
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
    ui.label("Crop: drag the box corners on the source preview");
    crop_preview(ui, editor, state, id, &surface);
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

/// Inspector for a source: preview, transport, generator and file settings.
#[allow(clippy::too_many_lines)]
fn source_inspector(ui: &mut egui::Ui, editor: &mut Editor, id: &str, state: &mut PanelState) {
    ui.heading(format!("Source {id}"));
    if rename_box(ui, editor, state, id) {
        return;
    }
    let Some(source) = editor.project.source(id).cloned() else {
        return;
    };
    ui.label(RichText::new(describe(&source.kind)).weak());
    if let Some(frame) = editor.sources.frame(id).cloned() {
        let (tex, size) = state.thumbs.get(ui.ctx(), id, &frame);
        let w = ui.available_width().min(280.0);
        let h = w * size.1 as f32 / size.0.max(1) as f32;
        ui.add(egui::Image::new((tex, egui::Vec2::new(w, h))));
        ui.label(
            RichText::new(format!("{} × {} px", frame.width, frame.height))
                .weak()
                .small(),
        );
    }
    if let Some(pb) = source.kind.playback() {
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .button(if pb.paused { "Play" } else { "Pause" })
                .clicked()
            {
                editor.edit_source(id, |k| {
                    if let Some(p) = k.playback_mut() {
                        p.paused = !p.paused;
                    }
                });
            }
            if ui.button("Restart").clicked() {
                editor.edit_source(id, |k| {
                    if let Some(p) = k.playback_mut() {
                        p.restart_token = p.restart_token.wrapping_add(1);
                    }
                });
                editor.sources.sync(&editor.project);
            }
            let mut looping = pb.looping;
            if ui.checkbox(&mut looping, "Loop").changed() {
                editor.edit_source(id, |k| {
                    if let Some(p) = k.playback_mut() {
                        p.looping = looping;
                    }
                });
            }
        });
        let mut speed = pb.speed;
        let r = ui.add(egui::Slider::new(&mut speed, 0.0..=4.0).text("Speed"));
        if r.drag_stopped() || r.lost_focus() {
            editor.edit_source(id, |k| {
                if let Some(p) = k.playback_mut() {
                    p.speed = speed;
                }
            });
        }
        ui.label(
            RichText::new(format!("OSC: /source/{id}/play|loop|speed|restart"))
                .weak()
                .small(),
        );
    }
    match &source.kind {
        SourceKind::Generator {
            pattern,
            speed,
            color,
        } => {
            ui.separator();
            let mut chosen = *pattern;
            egui::ComboBox::from_label("Pattern")
                .selected_text(format!("{chosen:?}"))
                .show_ui(ui, |ui| {
                    for g in GeneratorKind::ALL {
                        ui.selectable_value(&mut chosen, g, format!("{g:?}"));
                    }
                });
            if chosen != *pattern {
                editor.edit_source(id, |k| {
                    if let SourceKind::Generator { pattern, .. } = k {
                        *pattern = chosen;
                    }
                });
            }
            let mut sp = *speed;
            let r = ui.add(egui::Slider::new(&mut sp, 0.0..=4.0).text("Animation speed"));
            if r.drag_stopped() || r.lost_focus() {
                editor.edit_source(id, |k| {
                    if let SourceKind::Generator { speed, .. } = k {
                        *speed = sp;
                    }
                });
            }
            let mut c = *color;
            ui.horizontal(|ui| {
                ui.label("Tint");
                if ui.color_edit_button_rgba_unmultiplied(&mut c).changed()
                    && c.iter()
                        .zip(color.iter())
                        .any(|(a, b)| (a - b).abs() > f32::EPSILON)
                {
                    editor.edit_source(id, |k| {
                        if let SourceKind::Generator { color, .. } = k {
                            *color = c;
                        }
                    });
                }
            });
        }
        SourceKind::Sequence { fps, path, .. } => {
            ui.separator();
            ui.label(format!("Path: {path}"));
            if !path.to_ascii_lowercase().ends_with(".gif") {
                let mut f = *fps;
                let r = ui.add(egui::Slider::new(&mut f, 1.0..=120.0).text("Frames per second"));
                if r.drag_stopped() || r.lost_focus() {
                    editor.edit_source(id, |k| {
                        if let SourceKind::Sequence { fps, .. } = k {
                            *fps = f;
                        }
                    });
                }
            }
        }
        SourceKind::Video { path, .. } | SourceKind::Image { path } => {
            ui.separator();
            ui.label(format!("Path: {path}"));
            if matches!(source.kind, SourceKind::Video { .. }) && !state.ffmpeg_ok {
                ui.colored_label(
                    Color32::LIGHT_RED,
                    "ffmpeg not found: this video cannot play. Set the path in the Show panel.",
                );
            }
        }
        SourceKind::Solid { color } => {
            ui.separator();
            let mut c = *color;
            ui.horizontal(|ui| {
                ui.label("Colour");
                if ui.color_edit_button_rgba_unmultiplied(&mut c).changed()
                    && c.iter()
                        .zip(color.iter())
                        .any(|(a, b)| (a - b).abs() > f32::EPSILON)
                {
                    editor.edit_source(id, |k| {
                        if let SourceKind::Solid { color } = k {
                            *color = c;
                        }
                    });
                    editor.reload_sources();
                }
            });
        }
        SourceKind::TestPattern { .. } | SourceKind::Spout { .. } => {}
    }
    ui.separator();
    let used = editor
        .project
        .surfaces
        .iter()
        .filter(|s| s.source == *id)
        .count();
    ui.label(RichText::new(format!("used by {used} surface(s)")).weak());
    if ui
        .add_enabled(used == 0, egui::Button::new("Delete source"))
        .clicked()
    {
        editor.selection = Selection::Source(id.to_string());
        editor.delete_selection();
    }
}

/// Source preview with a draggable crop box.
fn crop_preview(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut PanelState,
    id: &str,
    surface: &lumen_core::project::Surface,
) {
    let Some(frame) = editor.sources.frame(&surface.source).cloned() else {
        ui.label(RichText::new("(no frame yet)").weak());
        return;
    };
    let (tex, size) = state.thumbs.get(ui.ctx(), &surface.source, &frame);
    let w = ui.available_width().min(280.0);
    let h = (w * size.1 as f32 / size.0.max(1) as f32).max(40.0);
    let (rect, response) =
        ui.allocate_exact_size(egui::Vec2::new(w, h), egui::Sense::click_and_drag());
    ui.painter().image(
        tex,
        rect,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
    let c = surface.crop;
    let to_screen = |x: f64, y: f64| {
        egui::Pos2::new(
            rect.min.x + x as f32 * rect.width(),
            rect.min.y + y as f32 * rect.height(),
        )
    };
    let box_rect = egui::Rect::from_min_max(to_screen(c.x, c.y), to_screen(c.x + c.w, c.y + c.h));
    ui.painter().rect_stroke(
        box_rect,
        0.0,
        egui::Stroke::new(1.5, Color32::from_rgb(255, 200, 60)),
        egui::StrokeKind::Outside,
    );
    for p in [box_rect.min, box_rect.max] {
        ui.painter()
            .circle_filled(p, 5.0, Color32::from_rgb(255, 200, 60));
    }
    if response.dragged() {
        if let Some(pos) = response.interact_pointer_pos() {
            let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            let nx = f64::from(((pos.x - rect.min.x) / rect.width()).clamp(0.0, 1.0));
            let ny = f64::from(((pos.y - rect.min.y) / rect.height()).clamp(0.0, 1.0));
            let mut crop = c;
            if origin.distance(box_rect.min) <= 12.0 {
                let (x1, y1) = (c.x + c.w, c.y + c.h);
                crop.x = nx.min(x1 - 0.01);
                crop.y = ny.min(y1 - 0.01);
                crop.w = x1 - crop.x;
                crop.h = y1 - crop.y;
            } else if origin.distance(box_rect.max) <= 12.0 {
                crop.w = (nx - c.x).max(0.01);
                crop.h = (ny - c.y).max(0.01);
            } else {
                let d = response.drag_delta();
                crop.x = (c.x + f64::from(d.x / rect.width())).clamp(0.0, 1.0 - c.w);
                crop.y = (c.y + f64::from(d.y / rect.height())).clamp(0.0, 1.0 - c.h);
            }
            editor.set_crop(id, crop);
        }
    }
    if response.drag_stopped() {
        editor.commit();
    }
    if response.double_clicked() {
        editor.set_crop(id, lumen_core::project::Crop::default());
        editor.commit();
    }
    ui.label(
        RichText::new(format!(
            "x {:.3}  y {:.3}  w {:.3}  h {:.3}  (double-click to reset)",
            c.x, c.y, c.w, c.h
        ))
        .weak()
        .small(),
    );
}

/// Modal prompts: autosave recovery, unsaved changes, welcome and the cheat sheet.
#[allow(clippy::too_many_lines)]
pub fn dialogs(
    ctx: &egui::Context,
    editor: &mut Editor,
    state: &mut PanelState,
    monitors: &[MonitorInfo],
    settings: &Settings,
    actions: &mut Actions,
) {
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
    if let Some(pending) = editor.pending.clone() {
        let what = match &pending {
            Pending::Close => "quitting",
            Pending::New => "starting a new project",
            Pending::Open(_) => "opening another project",
        };
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("Save before {what}?"));
                ui.horizontal(|ui| {
                    if ui.button("Save and continue").clicked() {
                        actions.save = true;
                        actions.resolve_pending = Some(true);
                    }
                    if ui.button("Discard changes").clicked() {
                        actions.resolve_pending = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        actions.resolve_pending = Some(false);
                    }
                });
            });
    }
    if state.welcome && !editor.show_mode {
        egui::Window::new("Welcome to lumen").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.set_min_width(380.0);
            ui.label("Map images onto walls, objects and screens with one or many projectors.");
            ui.add_space(8.0);
            let secondary = monitors.len().saturating_sub(1);
            if monitors.len() > 1 {
                ui.label(format!("{} displays detected. Make each extra display a projector output?", monitors.len()));
                if ui.button(format!("Create {secondary} output(s) with a test grid")).clicked() {
                    actions.create_outputs = true;
                }
            } else {
                ui.label(RichText::new("Only one display detected. Plug in a projector and reopen lumen, or map on this screen for now.").weak());
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Open a project…").clicked() {
                    actions.open = true;
                    state.welcome = false;
                }
                if ui.button("Start from the demo").clicked() {
                    state.welcome = false;
                }
                if ui.button("Add media…").clicked() {
                    actions.add_media = true;
                    state.welcome = false;
                }
            });
            let recent = settings.existing_recent();
            if !recent.is_empty() {
                ui.add_space(8.0);
                ui.label("Recent");
                for p in recent.iter().take(6) {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if ui.link(name).on_hover_text(p.display().to_string()).clicked() {
                        actions.open_path = Some(p.clone());
                        state.welcome = false;
                    }
                }
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Tips: drag corners on the canvas or on the projector itself. Scroll to zoom, G for a grid, I to identify outputs, F10 for show mode, ? for every shortcut.").weak().small());
            ui.horizontal(|ui| {
                if ui.button("Close").clicked() {
                    state.welcome = false;
                }
                if ui.button("Don't show again").clicked() {
                    actions.skip_welcome = true;
                }
            });
        });
    }
    if state.help {
        egui::Window::new("Keyboard shortcuts")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("shortcuts")
                    .num_columns(2)
                    .spacing([24.0, 4.0])
                    .show(ui, |ui| {
                        for (k, what) in SHORTCUTS {
                            ui.label(RichText::new(*k).monospace());
                            ui.label(*what);
                            ui.end_row();
                        }
                    });
                if ui.button("Close").clicked() {
                    state.help = false;
                }
            });
    }
    if let Some(e) = state.error.clone() {
        egui::Window::new("Problem")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.colored_label(Color32::LIGHT_RED, &e);
                if ui.button("Dismiss").clicked() {
                    state.error = None;
                }
            });
    }
}

/// The cheat sheet.
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("Drag handle", "Move a corner, control point or mask point"),
    ("Shift+drag", "Move the whole surface or mask"),
    (
        "Arrows",
        "Nudge the active handle by 1 px (Shift ×10, Alt ×0.1)",
    ),
    ("Click surface", "Select it (Shift adds to the selection)"),
    ("Scroll / middle-drag", "Zoom and pan the canvas, 0 fits"),
    ("G / S", "Toggle grid / snapping"),
    ("L", "Lock or unlock the selected surface"),
    ("Ctrl+D", "Duplicate the selected surface"),
    ("Delete", "Delete the selection"),
    ("Ctrl+Z / Ctrl+Y", "Undo / redo"),
    ("Ctrl+N / Ctrl+O / Ctrl+S", "New / open / save"),
    ("I", "Identify: flash each output's name on its projector"),
    ("B", "Blackout"),
    ("F10", "Show mode; Escape twice leaves it"),
    ("?", "This list"),
];

/// Overlay while files hover over the window.
pub fn drop_overlay(ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::unique("drop-overlay"),
    ));
    let rect = ctx.content_rect();
    painter.rect_filled(rect, 0.0, Color32::from_black_alpha(140));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Drop to add as sources",
        egui::FontId::proportional(28.0),
        Color32::WHITE,
    );
}

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
