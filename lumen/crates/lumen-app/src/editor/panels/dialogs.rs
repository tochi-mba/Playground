//! Modal prompts.

use super::super::{Editor, MonitorInfo, Pending};
use super::{Actions, PanelState, SHORTCUTS};
use crate::settings::Settings;
use eframe::egui::{self, Color32, RichText};

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
