//! Output windows: one borderless, always-on-top viewport per open output.

use super::{monitor_at, physical_to_points, scene_for, Editor, MonitorInfo};
use eframe::egui;
use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use lumen_render::gpu::GpuRenderer;
use lumen_render::Scene;
use std::sync::Arc;

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

/// Shows every open output window.
pub fn show_output_windows(
    ctx: &egui::Context,
    editor: &mut Editor,
    monitors: &[MonitorInfo],
    format: wgpu::TextureFormat,
) {
    let ids = editor.open_outputs.clone();
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
        ctx.show_viewport_immediate(viewport_id, builder, |ui, _class| {
            let rect = ui.max_rect();
            ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
            if let Some(scene) = &scene {
                paint_scene(ui, rect, format!("output:{id}"), Arc::clone(scene), format);
            }
            let info = ui.input(|i| i.viewport().clone());
            close = info.close_requested();
            outer = info.outer_rect;
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
        }
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
