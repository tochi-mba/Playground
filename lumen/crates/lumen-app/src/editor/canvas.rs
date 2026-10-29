//! The preview canvas: the selected output rendered live, with draggable geometry handles.

use super::outputs::paint_scene;
use super::{scene_for, Editor, Selection};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2 as EVec2};
use lumen_core::geom::Vec2;
use lumen_core::mask::Mask;
use lumen_core::warp::WarpKind;
use std::sync::Arc;

/// Pixel radius within which a handle grabs the pointer.
pub const GRAB_RADIUS: f32 = 10.0;

/// Which handle is being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    /// Quad corner `n` or Bezier control point `n` (row-major).
    Warp(usize),
    /// Point `n` of the selected mask.
    Mask(usize),
    /// Everything at once (shift-drag).
    All,
}

/// Canvas interaction state.
#[derive(Debug, Default)]
pub struct CanvasState {
    /// Handle under drag.
    pub active: Option<Handle>,
    /// Which output the canvas shows; defaults to the selection's output or the first.
    pub output: Option<String>,
    /// Screen rect of the preview during the last frame (tests use it to aim the pointer).
    pub last_rect: Option<Rect>,
}

/// Maps a normalised point to screen space inside `rect`.
pub fn to_screen(rect: Rect, p: Vec2) -> Pos2 {
    Pos2::new(
        rect.min.x + p.x as f32 * rect.width(),
        rect.min.y + p.y as f32 * rect.height(),
    )
}

/// Maps a screen position to normalised output space.
pub fn to_norm(rect: Rect, p: Pos2) -> Vec2 {
    Vec2::new(
        f64::from((p.x - rect.min.x) / rect.width().max(1.0)),
        f64::from((p.y - rect.min.y) / rect.height().max(1.0)),
    )
}

/// Largest rect with `aspect` (w/h) centred in `avail`.
pub fn fit_rect(avail: Rect, aspect: f32) -> Rect {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        16.0 / 9.0
    };
    let (w, h) = if avail.width() / avail.height() > aspect {
        (avail.height() * aspect, avail.height())
    } else {
        (avail.width(), avail.width() / aspect)
    };
    Rect::from_center_size(avail.center(), EVec2::new(w, h))
}

/// Handle positions (normalised) for the selected geometry.
pub fn handles(warp: &WarpKind, mask: Option<&Mask>) -> Vec<(Handle, Vec2)> {
    if let Some(m) = mask {
        return m
            .points
            .iter()
            .enumerate()
            .map(|(i, p)| (Handle::Mask(i), *p))
            .collect();
    }
    match warp {
        WarpKind::Quad(q) => q
            .corners
            .iter()
            .enumerate()
            .map(|(i, p)| (Handle::Warp(i), *p))
            .collect(),
        WarpKind::Bezier(b) => b
            .control_points
            .iter()
            .enumerate()
            .map(|(i, p)| (Handle::Warp(i), *p))
            .collect(),
    }
}

/// Nearest handle to `pos` within `radius` screen pixels.
pub fn pick(rect: Rect, list: &[(Handle, Vec2)], pos: Pos2, radius: f32) -> Option<Handle> {
    list.iter()
        .map(|(h, p)| (*h, to_screen(rect, *p).distance(pos)))
        .filter(|(_, d)| *d <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(h, _)| h)
}

/// Returns the geometry with `handle` moved to `p` (or everything shifted by `delta`).
pub fn moved(
    warp: &WarpKind,
    mask: Option<&Mask>,
    handle: Handle,
    p: Vec2,
    delta: Vec2,
) -> (Option<WarpKind>, Option<Mask>) {
    match (handle, mask) {
        (Handle::Mask(i), Some(m)) => {
            let mut m = m.clone();
            if let Some(pt) = m.points.get_mut(i) {
                *pt = p;
            }
            (None, Some(m))
        }
        (Handle::All, Some(m)) => {
            let mut m = m.clone();
            for pt in &mut m.points {
                *pt = *pt + delta;
            }
            (None, Some(m))
        }
        (Handle::Warp(i), _) => {
            let mut w = warp.clone();
            match &mut w {
                WarpKind::Quad(q) => q.set_corner(i, p),
                WarpKind::Bezier(b) => {
                    if let Some(pt) = b.control_points.get_mut(i) {
                        *pt = p;
                    }
                }
            }
            (Some(w), None)
        }
        (Handle::All, None) => {
            let mut w = warp.clone();
            match &mut w {
                WarpKind::Quad(q) => q.corners.iter_mut().for_each(|c| *c = *c + delta),
                WarpKind::Bezier(b) => b.control_points.iter_mut().for_each(|c| *c = *c + delta),
            }
            (Some(w), None)
        }
        (Handle::Mask(_), None) => (None, None),
    }
}

/// Draws and runs the canvas for one frame.
#[allow(clippy::too_many_lines)]
pub fn show(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut CanvasState,
    format: wgpu::TextureFormat,
) {
    // Decide which output to show.
    let selected_output = match &editor.selection {
        Selection::Output(id) => Some(id.clone()),
        Selection::Surface { id, .. } => editor.project.surface(id).map(|s| s.output.clone()),
        Selection::None => None,
    };
    if let Some(o) = selected_output {
        state.output = Some(o);
    }
    if state
        .output
        .as_ref()
        .is_none_or(|o| editor.project.output(o).is_none())
    {
        state.output = editor.project.outputs.first().map(|o| o.id.clone());
    }
    let Some(output_id) = state.output.clone() else {
        ui.centered_and_justified(|ui| ui.label("Add an output to start mapping."));
        return;
    };
    let Some(output) = editor.project.output(&output_id).cloned() else {
        return;
    };

    let avail = ui.available_rect_before_wrap().shrink(12.0);
    let rect = fit_rect(avail, output.width as f32 / output.height.max(1) as f32);
    state.last_rect = Some(rect);
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
    if let Some(scene) = scene_for(editor, &output_id) {
        paint_scene(
            ui,
            rect,
            format!("preview:{output_id}"),
            Arc::new(scene),
            format,
        );
    }
    ui.painter().rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, Color32::from_gray(90)),
        egui::StrokeKind::Outside,
    );

    // Geometry overlay for the selected surface.
    let Selection::Surface { id, mask } = editor.selection.clone() else {
        draw_output_label(ui, rect, &output_id);
        return;
    };
    let Some(surface) = editor.project.surface(&id).cloned() else {
        return;
    };
    if surface.output != output_id {
        return;
    }
    let mask_ref = mask.and_then(|i| surface.masks.get(i));
    draw_wireframe(ui, rect, &surface.warp, &surface.masks, mask);
    let list = handles(&surface.warp, mask_ref);
    for (h, p) in &list {
        let pos = to_screen(rect, *p);
        let active = state.active == Some(*h);
        ui.painter().circle_filled(
            pos,
            if active { 6.0 } else { 4.5 },
            if active {
                Color32::YELLOW
            } else {
                Color32::WHITE
            },
        );
        ui.painter()
            .circle_stroke(pos, 7.0, Stroke::new(1.0, Color32::from_black_alpha(160)));
    }

    // Interaction.
    let pointer = response.interact_pointer_pos();
    let shift = ui.input(|i| i.modifiers.shift);
    if response.drag_started() {
        // Pick where the button went down, not where the pointer is now that the drag
        // threshold has been crossed.
        let origin = ui.input(|i| i.pointer.press_origin()).or(pointer);
        state.active = origin
            .and_then(|pos| pick(rect, &list, pos, GRAB_RADIUS))
            .or(if shift { Some(Handle::All) } else { None });
    }
    if response.dragged() {
        if let (Some(h), Some(pos)) = (state.active, pointer) {
            let delta_px = response.drag_delta();
            let delta = Vec2::new(
                f64::from(delta_px.x / rect.width()),
                f64::from(delta_px.y / rect.height()),
            );
            let p = to_norm(rect, pos);
            apply_move(editor, &id, &surface.warp, mask_ref, h, p, delta);
            let mut masks = surface.masks.clone();
            if let (Some(i), Some(m)) = (mask, moved(&surface.warp, mask_ref, h, p, delta).1) {
                masks[i] = m;
            }
            let _ = masks;
        }
    }
    if response.drag_stopped() {
        state.active = None;
        editor.commit();
    }
    if response.clicked() && !response.dragged() {
        state.active = pointer.and_then(|pos| pick(rect, &list, pos, GRAB_RADIUS));
    }

    // Arrow-key nudging of the active handle.
    if let Some(h) = state.active {
        let step = if shift { 10.0 } else { 1.0 };
        let (dx, dy) = ui.input(|i| {
            let mut d = (0.0f32, 0.0f32);
            if i.key_pressed(egui::Key::ArrowLeft) {
                d.0 -= step;
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                d.0 += step;
            }
            if i.key_pressed(egui::Key::ArrowUp) {
                d.1 -= step;
            }
            if i.key_pressed(egui::Key::ArrowDown) {
                d.1 += step;
            }
            d
        });
        if dx != 0.0 || dy != 0.0 {
            let delta = Vec2::new(f64::from(dx / rect.width()), f64::from(dy / rect.height()));
            let current = list
                .iter()
                .find(|(hh, _)| *hh == h)
                .map(|(_, p)| *p)
                .unwrap_or_default();
            apply_move(
                editor,
                &id,
                &surface.warp,
                mask_ref,
                h,
                current + delta,
                delta,
            );
            editor.commit();
        }
    }
    draw_output_label(ui, rect, &output_id);
}

fn apply_move(
    editor: &mut Editor,
    surface: &str,
    warp: &WarpKind,
    mask: Option<&Mask>,
    h: Handle,
    p: Vec2,
    delta: Vec2,
) {
    let (new_warp, new_mask) = moved(warp, mask, h, p, delta);
    if let Some(w) = new_warp {
        editor.set_warp(surface, w);
    }
    if let (Some(m), Selection::Surface { mask: Some(i), .. }) =
        (new_mask, editor.selection.clone())
    {
        if let Some(mut masks) = editor.project.surface(surface).map(|s| s.masks.clone()) {
            if i < masks.len() {
                masks[i] = m;
                editor.set_masks(surface, masks);
            }
        }
    }
}

fn draw_output_label(ui: &egui::Ui, rect: Rect, output: &str) {
    ui.painter().text(
        rect.min + EVec2::new(6.0, 4.0),
        egui::Align2::LEFT_TOP,
        output,
        egui::FontId::proportional(12.0),
        Color32::from_white_alpha(140),
    );
}

fn draw_wireframe(
    ui: &egui::Ui,
    rect: Rect,
    warp: &WarpKind,
    masks: &[Mask],
    selected_mask: Option<usize>,
) {
    let painter = ui.painter();
    let stroke = Stroke::new(1.0, Color32::from_rgb(90, 200, 255));
    match warp {
        WarpKind::Quad(q) => {
            for i in 0..4 {
                painter.line_segment(
                    [
                        to_screen(rect, q.corners[i]),
                        to_screen(rect, q.corners[(i + 1) % 4]),
                    ],
                    stroke,
                );
            }
        }
        WarpKind::Bezier(b) => {
            let faint = Stroke::new(1.0, Color32::from_rgba_unmultiplied(90, 200, 255, 90));
            let (w, h) = (b.width(), b.height());
            for j in 0..h {
                for i in 0..w {
                    let p = b.point(i, j).unwrap_or_default();
                    if i + 1 < w {
                        painter.line_segment(
                            [
                                to_screen(rect, p),
                                to_screen(rect, b.point(i + 1, j).unwrap_or_default()),
                            ],
                            faint,
                        );
                    }
                    if j + 1 < h {
                        painter.line_segment(
                            [
                                to_screen(rect, p),
                                to_screen(rect, b.point(i, j + 1).unwrap_or_default()),
                            ],
                            faint,
                        );
                    }
                }
            }
            // Patch outline, evaluated along the surface.
            let n = 24;
            let edge = |f: &dyn Fn(f64) -> Vec2| {
                let pts: Vec<Pos2> = (0..=n)
                    .map(|k| to_screen(rect, f(f64::from(k) / f64::from(n))))
                    .collect();
                painter.add(egui::Shape::line(pts, stroke));
            };
            edge(&|t| b.evaluate(t, 0.0));
            edge(&|t| b.evaluate(t, 1.0));
            edge(&|t| b.evaluate(0.0, t));
            edge(&|t| b.evaluate(1.0, t));
        }
    }
    for (i, m) in masks.iter().enumerate() {
        let color = if selected_mask == Some(i) {
            Color32::from_rgb(255, 160, 60)
        } else {
            Color32::from_rgba_unmultiplied(255, 160, 60, 110)
        };
        let n = m.points.len();
        for k in 0..n {
            painter.line_segment(
                [
                    to_screen(rect, m.points[k]),
                    to_screen(rect, m.points[(k + 1) % n]),
                ],
                Stroke::new(1.0, color),
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used)]
mod tests {
    use super::*;
    use lumen_core::warp::{BezierMesh, Quad};

    #[test]
    fn coordinate_mapping_round_trips() {
        let rect = Rect::from_min_size(Pos2::new(10.0, 20.0), EVec2::new(200.0, 100.0));
        let p = Vec2::new(0.25, 0.5);
        let s = to_screen(rect, p);
        assert_eq!(s, Pos2::new(60.0, 70.0));
        let back = to_norm(rect, s);
        assert!((back.x - 0.25).abs() < 1e-6 && (back.y - 0.5).abs() < 1e-6);
        let zero = Rect::from_min_size(Pos2::ZERO, EVec2::ZERO);
        assert!(to_norm(zero, Pos2::new(1.0, 1.0)).is_finite());
    }

    #[test]
    fn fit_rect_keeps_aspect() {
        let avail = Rect::from_min_size(Pos2::ZERO, EVec2::new(400.0, 400.0));
        let r = fit_rect(avail, 2.0);
        assert_eq!(r.size(), EVec2::new(400.0, 200.0));
        assert_eq!(r.center(), avail.center());
        let r = fit_rect(avail, 0.5);
        assert_eq!(r.size(), EVec2::new(200.0, 400.0));
        assert_eq!(
            fit_rect(avail, f32::NAN).size().x / fit_rect(avail, f32::NAN).size().y,
            16.0 / 9.0
        );
    }

    #[test]
    fn handles_and_picking() {
        let quad = WarpKind::Quad(Quad::full_frame());
        let list = handles(&quad, None);
        assert_eq!(list.len(), 4);
        let bez = WarpKind::Bezier(BezierMesh::flat(1, 1).unwrap());
        assert_eq!(handles(&bez, None).len(), 16);
        let mask = Mask::hole(vec![
            Vec2::new(0.1, 0.1),
            Vec2::new(0.9, 0.1),
            Vec2::new(0.5, 0.9),
        ]);
        assert_eq!(handles(&quad, Some(&mask)).len(), 3);
        let rect = Rect::from_min_size(Pos2::ZERO, EVec2::new(100.0, 100.0));
        assert_eq!(
            pick(rect, &list, Pos2::new(97.0, 2.0), GRAB_RADIUS),
            Some(Handle::Warp(1))
        );
        assert_eq!(pick(rect, &list, Pos2::new(50.0, 50.0), GRAB_RADIUS), None);
        assert_eq!(
            pick(rect, &list, Pos2::new(50.0, 50.0), 1000.0),
            Some(Handle::Warp(0))
        );
    }

    #[test]
    fn moves() {
        let quad = WarpKind::Quad(Quad::full_frame());
        let (w, m) = moved(
            &quad,
            None,
            Handle::Warp(2),
            Vec2::new(0.8, 0.9),
            Vec2::ZERO,
        );
        assert!(m.is_none());
        let WarpKind::Quad(q) = w.unwrap() else {
            panic!()
        };
        assert_eq!(q.corners[2], Vec2::new(0.8, 0.9));
        let (w, _) = moved(&quad, None, Handle::All, Vec2::ZERO, Vec2::new(0.1, 0.0));
        let WarpKind::Quad(q) = w.unwrap() else {
            panic!()
        };
        assert_eq!(q.corners[0], Vec2::new(0.1, 0.0));
        let bez = WarpKind::Bezier(BezierMesh::flat(1, 1).unwrap());
        let (w, _) = moved(&bez, None, Handle::Warp(5), Vec2::new(0.5, 0.5), Vec2::ZERO);
        let WarpKind::Bezier(b) = w.unwrap() else {
            panic!()
        };
        assert_eq!(b.control_points[5], Vec2::new(0.5, 0.5));
        let (w, _) = moved(&bez, None, Handle::All, Vec2::ZERO, Vec2::new(0.0, 0.1));
        let WarpKind::Bezier(b) = w.unwrap() else {
            panic!()
        };
        assert_eq!(b.control_points[0], Vec2::new(0.0, 0.1));
        let (w, _) = moved(&bez, None, Handle::Warp(99), Vec2::ZERO, Vec2::ZERO);
        assert_eq!(w, Some(bez.clone()), "out of range index is ignored");
        let mask = Mask::hole(vec![
            Vec2::new(0.1, 0.1),
            Vec2::new(0.9, 0.1),
            Vec2::new(0.5, 0.9),
        ]);
        let (w, m) = moved(
            &quad,
            Some(&mask),
            Handle::Mask(1),
            Vec2::new(0.4, 0.4),
            Vec2::ZERO,
        );
        assert!(w.is_none());
        assert_eq!(m.unwrap().points[1], Vec2::new(0.4, 0.4));
        let (_, m) = moved(
            &quad,
            Some(&mask),
            Handle::Mask(9),
            Vec2::new(0.4, 0.4),
            Vec2::ZERO,
        );
        assert_eq!(m.unwrap(), mask);
        let (_, m) = moved(
            &quad,
            Some(&mask),
            Handle::All,
            Vec2::ZERO,
            Vec2::new(0.1, 0.1),
        );
        assert_eq!(m.unwrap().points[0], Vec2::new(0.2, 0.2));
        assert_eq!(
            moved(&quad, None, Handle::Mask(0), Vec2::ZERO, Vec2::ZERO),
            (None, None)
        );
    }
}
