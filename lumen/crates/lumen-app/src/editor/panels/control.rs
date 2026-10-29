//! Show control: MIDI, OSC feedback, the phone remote and the cue list.

use super::super::Editor;
use super::{Actions, PanelState};
use eframe::egui::{self, Color32, RichText};
use lumen_core::midimap::MidiMode;

/// MIDI, OSC feedback and the phone remote.
#[allow(clippy::too_many_lines)]
pub(super) fn control_section(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut PanelState,
    actions: &mut Actions,
) {
    ui.heading("Control");
    ui.label(RichText::new("MIDI").strong());
    ui.horizontal(|ui| {
        egui::ComboBox::from_label("MIDI port")
            .selected_text(if state.midi_port.is_empty() {
                "no MIDI port"
            } else {
                state.midi_port.as_str()
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.midi_port, String::new(), "none");
                let ports = state.midi_ports.clone();
                for port in ports {
                    ui.selectable_value(&mut state.midi_port, port.clone(), port);
                }
            });
        if ui.button("Refresh MIDI ports").clicked() {
            state.midi_ports = lumen_control::midi::ports().unwrap_or_default();
        }
        if ui.button("Open MIDI port").clicked() {
            actions.midi_port = Some(state.midi_port.clone());
        }
    });
    match &editor.midi {
        Some(m) => {
            ui.label(format!("listening on {}", m.name()));
        }
        None => {
            ui.label(RichText::new("MIDI off").weak());
        }
    }
    if let Some(e) = editor.last_midi {
        ui.label(RichText::new(format!("last: {:?} = {}", e.key, e.value)).monospace());
    }
    let label = ui.label("Learn address");
    ui.text_edit_singleline(&mut state.midi_address)
        .labelled_by(label.id);
    ui.horizontal(|ui| {
        for (what, mode) in [
            ("Learn fader", MidiMode::Fader),
            ("Learn button", MidiMode::Button),
            ("Learn switch", MidiMode::Switch),
        ] {
            if ui.button(what).clicked() {
                editor.arm_midi_learn(state.midi_address.trim(), mode);
            }
        }
    });
    if let Some((address, _)) = editor.midi_learn.clone() {
        ui.horizontal(|ui| {
            ui.colored_label(Color32::YELLOW, format!("move a control for {address}"));
            if ui.button("Cancel learn").clicked() {
                editor.midi_learn = None;
            }
        });
    }
    let mappings = editor.project.midi.mappings.clone();
    for m in mappings {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{:?} -> {} ({:?})", m.key, m.address, m.mode)).small());
            if ui.button(format!("Forget {}", m.address)).clicked() {
                let address = m.address.clone();
                editor.edit(move |p| p.midi.forget(&address));
            }
        });
    }
    ui.separator();
    ui.label(RichText::new("OSC feedback").strong());
    let label = ui.label("Feedback host:port");
    ui.text_edit_singleline(&mut state.feedback_target)
        .labelled_by(label.id)
        .on_hover_text("Changed parameters are sent back here, e.g. 192.168.1.20:9001");
    ui.horizontal(|ui| {
        if ui.button("Apply feedback").clicked() {
            actions.feedback = Some(state.feedback_target.trim().to_string());
        }
        match &editor.feedback {
            Some(f) => ui.label(format!("sending to {}", f.target())),
            None => ui.label(RichText::new("feedback off").weak()),
        };
    });
    ui.separator();
    ui.label(RichText::new("Phone remote").strong());
    ui.horizontal(|ui| {
        let label = ui.label("Remote port");
        ui.add(egui::DragValue::new(&mut state.remote_port).range(0..=65535))
            .labelled_by(label.id);
        if ui.button("Start remote").clicked() {
            actions.remote_port = Some(state.remote_port);
        }
        if ui.button("Stop remote").clicked() {
            actions.remote_port = Some(0);
        }
    });
    if let Some(url) = editor.remote_url() {
        ui.label(RichText::new(&url).monospace());
        ui.label("Scan with a phone on the same Wi-Fi:");
        if let Some((n, modules)) = lumen_control::web::qr_modules(&url) {
            let cell = 3.0;
            let size = cell * n as f32;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, Color32::WHITE);
            for y in 0..n {
                for x in 0..n {
                    if modules[y * n + x] {
                        let min = rect.min + egui::vec2(x as f32 * cell, y as f32 * cell);
                        painter.rect_filled(
                            egui::Rect::from_min_size(min, egui::vec2(cell, cell)),
                            0.0,
                            Color32::BLACK,
                        );
                    }
                }
            }
        }
    } else {
        ui.label(RichText::new("remote off").weak());
    }
}

/// The cue list along the bottom.
#[allow(clippy::too_many_lines)]
pub fn cue_panel(ui: &mut egui::Ui, editor: &mut Editor, state: &mut PanelState, now: f64) {
    egui::Panel::bottom("lumen-cues")
        .resizable(true)
        .default_size(140.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Cues");
                let label = ui.label("Cue name");
                ui.add(
                    egui::TextEdit::singleline(&mut state.cue_name)
                        .desired_width(120.0)
                        .hint_text("name"),
                )
                .labelled_by(label.id);
                let label = ui.label("Fade s");
                ui.add(
                    egui::DragValue::new(&mut state.cue_fade)
                        .range(0.0..=600.0)
                        .speed(0.1),
                )
                .labelled_by(label.id);
                if ui
                    .button("Store cue")
                    .on_hover_text("Snapshot every live parameter as a new cue")
                    .clicked()
                {
                    editor.store_cue(&state.cue_name.clone(), state.cue_fade);
                    state.cue_name.clear();
                }
                if ui
                    .button("GO")
                    .on_hover_text("Fade to the next cue (Space)")
                    .clicked()
                {
                    editor.cue_go_next(now);
                }
                if ui.button("Stop fade").clicked() {
                    editor.cues.stop();
                }
                if let Some(f) = &editor.cues.fade {
                    ui.add(
                        egui::ProgressBar::new(f.progress(now) as f32)
                            .desired_width(80.0)
                            .text("fading"),
                    );
                }
            });
            let cues = editor.project.cues.clone();
            if cues.is_empty() {
                ui.label(
                    RichText::new(
                        "No cues yet. Set up a look, then Store cue. Space fades to the next one.",
                    )
                    .weak(),
                );
                return;
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (i, cue) in cues.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let current = editor.cues.current == Some(i);
                        let selected = editor.selected_cue == Some(i);
                        let title = if cue.name.is_empty() {
                            cue.id.clone()
                        } else {
                            format!("{} ({})", cue.name, cue.id)
                        };
                        let text = format!(
                            "{}{} {title} {:.1}s",
                            if current { "(current) " } else { "" },
                            i + 1,
                            cue.fade_seconds
                        );
                        if ui.selectable_label(selected, text).clicked() {
                            editor.selected_cue = Some(i);
                        }
                        if ui.button(format!("GO {}", cue.id)).clicked() {
                            editor.cue_go(i, now);
                        }
                        if selected {
                            if ui
                                .button("Update cue")
                                .on_hover_text("Overwrite this cue with the live look")
                                .clicked()
                            {
                                editor.update_cue(i);
                            }
                            if ui.button("Cue up").clicked() {
                                editor.move_cue(i, -1);
                            }
                            if ui.button("Cue down").clicked() {
                                editor.move_cue(i, 1);
                            }
                            let mut fade = cue.fade_seconds;
                            let label = ui.label("Cue fade");
                            if ui
                                .add(
                                    egui::DragValue::new(&mut fade)
                                        .range(0.0..=600.0)
                                        .speed(0.1),
                                )
                                .labelled_by(label.id)
                                .changed()
                            {
                                editor.edit(move |p| {
                                    if let Some(c) = p.cues.get_mut(i) {
                                        c.fade_seconds = fade;
                                    }
                                });
                            }
                            if ui.button("Delete cue").clicked() {
                                editor.delete_cue(i);
                            }
                        }
                    });
                }
            });
        });
}
