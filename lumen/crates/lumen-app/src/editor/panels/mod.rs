//! Toolbar, status bar, object tree, inspector, dialogs and the show-mode screen.

use super::canvas::CanvasState;
use super::thumbs::Thumbnails;
use super::{Editor, Selection};
use eframe::egui::{self, Color32, RichText};
use lumen_core::params::ParamValue;
use lumen_core::project::{GeneratorKind, SourceKind, TestPattern};
use lumen_core::warp::WarpKind;
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
    /// MIDI input port to open (empty closes it).
    pub midi_port: Option<String>,
    /// OSC feedback target `host:port` (empty disables).
    pub feedback: Option<String>,
    /// Phone remote port (0 stops it).
    pub remote_port: Option<u16>,
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
    /// Name for the next stored cue.
    pub cue_name: String,
    /// Fade time for the next stored cue.
    pub cue_fade: f64,
    /// MIDI port field.
    pub midi_port: String,
    /// MIDI ports found last time the list was refreshed.
    pub midi_ports: Vec<String>,
    /// Address typed into the MIDI learn box.
    pub midi_address: String,
    /// OSC feedback field.
    pub feedback_target: String,
    /// Phone remote port field.
    pub remote_port: u16,
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
            cue_name: String::new(),
            cue_fade: 2.0,
            midi_port: String::new(),
            midi_ports: Vec::new(),
            midi_address: "/show/master".into(),
            feedback_target: String::new(),
            remote_port: 8080,
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
    ("Space", "GO: fade to the next cue"),
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

mod control;
mod dialogs;
mod inspectors;

pub use control::cue_panel;
pub use dialogs::dialogs;
pub use inspectors::inspector;
