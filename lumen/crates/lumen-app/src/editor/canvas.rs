//! The preview canvas: the selected output rendered live, with zoom, pan, grid and handles.

use super::handles::{self, HandleState, Mapping};
use super::outputs::paint_scene;
use super::{scene_for, Editor, Selection};
use eframe::egui::{self, Color32, Rect, Sense, Stroke, Vec2 as EVec2};
use std::sync::Arc;

/// Zoom limits.
pub const MIN_ZOOM: f32 = 0.25;
/// Zoom limits.
pub const MAX_ZOOM: f32 = 16.0;

/// Canvas state.
#[derive(Debug)]
pub struct CanvasState {
    /// Handle interaction.
    pub handles: HandleState,
    /// Which output the canvas shows; defaults to the selection's output or the first.
    pub output: Option<String>,
    /// Screen rect of the (zoomed) output during the last frame; tests aim the pointer here.
    pub last_rect: Option<Rect>,
    /// Zoom factor, 1 = fit.
    pub zoom: f32,
    /// Pan offset in points.
    pub pan: EVec2,
    /// Draw the grid.
    pub show_grid: bool,
    /// Grid divisions.
    pub grid_divisions: u32,
    /// Readout from the last frame (handle and output-pixel position).
    pub readout: Option<(handles::Handle, lumen_core::geom::Vec2)>,
    /// Stage view (every placed output on the virtual desktop) instead of one output.
    pub stage: bool,
    /// Screen rects of the outputs drawn by the stage view last frame; tests aim here.
    pub stage_rects: Vec<(String, Rect)>,
    /// Output being dragged on the stage and the desktop offset accumulated so far.
    pub stage_drag: Option<(String, egui::Vec2)>,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            handles: HandleState {
                active: None,
                snap: true,
                grid: 0,
            },
            output: None,
            last_rect: None,
            zoom: 1.0,
            pan: EVec2::ZERO,
            show_grid: false,
            grid_divisions: 10,
            readout: None,
            stage: false,
            stage_rects: Vec::new(),
            stage_drag: None,
        }
    }
}

impl CanvasState {
    /// Fit the output in the view again.
    pub fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.pan = EVec2::ZERO;
    }

    /// Zooms by `factor` keeping `anchor` (a screen position) fixed, given the current rect.
    pub fn zoom_at(&mut self, factor: f32, anchor: egui::Pos2, base: Rect) {
        let old = self.zoom;
        let new = (old * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        if (new - old).abs() < f32::EPSILON {
            return;
        }
        // Keep the point under the pointer still: pan += (anchor - centre - pan) * (1 - new/old)
        let centre = base.center().to_vec2();
        let offset = anchor.to_vec2() - centre - self.pan;
        self.pan += offset * (1.0 - new / old);
        self.zoom = new;
    }
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

/// Applies zoom and pan to the fitted rect.
pub fn view_rect(base: Rect, zoom: f32, pan: EVec2) -> Rect {
    Rect::from_center_size(base.center() + pan, base.size() * zoom)
}

/// Draws and runs the canvas for one frame.
#[allow(clippy::too_many_lines)]
pub fn show(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    state: &mut CanvasState,
    format: wgpu::TextureFormat,
) {
    let selected_output = match &editor.selection {
        Selection::Output(id) => Some(id.clone()),
        Selection::Surface { id, .. } => editor.project.surface(id).map(|s| s.output.clone()),
        Selection::None | Selection::Source(_) => None,
    };
    if let Some(o) = selected_output {
        if state.output.as_ref() != Some(&o) {
            state.output = Some(o);
        }
    }
    if state
        .output
        .as_ref()
        .is_none_or(|o| editor.project.output(o).is_none())
    {
        state.output = editor.project.outputs.first().map(|o| o.id.clone());
    }
    let Some(output_id) = state.output.clone() else {
        state.last_rect = None;
        ui.centered_and_justified(|ui| {
            ui.label("Add an output to start mapping (or drop an image here).");
        });
        return;
    };
    let Some(output) = editor.project.output(&output_id).cloned() else {
        return;
    };

    let avail = ui.available_rect_before_wrap().shrink(12.0);
    let base = fit_rect(avail, output.width as f32 / output.height.max(1) as f32);
    let rect = view_rect(base, state.zoom, state.pan);
    state.last_rect = Some(rect);
    state.handles.grid = if state.show_grid {
        state.grid_divisions
    } else {
        0
    };

    // The whole available area takes input so zoom and pan work outside the output too.
    let response = ui.allocate_rect(avail, Sense::click_and_drag());
    let painter = ui.painter_at(avail);
    painter.rect_filled(rect, 0.0, Color32::BLACK);
    if let Some(scene) = scene_for(editor, &output_id) {
        ui.set_clip_rect(avail);
        paint_scene(
            ui,
            rect,
            format!("preview:{output_id}"),
            Arc::new(scene),
            format,
        );
    }
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, Color32::from_gray(90)),
        egui::StrokeKind::Outside,
    );

    // Zoom with the wheel, pan with the middle button (or Ctrl-drag).
    let (scroll, pointer, middle_down, ctrl) = ui.input(|i| {
        (
            i.raw
                .events
                .iter()
                .map(|e| match e {
                    egui::Event::MouseWheel { delta, unit, .. } => match unit {
                        egui::MouseWheelUnit::Point => delta.y,
                        egui::MouseWheelUnit::Line => delta.y * 40.0,
                        egui::MouseWheelUnit::Page => delta.y * 400.0,
                    },
                    _ => 0.0,
                })
                .sum::<f32>(),
            i.pointer.hover_pos(),
            i.pointer.middle_down(),
            i.modifiers.ctrl || i.modifiers.command,
        )
    });
    if scroll != 0.0 && response.hovered() {
        if let Some(p) = pointer {
            state.zoom_at((scroll / 200.0).exp(), p, base);
        }
    }
    if response.dragged() && (middle_down || ctrl) {
        state.pan += response.drag_delta();
    } else {
        let map = Mapping { rect };
        state.readout =
            handles::interact(ui, editor, &mut state.handles, &response, &map, &output_id);
    }
    if response.double_clicked_by(egui::PointerButton::Middle) {
        state.reset_view();
    }

    // Identify flash and labels.
    let now = std::time::Instant::now();
    if editor.identify_active(now) {
        let index = editor
            .project
            .outputs
            .iter()
            .position(|o| o.id == output_id)
            .unwrap_or(0);
        handles::draw_identify(
            &painter,
            rect.intersect(avail),
            &output_id,
            output.width,
            output.height,
            index,
        );
    }
    painter.text(
        avail.min + EVec2::new(6.0, 4.0),
        egui::Align2::LEFT_TOP,
        format!(
            "{output_id}  {}×{}  {:.0}%",
            output.width,
            output.height,
            state.zoom * 100.0
        ),
        egui::FontId::proportional(12.0),
        Color32::from_white_alpha(140),
    );
    if let Some((h, p)) = state.readout {
        painter.text(
            avail.left_bottom() + EVec2::new(6.0, -4.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{h:?}: {:.1}, {:.1} px", p.x, p.y),
            egui::FontId::monospace(12.0),
            Color32::from_white_alpha(180),
        );
    }
    if let Selection::Surface { id, .. } = &editor.selection {
        if editor.is_locked(id) {
            painter.text(
                avail.right_bottom() + EVec2::new(-6.0, -4.0),
                egui::Align2::RIGHT_BOTTOM,
                "locked",
                egui::FontId::proportional(12.0),
                Color32::from_rgb(255, 170, 60),
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use eframe::egui::Pos2;

    #[test]
    fn fit_rect_keeps_aspect() {
        let avail = Rect::from_min_size(Pos2::ZERO, EVec2::new(400.0, 400.0));
        let r = fit_rect(avail, 2.0);
        assert_eq!(r.size(), EVec2::new(400.0, 200.0));
        assert_eq!(r.center(), avail.center());
        let r = fit_rect(avail, 0.5);
        assert_eq!(r.size(), EVec2::new(200.0, 400.0));
        let nan = fit_rect(avail, f32::NAN);
        assert!((nan.size().x / nan.size().y - 16.0 / 9.0).abs() < 1e-5);
    }

    #[test]
    fn zoom_keeps_the_anchor_still() {
        let base = Rect::from_min_size(Pos2::ZERO, EVec2::new(400.0, 200.0));
        let mut s = CanvasState::default();
        let anchor = Pos2::new(100.0, 50.0);
        let before = view_rect(base, s.zoom, s.pan);
        let norm = (
            (anchor.x - before.min.x) / before.width(),
            (anchor.y - before.min.y) / before.height(),
        );
        s.zoom_at(2.0, anchor, base);
        let after = view_rect(base, s.zoom, s.pan);
        let x = after.min.x + norm.0 * after.width();
        let y = after.min.y + norm.1 * after.height();
        assert!((x - anchor.x).abs() < 1e-3 && (y - anchor.y).abs() < 1e-3);
        s.zoom_at(1000.0, anchor, base);
        assert_eq!(s.zoom, MAX_ZOOM);
        s.zoom_at(0.0001, anchor, base);
        assert_eq!(s.zoom, MIN_ZOOM);
        let pan = s.pan;
        s.zoom_at(1.0, anchor, base);
        assert_eq!(s.pan, pan, "no-op zoom leaves the pan alone");
        s.reset_view();
        assert_eq!((s.zoom, s.pan), (1.0, EVec2::ZERO));
    }
}
