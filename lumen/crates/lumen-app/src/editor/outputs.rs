//! Output windows: one borderless, always-on-top viewport per open output.

use super::handles::{self, HandleState, Mapping};
use super::{monitor_at, physical_to_points, scene_for, Editor, MonitorInfo};
use eframe::egui::{self, Sense};
use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use lumen_render::gpu::GpuRenderer;
use lumen_render::Scene;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Paints a scene into the current egui render pass through the shared [`GpuRenderer`].
pub struct ScenePaint {
    /// Composite key (`output:<id>` or `preview:<id>`).
    pub key: String,
    /// What to draw.
    pub scene: Arc<Scene>,
    /// Target size in physical pixels.
    pub size: [u32; 2],
    /// Surface format of the window being drawn into.
    pub format: wgpu::TextureFormat,
}

impl CallbackTrait for ScenePaint {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(renderer) = resources.get_mut::<GpuRenderer>() {
            renderer.prepare(
                &self.key,
                &self.scene,
                self.size[0],
                self.size[1],
                self.format,
                encoder,
            );
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        if let Some(renderer) = resources.get::<GpuRenderer>() {
            renderer.paint(&self.key, self.format, pass);
        }
    }
}

/// Adds a scene paint callback covering `rect`.
pub fn paint_scene(
    ui: &egui::Ui,
    rect: egui::Rect,
    key: String,
    scene: Arc<Scene>,
    format: wgpu::TextureFormat,
) {
    let ppp = ui.ctx().pixels_per_point();
    let size = [
        (rect.width() * ppp).round().max(1.0) as u32,
        (rect.height() * ppp).round().max(1.0) as u32,
    ];
    ui.painter().add(egui_wgpu::Callback::new_paint_callback(
        rect,
        ScenePaint {
            key,
            scene,
            size,
            format,
        },
    ));
}

/// Where and how big an output window should be, in egui points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowSpec {
    /// Outer position.
    pub pos: Option<[f32; 2]>,
    /// Inner size.
    pub size: [f32; 2],
    /// Borderless, always-on-top projector window versus a movable preview.
    pub locked: bool,
}

/// Decides the window for an output: placed outputs open borderless at full size on their
/// monitor; unplaced ones open as a small decorated preview the user can drag to a projector.
pub fn window_spec(output: &lumen_core::project::Output, monitors: &[MonitorInfo]) -> WindowSpec {
    if let Some(p) = &output.placement {
        let scale = monitor_at(monitors, p.x, p.y).map_or(1.0, |m| m.scale);
        WindowSpec {
            pos: Some([
                physical_to_points(p.x, scale),
                physical_to_points(p.y, scale),
            ]),
            size: [
                physical_to_points(output.width as i32, scale),
                physical_to_points(output.height as i32, scale),
            ],
            locked: true,
        }
    } else {
        let aspect = output.width.max(1) as f32 / output.height.max(1) as f32;
        WindowSpec {
            pos: None,
            size: [480.0, (480.0 / aspect).max(120.0)],
            locked: false,
        }
    }
}

/// Idle time after which the pointer disappears over an output window.
pub const CURSOR_HIDE_AFTER: Duration = Duration::from_secs(2);

/// Per-window interaction state.
#[derive(Debug)]
pub struct OutputUiState {
    /// Handle interaction on the wall.
    pub handles: HandleState,
    /// Last pointer movement seen on this window.
    pub last_move: Instant,
    /// Last pointer position seen.
    pub last_pos: Option<egui::Pos2>,
}

impl Default for OutputUiState {
    fn default() -> Self {
        Self {
            handles: HandleState::default(),
            last_move: Instant::now(),
            last_pos: None,
        }
    }
}

/// True when the pointer has not moved for [`CURSOR_HIDE_AFTER`].
pub fn cursor_idle(state: &mut OutputUiState, pos: Option<egui::Pos2>, now: Instant) -> bool {
    if pos != state.last_pos {
        state.last_pos = pos;
        state.last_move = now;
    }
    now.duration_since(state.last_move) >= CURSOR_HIDE_AFTER
}

/// Shows every open output window.
///
/// Outside show mode the windows carry the same handles as the canvas when
/// `editor.edit_on_wall` is set, so a surface can be calibrated while looking at the wall.
#[allow(clippy::too_many_lines)]
pub fn show_output_windows(
    ctx: &egui::Context,
    editor: &mut Editor,
    monitors: &[MonitorInfo],
    format: wgpu::TextureFormat,
    ui_state: &mut HashMap<String, OutputUiState>,
    snap: bool,
    grid: u32,
) {
    let ids = editor.open_outputs.clone();
    ui_state.retain(|k, _| ids.contains(k));
    let now = Instant::now();
    let identify = editor.identify_active(now);
    for id in ids {
        let Some(output) = editor.project.output(&id).cloned() else {
            editor.open_outputs.retain(|o| *o != id);
            continue;
        };
        let spec = window_spec(&output, monitors);
        let mut builder = egui::ViewportBuilder::default()
            .with_title(format!("lumen output: {id}"))
            .with_inner_size(spec.size)
            .with_decorations(!spec.locked)
            .with_resizable(!spec.locked)
            .with_taskbar(!spec.locked);
        if let Some(pos) = spec.pos {
            builder = builder.with_position(pos);
        }
        if spec.locked {
            builder = builder.with_window_level(egui::WindowLevel::AlwaysOnTop);
        }
        let scene = scene_for(editor, &id).map(Arc::new);
        let viewport_id = egui::ViewportId::from_hash_of(("lumen-output", &id));
        let mut close = false;
        let mut outer = None;
        let index = editor
            .project
            .outputs
            .iter()
            .position(|o| o.id == id)
            .unwrap_or(0);
        let state = ui_state.entry(id.clone()).or_default();
        state.handles.snap = snap;
        state.handles.grid = 0;
        let editing = !editor.show_mode && editor.edit_on_wall;
        ctx.show_viewport_immediate(viewport_id, builder, |ui, class| {
            let rect = ui.max_rect();
            ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
            if let Some(scene) = &scene {
                paint_scene(ui, rect, format!("output:{id}"), Arc::clone(scene), format);
            }
            if editing {
                let response = ui.allocate_rect(rect, Sense::click_and_drag());
                let map = Mapping { rect };
                state.handles.grid = grid;
                handles::interact(ui, editor, &mut state.handles, &response, &map, &id);
            }
            if identify {
                handles::draw_identify(ui.painter(), rect, &id, output.width, output.height, index);
            }
            let pos = ui.input(|i| i.pointer.hover_pos());
            if cursor_idle(state, pos, now) || editor.show_mode {
                ui.ctx().set_cursor_icon(egui::CursorIcon::None);
            }
            let info = ui.input(|i| i.viewport().clone());
            close = info.close_requested();
            // A backend without native viewports embeds the output as an egui window; its
            // rect inside the editor is still a useful "position" for the placement button.
            outer = if class == egui::ViewportClass::Immediate {
                info.outer_rect
            } else {
                Some(ui.max_rect())
            };
        });
        if let Some(r) = outer {
            editor
                .window_positions
                .insert(id.clone(), (r.min.x, r.min.y));
        }
        if close && !editor.show_mode {
            editor.open_outputs.retain(|o| *o != id);
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use lumen_core::project::{Output, Placement};

    fn output(placement: Option<Placement>) -> Output {
        Output {
            id: "o".into(),
            width: 1920,
            height: 1080,
            enabled: true,
            blend: lumen_core::blend::EdgeBlend::none(),
            color: lumen_core::project::ColorAdjust::default(),
            placement,
            overlay: None,
        }
    }

    #[test]
    fn cursor_hides_after_idle_and_returns_on_movement() {
        let mut s = OutputUiState::default();
        let t = Instant::now();
        assert!(!cursor_idle(&mut s, Some(egui::Pos2::new(1.0, 1.0)), t));
        assert!(cursor_idle(
            &mut s,
            Some(egui::Pos2::new(1.0, 1.0)),
            t + Duration::from_secs(3)
        ));
        assert!(!cursor_idle(
            &mut s,
            Some(egui::Pos2::new(2.0, 1.0)),
            t + Duration::from_secs(3)
        ));
        assert!(
            !cursor_idle(&mut s, None, t + Duration::from_secs(4)),
            "leaving the window counts as movement"
        );
    }

    #[test]
    fn placed_outputs_are_locked_full_size_windows() {
        let monitors = vec![MonitorInfo {
            name: "m".into(),
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 2.0,
        }];
        let spec = window_spec(
            &output(Some(Placement {
                monitor: "m".into(),
                x: 1920,
                y: 0,
            })),
            &monitors,
        );
        assert_eq!(
            spec,
            WindowSpec {
                pos: Some([960.0, 0.0]),
                size: [960.0, 540.0],
                locked: true
            }
        );
        let spec = window_spec(
            &output(Some(Placement {
                monitor: String::new(),
                x: 10,
                y: 10,
            })),
            &[],
        );
        assert_eq!(spec.pos, Some([10.0, 10.0]));
        assert!(spec.locked);
    }

    #[test]
    fn unplaced_outputs_are_small_previews() {
        let spec = window_spec(&output(None), &[]);
        assert_eq!(
            spec,
            WindowSpec {
                pos: None,
                size: [480.0, 270.0],
                locked: false
            }
        );
        let mut tall = output(None);
        tall.width = 100;
        tall.height = 1000;
        assert_eq!(window_spec(&tall, &[]).size[1], 4800.0);
    }
}
