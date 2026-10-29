//! The egui editor and show player.

mod canvas;
mod outputs;
mod panels;
mod state;

use crate::watchdog::{Heartbeat, Watchdog};
use eframe::egui;
use lumen_core::project::{demo_project, Project};
use lumen_render::gpu::GpuRenderer;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

pub use state::{Editor, Selection};

/// How the app was started.
#[derive(Debug, Clone)]
pub struct Launch {
    /// Project to open; `None` starts with the demo.
    pub project: Option<PathBuf>,
    /// OSC port, `0` to disable.
    pub osc_port: u16,
    /// Start in show mode.
    pub show: bool,
    /// Relaunch after a stall of this many seconds (show mode only).
    pub restart_on_stall: Option<f64>,
}

/// A monitor the OS reported at startup.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorInfo {
    /// OS name, e.g. `\\.\DISPLAY2`.
    pub name: String,
    /// Origin in physical pixels of the virtual desktop.
    pub x: i32,
    /// Origin in physical pixels of the virtual desktop.
    pub y: i32,
    /// Size in physical pixels.
    pub width: u32,
    /// Size in physical pixels.
    pub height: u32,
    /// DPI scale.
    pub scale: f64,
}

/// Runs the editor until the window closes.
pub fn run(launch: &Launch) -> anyhow::Result<ExitCode> {
    let editor = match &launch.project {
        Some(path) => {
            let mut editor = Editor::open(path)?;
            if let Some(auto) = crate::autosave::recoverable(path) {
                editor.status = format!("autosave found: {}", auto.display());
                editor.recovery = Some(auto);
            }
            editor
        }
        None => Editor::new(demo_project(), None),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("lumen")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 500.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let launch_for_app = launch.clone();
    eframe::run_native(
        "lumen",
        options,
        Box::new(move |cc| {
            let Some(render_state) = cc.wgpu_render_state.as_ref() else {
                return Err("lumen needs the wgpu backend".into());
            };
            render_state
                .renderer
                .write()
                .callback_resources
                .insert(GpuRenderer::new(
                    render_state.device.clone(),
                    render_state.queue.clone(),
                ));
            let monitors = cc
                .winit_window()
                .map(|w| {
                    w.available_monitors()
                        .map(|m| MonitorInfo {
                            name: m.name().unwrap_or_default(),
                            x: m.position().x,
                            y: m.position().y,
                            width: m.size().width,
                            height: m.size().height,
                            scale: m.scale_factor(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(Box::new(LumenApp::new(
                editor,
                &launch_for_app,
                monitors,
                render_state.target_format,
            )))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(ExitCode::SUCCESS)
}

/// The eframe application.
pub struct LumenApp {
    editor: Editor,
    monitors: Vec<MonitorInfo>,
    surface_format: wgpu::TextureFormat,
    heartbeat: Heartbeat,
    _watchdog: Option<Watchdog>,
    canvas: canvas::CanvasState,
    panels: panels::PanelState,
    frame_times: std::collections::VecDeque<Instant>,
}

impl LumenApp {
    fn new(
        mut editor: Editor,
        launch: &Launch,
        monitors: Vec<MonitorInfo>,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        editor.set_osc_port(launch.osc_port);
        if launch.show {
            editor.enter_show_mode();
        }
        let heartbeat = Heartbeat::new();
        let watchdog = launch.restart_on_stall.filter(|s| *s > 0.0).map(|secs| {
            let limit = Duration::from_secs_f64(secs);
            Watchdog::spawn(heartbeat.clone(), limit, move |since| {
                log::error!("render loop stalled for {since:?}; relaunching");
                let err = crate::watchdog::relaunch_self();
                log::error!("relaunch failed: {err}");
            })
        });
        Self {
            editor,
            monitors,
            surface_format,
            heartbeat,
            _watchdog: watchdog,
            canvas: canvas::CanvasState::default(),
            panels: panels::PanelState::default(),
            frame_times: std::collections::VecDeque::new(),
        }
    }

    fn fps(&self) -> f32 {
        let n = self.frame_times.len();
        if n < 2 {
            return 0.0;
        }
        let span = self.frame_times[n - 1]
            .duration_since(self.frame_times[0])
            .as_secs_f32();
        if span <= 0.0 {
            0.0
        } else {
            (n - 1) as f32 / span
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let (undo, redo, save, show, escape, delete) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
                    || i.consume_key(
                        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                        egui::Key::Z,
                    ),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
                i.consume_key(egui::Modifiers::NONE, egui::Key::F10),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Delete) && !i.focused,
            )
        });
        let typing = ctx.egui_wants_keyboard_input();
        if undo && !typing {
            self.editor.undo();
        }
        if redo && !typing {
            self.editor.redo();
        }
        if save {
            self.panels.save(&mut self.editor);
        }
        if show {
            if self.editor.show_mode {
                self.editor.exit_show_mode();
            } else {
                self.editor.enter_show_mode();
            }
        }
        if escape {
            self.editor.press_escape(now, Duration::from_secs(2));
        }
        if delete && !typing {
            self.editor.delete_selection();
        }
    }
}

impl eframe::App for LumenApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.heartbeat.beat();
        let now = Instant::now();
        self.frame_times.push_back(now);
        while self.frame_times.len() > 60 {
            self.frame_times.pop_front();
        }
        self.editor.sources.poll();
        if self.editor.poll_osc() > 0 {
            ctx.request_repaint();
        }
        self.editor.maybe_autosave(now);
        self.handle_shortcuts(&ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.editor.title()));

        if self.editor.show_mode {
            let fps = self.fps();
            panels::show_mode_screen(ui, &self.editor, fps);
        } else {
            panels::top_bar(ui, &mut self.editor, &mut self.panels);
            panels::left_panel(ui, &mut self.editor);
            panels::inspector(ui, &mut self.editor, &self.monitors, &mut self.panels);
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(egui::Color32::from_gray(18)))
                .show(ui, |ui| {
                    canvas::show(ui, &mut self.editor, &mut self.canvas, self.surface_format);
                });
            panels::dialogs(&ctx, &mut self.editor, &mut self.panels);
        }
        outputs::show_output_windows(&ctx, &mut self.editor, &self.monitors, self.surface_format);

        // Keep live sources and the show-mode status fresh without spinning when idle.
        if !self.editor.sources.live_ids().is_empty() || self.editor.show_mode {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}

/// Converts physical desktop pixels to egui points for a viewport on a monitor with `scale`.
pub fn physical_to_points(px: i32, scale: f64) -> f32 {
    (f64::from(px) / scale.max(0.01)) as f32
}

/// Chooses the monitor whose area contains `(x, y)`, else the first, else none.
pub fn monitor_at(monitors: &[MonitorInfo], x: i32, y: i32) -> Option<&MonitorInfo> {
    monitors
        .iter()
        .find(|m| x >= m.x && y >= m.y && x < m.x + m.width as i32 && y < m.y + m.height as i32)
        .or_else(|| monitors.first())
}

/// Builds a project scene for display, ignoring errors (a broken warp just draws nothing).
pub fn scene_for(editor: &mut Editor, output: &str) -> Option<lumen_render::Scene> {
    let project: &Project = &editor.project;
    lumen_render::Scene::build_cached(
        project,
        output,
        &editor.sources,
        lumen_render::scene::DEFAULT_MESH_DENSITY,
        &mut editor.mask_cache,
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_conversions_and_monitor_lookup() {
        assert!((physical_to_points(200, 2.0) - 100.0).abs() < 1e-6);
        assert!((physical_to_points(200, 0.0) - 20000.0).abs() < 1e-3);
        let monitors = vec![
            MonitorInfo {
                name: "a".into(),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 1.0,
            },
            MonitorInfo {
                name: "b".into(),
                x: 1920,
                y: 0,
                width: 1280,
                height: 720,
                scale: 1.5,
            },
        ];
        assert_eq!(
            monitor_at(&monitors, 2000, 10).map(|m| m.name.as_str()),
            Some("b")
        );
        assert_eq!(
            monitor_at(&monitors, -5, -5).map(|m| m.name.as_str()),
            Some("a")
        );
        assert_eq!(monitor_at(&[], 0, 0), None);
    }

    #[test]
    fn scene_for_ignores_bad_outputs() {
        let mut e = Editor::new(demo_project(), None);
        assert!(scene_for(&mut e, "left").is_some());
        assert!(scene_for(&mut e, "ghost").is_none());
    }
}
