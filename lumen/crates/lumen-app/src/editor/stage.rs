//! The stage view: every placed output drawn to scale on the virtual desktop, with monitor
//! outlines and overlaps highlighted. Click selects an output, drag moves its placement.

use super::canvas::CanvasState;
use super::outputs::paint_scene;
use super::{scene_for, Editor, MonitorInfo, Selection};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use std::sync::Arc;

/// Desktop rectangle of a placed output, in virtual-desktop pixels.
fn desktop_rect(x: i32, y: i32, w: u32, h: u32) -> Rect {
    Rect::from_min_size(
        Pos2::new(x as f32, y as f32),
        Vec2::new(w.max(1) as f32, h.max(1) as f32),
    )
}

/// Maps the union of `rects` into `avail` keeping aspect, with a margin. Returns
/// `(desktop origin, scale)` such that `screen = origin + desktop * scale`.
pub fn fit(avail: Rect, rects: &[Rect]) -> (Pos2, f32) {
    let mut bounds = Rect::NOTHING;
    for r in rects {
        bounds = bounds.union(*r);
    }
    if !bounds.is_positive() || !bounds.is_finite() {
        return (avail.min, 1.0);
    }
    let inner = avail.shrink(24.0);
    let scale = (inner.width() / bounds.width())
        .min(inner.height() / bounds.height())
        .max(1e-4);
    let size = bounds.size() * scale;
    let origin = inner.center() - size * 0.5 - bounds.min.to_vec2() * scale;
    (origin, scale)
}

fn to_screen(origin: Pos2, scale: f32, r: Rect) -> Rect {
    Rect::from_min_size(origin + r.min.to_vec2() * scale, r.size() * scale)
}

/// Draws the stage.
#[allow(clippy::too_many_lines)]
pub fn show(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut CanvasState,
    monitors: &[MonitorInfo],
    format: wgpu::TextureFormat,
) {
    let avail = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(avail, Sense::click_and_drag());
    let painter = ui.painter_at(avail);
    painter.rect_filled(avail, 0.0, crate::theme::CANVAS_BG);

    let placed: Vec<(String, Rect)> = editor
        .project
        .outputs
        .iter()
        .filter_map(|o| {
            let p = o.placement.as_ref()?;
            Some((o.id.clone(), desktop_rect(p.x, p.y, o.width, o.height)))
        })
        .collect();
    let unplaced: Vec<String> = editor
        .project
        .outputs
        .iter()
        .filter(|o| o.placement.is_none())
        .map(|o| o.id.clone())
        .collect();
    let monitor_rects: Vec<Rect> = monitors
        .iter()
        .map(|m| desktop_rect(m.x, m.y, m.width, m.height))
        .collect();
    let mut all: Vec<Rect> = placed.iter().map(|(_, r)| *r).collect();
    all.extend(monitor_rects.iter().copied());
    if all.is_empty() {
        state.stage_rects.clear();
        ui.centered_and_justified(|ui| {
            ui.label("Place an output on a monitor (output inspector) to see the stage.");
        });
        return;
    }
    let (origin, scale) = fit(avail, &all);

    // Monitors first, as outlines.
    for (m, r) in monitors.iter().zip(&monitor_rects) {
        let sr = to_screen(origin, scale, *r);
        painter.rect_stroke(
            sr,
            0.0,
            Stroke::new(1.0, Color32::from_gray(70)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            sr.min + Vec2::new(4.0, 2.0),
            egui::Align2::LEFT_TOP,
            format!("{} {}x{}", m.name, m.width, m.height),
            egui::FontId::proportional(11.0),
            Color32::from_gray(110),
        );
    }

    // Outputs, with the live composite.
    let selected = match &editor.selection {
        Selection::Output(id) => Some(id.clone()),
        _ => None,
    };
    let drag_offset = |id: &str| -> Vec2 {
        match &state.stage_drag {
            Some((d, off)) if d == id => *off,
            _ => Vec2::ZERO,
        }
    };
    state.stage_rects.clear();
    for (id, r) in &placed {
        let sr = to_screen(origin, scale, r.translate(drag_offset(id)));
        state.stage_rects.push((id.clone(), sr));
        painter.rect_filled(sr, 0.0, Color32::BLACK);
        if let Some(scene) = scene_for(editor, id) {
            ui.set_clip_rect(avail);
            paint_scene(ui, sr, format!("stage:{id}"), Arc::new(scene), format);
        }
    }
    // Overlaps on top, so the blend region is obvious.
    for (i, (_, a)) in placed.iter().enumerate() {
        for (_, b) in &placed[i + 1..] {
            let overlap = a.intersect(*b);
            if overlap.is_positive() {
                let so = to_screen(origin, scale, overlap);
                painter.rect_filled(so, 0.0, Color32::from_rgba_unmultiplied(255, 200, 0, 60));
                painter.text(
                    so.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{}x{}", overlap.width() as i32, overlap.height() as i32),
                    egui::FontId::monospace(11.0),
                    Color32::from_rgb(255, 220, 120),
                );
            }
        }
    }
    for (id, sr) in &state.stage_rects {
        let is_sel = selected.as_deref() == Some(id.as_str());
        painter.rect_stroke(
            *sr,
            0.0,
            Stroke::new(
                if is_sel { 2.0 } else { 1.0 },
                if is_sel {
                    Color32::from_rgb(80, 180, 255)
                } else {
                    Color32::from_gray(160)
                },
            ),
            egui::StrokeKind::Outside,
        );
        let o = editor.project.output(id);
        let label = match o {
            Some(o) => format!("{id} {}x{}", o.width, o.height),
            None => id.clone(),
        };
        painter.text(
            sr.min + Vec2::new(4.0, 4.0),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );
    }

    // Interaction: click selects, drag moves the placement (committed on release).
    let hit = |pos: Pos2, rects: &[(String, Rect)]| -> Option<String> {
        rects
            .iter()
            .rev()
            .find(|(_, r)| r.contains(pos))
            .map(|(id, _)| id.clone())
    };
    if response.drag_started() {
        if let Some(id) = response
            .interact_pointer_pos()
            .and_then(|p| hit(p, &state.stage_rects))
        {
            editor.selection = Selection::Output(id.clone());
            state.output = Some(id.clone());
            state.stage_drag = Some((id, Vec2::ZERO));
        }
    }
    if response.dragged() {
        if let Some((_, off)) = &mut state.stage_drag {
            *off += response.drag_delta() / scale;
        }
    }
    if response.drag_stopped() {
        if let Some((id, off)) = state.stage_drag.take() {
            editor.move_placement(&id, off.x.round() as i32, off.y.round() as i32);
        }
    }
    if response.clicked() {
        match response
            .interact_pointer_pos()
            .and_then(|p| hit(p, &state.stage_rects))
        {
            Some(id) => {
                editor.selection = Selection::Output(id.clone());
                state.output = Some(id);
            }
            None => editor.selection = Selection::None,
        }
    }

    // Footer: what is not on the stage.
    if !unplaced.is_empty() {
        painter.text(
            avail.left_bottom() + Vec2::new(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            format!(
                "Not placed: {} (use \"Fill monitor\" in the output inspector)",
                unplaced.join(", ")
            ),
            egui::FontId::proportional(12.0),
            Color32::from_gray(170),
        );
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn fit_scales_the_union_into_the_area() {
        let avail = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 500.0));
        let a = desktop_rect(0, 0, 1920, 1080);
        let b = desktop_rect(1920, 0, 1920, 1080);
        let (origin, scale) = fit(avail, &[a, b]);
        let sa = to_screen(origin, scale, a);
        let sb = to_screen(origin, scale, b);
        assert!(
            (sa.width() + sb.width() - 952.0).abs() < 0.5,
            "fills the width minus margins"
        );
        assert!(sa.max.x <= sb.min.x + 0.01);
        assert!(avail.contains_rect(sa) && avail.contains_rect(sb));
        assert_eq!(fit(avail, &[]), (avail.min, 1.0));
        assert_eq!(fit(avail, &[Rect::NOTHING]), (avail.min, 1.0));
    }
}
