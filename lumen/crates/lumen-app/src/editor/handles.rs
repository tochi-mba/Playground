//! Geometry handles: drawing and dragging corners, control points and mask points.
//!
//! The canvas and the output windows share this code, so calibrating on the wall behaves
//! exactly like calibrating in the editor. Everything that can be unit-tested without a
//! pointer is a pure function here; `interact` is the thin egui layer over them.

use super::{Editor, Selection};
use eframe::egui::{self, Color32, Pos2, Rect, Response, Stroke, Vec2 as EVec2};
use lumen_core::geom::Vec2;
use lumen_core::mask::Mask;
use lumen_core::warp::WarpKind;

/// Pixel radius within which a handle grabs the pointer.
pub const GRAB_RADIUS: f32 = 10.0;
/// Pixel distance within which snapping engages.
pub const SNAP_RADIUS: f32 = 8.0;

/// Which handle is being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    /// Quad corner `n` or Bezier control point `n` (row-major).
    Warp(usize),
    /// Point `n` of the selected mask.
    Mask(usize),
    /// Everything at once (Shift-drag).
    All,
}

/// Interaction state of one view (the canvas or an output window).
#[derive(Debug, Clone, Default)]
pub struct HandleState {
    /// Handle under drag or last clicked.
    pub active: Option<Handle>,
    /// Snap to edges, centre and grid.
    pub snap: bool,
    /// Grid divisions per axis used for snapping and drawing (`0` disables the grid).
    pub grid: u32,
}

/// Maps normalised output space onto a screen rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mapping {
    /// Where `(0,0)..(1,1)` lands on screen.
    pub rect: Rect,
}

impl Mapping {
    /// Normalised → screen.
    pub fn to_screen(self, p: Vec2) -> Pos2 {
        Pos2::new(
            self.rect.min.x + p.x as f32 * self.rect.width(),
            self.rect.min.y + p.y as f32 * self.rect.height(),
        )
    }

    /// Screen → normalised.
    pub fn to_norm(self, p: Pos2) -> Vec2 {
        Vec2::new(
            f64::from((p.x - self.rect.min.x) / self.rect.width().max(1.0)),
            f64::from((p.y - self.rect.min.y) / self.rect.height().max(1.0)),
        )
    }

    /// A screen delta as a normalised delta.
    pub fn delta(&self, d: EVec2) -> Vec2 {
        Vec2::new(
            f64::from(d.x / self.rect.width().max(1.0)),
            f64::from(d.y / self.rect.height().max(1.0)),
        )
    }
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
    warp.points()
        .into_iter()
        .enumerate()
        .map(|(i, p)| (Handle::Warp(i), p))
        .collect()
}

/// Nearest handle to `pos` within `radius` screen pixels.
pub fn pick(map: &Mapping, list: &[(Handle, Vec2)], pos: Pos2, radius: f32) -> Option<Handle> {
    list.iter()
        .map(|(h, p)| (*h, map.to_screen(*p).distance(pos)))
        .filter(|(_, d)| *d <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(h, _)| h)
}

/// Snaps `p` to the nearest output edge, centre line or grid line when one is within
/// `radius` screen pixels; otherwise returns `p` unchanged.
pub fn snap(map: &Mapping, p: Vec2, grid: u32, radius: f32) -> Vec2 {
    let candidates = |size: f32| -> Vec<f64> {
        let mut c = vec![0.0, 0.5, 1.0];
        if grid > 0 {
            c.extend((1..grid).map(|i| f64::from(i) / f64::from(grid)));
        }
        let _ = size;
        c
    };
    let snap_axis = |v: f64, size: f32| -> f64 {
        candidates(size)
            .into_iter()
            .map(|c| (c, ((c - v) as f32 * size).abs()))
            .filter(|(_, d)| *d <= radius)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(v, |(c, _)| c)
    };
    Vec2::new(
        snap_axis(p.x, map.rect.width()),
        snap_axis(p.y, map.rect.height()),
    )
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
        (Handle::All, None) => (Some(warp.map_points(|c| c + delta)), None),
        (Handle::Mask(_), None) => (None, None),
    }
}

/// True when `p` is inside the quadrilateral/outline of a warp (used for click-to-select).
pub fn contains(warp: &WarpKind, p: Vec2) -> bool {
    let outline: Vec<Vec2> = match warp {
        WarpKind::Quad(q) => q.corners.to_vec(),
        WarpKind::Bezier(b) => {
            let n = 8;
            let mut pts = Vec::new();
            pts.extend((0..n).map(|k| b.evaluate(f64::from(k) / f64::from(n), 0.0)));
            pts.extend((0..n).map(|k| b.evaluate(1.0, f64::from(k) / f64::from(n))));
            pts.extend((0..n).map(|k| b.evaluate(1.0 - f64::from(k) / f64::from(n), 1.0)));
            pts.extend((0..n).map(|k| b.evaluate(0.0, 1.0 - f64::from(k) / f64::from(n))));
            pts
        }
    };
    Mask::hole(outline).contains(p)
}

/// Colour used to outline and identify an output, by its index in the project.
pub fn output_color(index: usize) -> Color32 {
    const PALETTE: [Color32; 6] = [
        Color32::from_rgb(90, 200, 255),
        Color32::from_rgb(255, 170, 60),
        Color32::from_rgb(120, 230, 120),
        Color32::from_rgb(255, 110, 180),
        Color32::from_rgb(220, 220, 90),
        Color32::from_rgb(180, 140, 255),
    ];
    PALETTE[index % PALETTE.len()]
}

/// Draws the grid, the outlines of every surface on the output (selected one highlighted),
/// handles for the selected geometry, and runs drags, clicks and key nudges.
///
/// `response` is the interaction over `map.rect`'s area (a bigger clip rect is fine).
/// Returns the active handle and its position in output pixels for a readout.
#[allow(clippy::too_many_lines)]
pub fn interact(
    ui: &egui::Ui,
    editor: &mut Editor,
    state: &mut HandleState,
    response: &Response,
    map: &Mapping,
    output_id: &str,
) -> Option<(Handle, Vec2)> {
    let painter = ui.painter_at(
        response
            .interact_rect
            .union(map.rect)
            .intersect(ui.clip_rect()),
    );
    if state.grid > 0 {
        draw_grid(&painter, map, state.grid);
    }
    let surfaces: Vec<_> = editor
        .project
        .surfaces_for_output(output_id)
        .iter()
        .map(|s| (s.id.clone(), s.warp.clone(), s.masks.clone(), s.locked))
        .collect();
    let selected = match &editor.selection {
        Selection::Surface { id, mask } => Some((id.clone(), *mask)),
        _ => None,
    };
    let extra = editor.extra_selected.clone();
    for (id, warp, masks, locked) in &surfaces {
        let is_selected = selected.as_ref().is_some_and(|(s, _)| s == id);
        let is_extra = extra.contains(id);
        let color = if is_selected {
            Color32::from_rgb(90, 200, 255)
        } else if is_extra {
            Color32::from_rgb(90, 200, 255).gamma_multiply(0.6)
        } else if *locked {
            Color32::from_rgba_unmultiplied(200, 200, 200, 50)
        } else {
            Color32::from_rgba_unmultiplied(90, 200, 255, 60)
        };
        draw_wireframe(
            &painter,
            map,
            warp,
            masks,
            if is_selected {
                selected.as_ref().and_then(|s| s.1)
            } else {
                None
            },
            color,
        );
    }

    let Some((id, mask_index)) = selected else {
        handle_select_click(editor, response, map, &surfaces);
        return None;
    };
    let (_, warp, masks, locked) = surfaces.iter().find(|(s, ..)| *s == id).cloned()?;
    if locked {
        handle_select_click(editor, response, map, &surfaces);
        return None;
    }
    let mask_ref = mask_index.and_then(|i| masks.get(i));
    let list = handles(&warp, mask_ref);
    for (h, p) in &list {
        let pos = map.to_screen(*p);
        let active = state.active == Some(*h);
        painter.circle_filled(
            pos,
            if active { 6.0 } else { 4.5 },
            if active {
                Color32::YELLOW
            } else {
                Color32::WHITE
            },
        );
        painter.circle_stroke(pos, 7.0, Stroke::new(1.0, Color32::from_black_alpha(160)));
    }

    let pointer = response.interact_pointer_pos();
    let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
    if response.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin()).or(pointer);
        state.active = origin
            .and_then(|pos| pick(map, &list, pos, GRAB_RADIUS))
            .or(if shift { Some(Handle::All) } else { None });
    }
    if response.dragged() {
        if let (Some(h), Some(pos)) = (state.active, pointer) {
            let delta = map.delta(response.drag_delta());
            let mut p = map.to_norm(pos);
            if state.snap && !alt {
                p = snap(map, p, state.grid, SNAP_RADIUS);
            }
            apply_move(editor, &id, &warp, mask_ref, h, p, delta);
        }
    }
    if response.drag_stopped() {
        state.active = None;
        editor.commit();
    }
    if response.clicked() {
        if let Some(h) = pointer.and_then(|pos| pick(map, &list, pos, GRAB_RADIUS)) {
            state.active = Some(h);
        } else {
            state.active = None;
            handle_select_click(editor, response, map, &surfaces);
        }
    }

    // Arrow-key nudging of the active handle: 1 px, Shift ×10, Alt ×0.1.
    let mut readout = None;
    if let Some(h) = state.active {
        let step = if shift {
            10.0
        } else if alt {
            0.1
        } else {
            1.0
        };
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
        let output_px = editor
            .project
            .output(output_id)
            .map_or((1920.0, 1080.0), |o| {
                (f64::from(o.width), f64::from(o.height))
            });
        let current = list
            .iter()
            .find(|(hh, _)| *hh == h)
            .map(|(_, p)| *p)
            .unwrap_or_default();
        if dx != 0.0 || dy != 0.0 {
            let delta = Vec2::new(f64::from(dx) / output_px.0, f64::from(dy) / output_px.1);
            apply_move(editor, &id, &warp, mask_ref, h, current + delta, delta);
            editor.commit();
        }
        readout = Some((
            h,
            Vec2::new(current.x * output_px.0, current.y * output_px.1),
        ));
    }
    readout
}

/// A plain click on a surface's outline selects it (topmost first); Shift extends.
fn handle_select_click(
    editor: &mut Editor,
    response: &Response,
    map: &Mapping,
    surfaces: &[(String, WarpKind, Vec<Mask>, bool)],
) {
    if !response.clicked() {
        return;
    }
    let Some(pos) = response.interact_pointer_pos() else {
        return;
    };
    let p = map.to_norm(pos);
    let shift = response.ctx.input(|i| i.modifiers.shift);
    if let Some((id, ..)) = surfaces.iter().rev().find(|(_, w, ..)| contains(w, p)) {
        editor.select_surface(id, shift);
    } else if !shift {
        if let Selection::Surface { .. } = editor.selection {
            // Clicking empty space keeps the selection; a second click clears the mask.
            if let Selection::Surface { id, mask: Some(_) } = editor.selection.clone() {
                editor.selection = Selection::Surface { id, mask: None };
            }
        }
    }
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

fn draw_grid(painter: &egui::Painter, map: &Mapping, grid: u32) {
    let faint = Stroke::new(1.0, Color32::from_white_alpha(28));
    let mid = Stroke::new(1.0, Color32::from_white_alpha(70));
    for i in 1..grid {
        let t = f64::from(i) / f64::from(grid);
        let stroke = if 2 * i == grid { mid } else { faint };
        painter.line_segment(
            [
                map.to_screen(Vec2::new(t, 0.0)),
                map.to_screen(Vec2::new(t, 1.0)),
            ],
            stroke,
        );
        painter.line_segment(
            [
                map.to_screen(Vec2::new(0.0, t)),
                map.to_screen(Vec2::new(1.0, t)),
            ],
            stroke,
        );
    }
    if grid % 2 == 1 {
        painter.line_segment(
            [
                map.to_screen(Vec2::new(0.5, 0.0)),
                map.to_screen(Vec2::new(0.5, 1.0)),
            ],
            mid,
        );
        painter.line_segment(
            [
                map.to_screen(Vec2::new(0.0, 0.5)),
                map.to_screen(Vec2::new(1.0, 0.5)),
            ],
            mid,
        );
    }
}

/// Draws a warp's outline (and control net for meshes) plus its masks.
pub fn draw_wireframe(
    painter: &egui::Painter,
    map: &Mapping,
    warp: &WarpKind,
    masks: &[Mask],
    selected_mask: Option<usize>,
    color: Color32,
) {
    let stroke = Stroke::new(1.0, color);
    match warp {
        WarpKind::Quad(q) => {
            for i in 0..4 {
                painter.line_segment(
                    [
                        map.to_screen(q.corners[i]),
                        map.to_screen(q.corners[(i + 1) % 4]),
                    ],
                    stroke,
                );
            }
        }
        WarpKind::Bezier(b) => {
            let faint = Stroke::new(1.0, color.gamma_multiply(0.4));
            let (w, h) = (b.width(), b.height());
            for j in 0..h {
                for i in 0..w {
                    let p = b.point(i, j).unwrap_or_default();
                    if i + 1 < w {
                        painter.line_segment(
                            [
                                map.to_screen(p),
                                map.to_screen(b.point(i + 1, j).unwrap_or_default()),
                            ],
                            faint,
                        );
                    }
                    if j + 1 < h {
                        painter.line_segment(
                            [
                                map.to_screen(p),
                                map.to_screen(b.point(i, j + 1).unwrap_or_default()),
                            ],
                            faint,
                        );
                    }
                }
            }
            let n = 24;
            let edge = |f: &dyn Fn(f64) -> Vec2| {
                let pts: Vec<Pos2> = (0..=n)
                    .map(|k| map.to_screen(f(f64::from(k) / f64::from(n))))
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
        let c = if selected_mask == Some(i) {
            Color32::from_rgb(255, 160, 60)
        } else {
            Color32::from_rgba_unmultiplied(255, 160, 60, 110)
        };
        let n = m.points.len();
        for k in 0..n {
            painter.line_segment(
                [
                    map.to_screen(m.points[k]),
                    map.to_screen(m.points[(k + 1) % n]),
                ],
                Stroke::new(1.0, c),
            );
        }
    }
}

/// Draws the identify flash: output id, size and colour, big enough to read from the floor.
pub fn draw_identify(
    painter: &egui::Painter,
    rect: Rect,
    id: &str,
    width: u32,
    height: u32,
    index: usize,
) {
    let color = output_color(index);
    painter.rect_filled(rect, 0.0, color.gamma_multiply(0.35));
    painter.rect_stroke(
        rect.shrink(6.0),
        0.0,
        Stroke::new(12.0, color),
        egui::StrokeKind::Inside,
    );
    let size = (rect.height() * 0.18).clamp(24.0, 240.0);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        id,
        egui::FontId::proportional(size),
        Color32::WHITE,
    );
    painter.text(
        rect.center() + EVec2::new(0.0, size * 0.8),
        egui::Align2::CENTER_CENTER,
        format!("{width} × {height}"),
        egui::FontId::proportional(size * 0.4),
        Color32::WHITE,
    );
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used)]
mod tests {
    use super::*;
    use lumen_core::warp::{BezierMesh, Quad};

    fn map() -> Mapping {
        Mapping {
            rect: Rect::from_min_size(Pos2::new(10.0, 20.0), EVec2::new(200.0, 100.0)),
        }
    }

    #[test]
    fn mapping_round_trips() {
        let m = map();
        let p = Vec2::new(0.25, 0.5);
        let s = m.to_screen(p);
        assert_eq!(s, Pos2::new(60.0, 70.0));
        let back = m.to_norm(s);
        assert!((back.x - 0.25).abs() < 1e-6 && (back.y - 0.5).abs() < 1e-6);
        let d = m.delta(EVec2::new(20.0, 10.0));
        assert!((d.x - 0.1).abs() < 1e-6 && (d.y - 0.1).abs() < 1e-6);
        let zero = Mapping {
            rect: Rect::from_min_size(Pos2::ZERO, EVec2::ZERO),
        };
        assert!(zero.to_norm(Pos2::new(1.0, 1.0)).is_finite());
    }

    #[test]
    fn handles_and_picking() {
        let quad = WarpKind::Quad(Quad::full_frame());
        let list = handles(&quad, None);
        assert_eq!(list.len(), 4);
        assert_eq!(
            handles(&WarpKind::Bezier(BezierMesh::flat(1, 1).unwrap()), None).len(),
            16
        );
        let mask = Mask::hole(vec![
            Vec2::new(0.1, 0.1),
            Vec2::new(0.9, 0.1),
            Vec2::new(0.5, 0.9),
        ]);
        assert_eq!(handles(&quad, Some(&mask)).len(), 3);
        let m = Mapping {
            rect: Rect::from_min_size(Pos2::ZERO, EVec2::new(100.0, 100.0)),
        };
        assert_eq!(
            pick(&m, &list, Pos2::new(97.0, 2.0), GRAB_RADIUS),
            Some(Handle::Warp(1))
        );
        assert_eq!(pick(&m, &list, Pos2::new(50.0, 50.0), GRAB_RADIUS), None);
        assert_eq!(
            pick(&m, &list, Pos2::new(50.0, 50.0), 1000.0),
            Some(Handle::Warp(0))
        );
    }

    #[test]
    fn snapping() {
        let m = Mapping {
            rect: Rect::from_min_size(Pos2::ZERO, EVec2::new(1000.0, 500.0)),
        };
        let s = snap(&m, Vec2::new(0.004, 0.49), 0, SNAP_RADIUS);
        assert_eq!(s, Vec2::new(0.0, 0.5), "edge and centre within 8 px");
        let s = snap(&m, Vec2::new(0.3, 0.3), 0, SNAP_RADIUS);
        assert_eq!(s, Vec2::new(0.3, 0.3), "nothing near");
        let s = snap(&m, Vec2::new(0.303, 0.3), 10, SNAP_RADIUS);
        assert_eq!(s.x, 0.3, "grid line at 0.3");
        assert_eq!(
            snap(&m, Vec2::new(0.303, 0.3), 10, 0.0),
            Vec2::new(0.303, 0.3),
            "zero radius never snaps"
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
        let (w, _) = moved(&bez, None, Handle::Warp(99), Vec2::ZERO, Vec2::ZERO);
        assert_eq!(w, Some(bez.clone()));
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

    #[test]
    fn containment_and_colours() {
        let q = WarpKind::Quad(Quad {
            corners: [
                Vec2::new(0.2, 0.2),
                Vec2::new(0.8, 0.2),
                Vec2::new(0.8, 0.8),
                Vec2::new(0.2, 0.8),
            ],
        });
        assert!(contains(&q, Vec2::new(0.5, 0.5)));
        assert!(!contains(&q, Vec2::new(0.1, 0.5)));
        let b = WarpKind::Bezier(BezierMesh::flat(2, 2).unwrap());
        assert!(contains(&b, Vec2::new(0.5, 0.5)));
        assert!(!contains(&b, Vec2::new(1.5, 0.5)));
        assert_ne!(output_color(0), output_color(1));
        assert_eq!(output_color(0), output_color(6));
    }
}
