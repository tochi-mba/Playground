//! The inspector: outputs, surfaces, sources, the show.

use super::super::{AlignOp, Editor, MonitorInfo, Selection};
use super::{describe, param_slider, param_toggle, rename_box, Actions, PanelState};
use eframe::egui::{self, Color32, RichText};
use lumen_core::blend::BlendCurve;
use lumen_core::midimap::MidiMode;
use lumen_core::params::ParamValue;
use lumen_core::project::{BlendMode, GeneratorKind, Overlay, Placement, SourceKind};
use lumen_core::warp::{BezierMesh, WarpKind};
use std::time::Instant;

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
    if ui
        .button("Suggest blends")
        .on_hover_text("Set blend widths from where placed outputs overlap on the desktop")
        .clicked()
    {
        editor.suggest_blends();
    }
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
    ui.separator();
    super::control::control_section(ui, editor, state, actions);
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
                    super::super::monitor_at(monitors, x as i32, y as i32).map_or(1.0, |m| m.scale);
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
    ui.label("Calibration");
    let mut overlay = output.overlay;
    egui::ComboBox::from_label("Overlay")
        .selected_text(overlay.map_or("none", Overlay::label))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut overlay, None, "none");
            for k in Overlay::ALL {
                ui.selectable_value(&mut overlay, Some(k), k.label());
            }
        });
    if overlay != output.overlay {
        editor.set_overlay(id, overlay);
    }
    ui.label(
        RichText::new(
            "Overlays never show in show mode. Gamma match: put Grey steps on both sides of a \
             seam, then press [ or ] until the steps match across it.",
        )
        .weak()
        .small(),
    );
    ui.horizontal(|ui| {
        if ui.button("Gamma match steps").clicked() {
            editor.set_overlay(id, Some(Overlay::GreySteps));
        }
        if ui.button("Clear overlay").clicked() {
            editor.set_overlay(id, None);
        }
    });
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
            .button("MIDI learn opacity")
            .on_hover_text("Move a fader to map it to this surface's opacity")
            .clicked()
        {
            editor.arm_midi_learn(&format!("/surface/{id}/opacity"), MidiMode::Fader);
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
