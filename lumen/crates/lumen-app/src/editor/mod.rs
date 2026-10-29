//! The egui editor and show player.

mod canvas;
mod handles;
mod outputs;
mod panels;
mod stage;
mod state;
mod thumbs;
#[cfg(test)]
mod ui_tests;

use crate::watchdog::{Heartbeat, Watchdog};
use eframe::egui;
use lumen_core::project::{demo_project, Project};
use lumen_render::gpu::GpuRenderer;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

pub use state::{AlignOp, Editor, Pending, Selection};

use crate::settings::Settings;
use std::collections::HashMap;

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
    /// Phone remote port from the command line; `None` uses the saved setting.
    pub remote_port: Option<u16>,
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
    let settings = Settings::load();
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
    let _ = &settings;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("lumen")
            .with_app_id("lumen")
            .with_icon(crate::theme::icon_data(64))
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
            crate::theme::apply(&cc.egui_ctx);
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
            let mut app = LumenApp::new(
                editor,
                &launch_for_app,
                monitors,
                render_state.target_format,
            );
            app.apply_settings(Settings::load());
            app.panels.welcome = launch_for_app.project.is_none() && !app.settings.skip_welcome;
            Ok(Box::new(app))
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
    last_title: String,
    /// Per-output-window interaction state.
    wall: HashMap<String, outputs::OutputUiState>,
    /// User preferences.
    settings: Settings,
    /// Set once the user confirmed quitting with unsaved changes.
    close_confirmed: bool,
    /// Files are being dragged over the window.
    drop_hover: bool,
    /// Frame clock origin.
    started: Instant,
}

impl LumenApp {
    fn new(
        mut editor: Editor,
        launch: &Launch,
        monitors: Vec<MonitorInfo>,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        editor.set_osc_port(launch.osc_port);
        if let Some(port) = launch.remote_port {
            if let Err(e) = editor.set_remote_port(port) {
                log::warn!("remote: {e}");
            }
        }
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
            last_title: String::new(),
            wall: HashMap::new(),
            settings: Settings::default(),
            close_confirmed: false,
            drop_hover: false,
            started: Instant::now(),
        }
    }

    /// Applies loaded preferences to the live state.
    pub fn apply_settings(&mut self, settings: Settings) {
        if !settings.ffmpeg_path.is_empty() {
            apply_ffmpeg_path(&settings.ffmpeg_path);
            self.panels.ffmpeg_ok = lumen_sources::video::Tools::from_env().available();
            self.panels.ffmpeg_path.clone_from(&settings.ffmpeg_path);
        }
        if !settings.midi_port.is_empty() {
            self.panels.midi_port.clone_from(&settings.midi_port);
            if let Err(e) = self.editor.set_midi_port(Some(&settings.midi_port)) {
                log::warn!("midi: {e}");
            }
        }
        if !settings.osc_feedback.is_empty() {
            self.panels
                .feedback_target
                .clone_from(&settings.osc_feedback);
            if let Err(e) = self.editor.set_feedback(&settings.osc_feedback) {
                log::warn!("osc feedback: {e}");
            }
        }
        if settings.remote_port != 0 {
            self.panels.remote_port = settings.remote_port;
            if self.editor.remote.is_none() {
                if let Err(e) = self.editor.set_remote_port(settings.remote_port) {
                    log::warn!("remote: {e}");
                }
            }
        }
        self.canvas.show_grid = settings.show_grid;
        self.canvas.grid_divisions = settings.grid_divisions.clamp(2, 64);
        self.canvas.handles.snap = settings.snap;
        self.editor.edit_on_wall = settings.edit_on_wall;
        self.editor.autosave =
            crate::autosave::Autosave::new(Duration::from_secs(settings.autosave_seconds.max(2)));
        self.settings = settings;
    }

    /// Copies live toggles back into the preferences and saves them.
    fn persist_settings(&mut self) {
        let changed = self.settings.show_grid != self.canvas.show_grid
            || self.settings.snap != self.canvas.handles.snap
            || self.settings.edit_on_wall != self.editor.edit_on_wall;
        if changed {
            self.settings.show_grid = self.canvas.show_grid;
            self.settings.snap = self.canvas.handles.snap;
            self.settings.edit_on_wall = self.editor.edit_on_wall;
            self.settings.save();
        }
    }

    /// Remembers an opened or saved project in the recent list.
    fn note_recent(&mut self) {
        if let Some(p) = self.editor.path.clone() {
            if self.settings.recent.first() != Some(&p) {
                self.settings.push_recent(&p);
                self.settings.save();
            }
        }
    }

    /// Carries out an action the user confirmed (or that needed no confirmation).
    pub fn perform(&mut self, action: Pending) {
        match action {
            Pending::Close => {
                self.close_confirmed = true;
                self.editor.osc = None;
            }
            Pending::New => {
                self.editor = Editor::new(demo_project(), None);
                self.editor.set_osc_port(self.settings.osc_port);
                self.canvas.reset_view();
                self.panels.welcome = true;
            }
            Pending::Open(path) => match Editor::open(&path) {
                Ok(mut e) => {
                    e.set_osc_port(self.settings.osc_port);
                    e.edit_on_wall = self.editor.edit_on_wall;
                    if let Some(auto) = crate::autosave::recoverable(&path) {
                        e.recovery = Some(auto);
                    }
                    self.editor = e;
                    self.canvas.reset_view();
                    self.panels.welcome = false;
                    self.note_recent();
                }
                Err(e) => self.panels.error = Some(format!("{}: {e}", path.display())),
            },
        }
    }

    /// Asks for an action, deferring it behind the unsaved-changes prompt when needed.
    pub fn request(&mut self, action: Pending) {
        if self.editor.request(action.clone()) {
            self.perform(action);
        }
    }

    /// Handles the window's close button: with unsaved changes the close is cancelled and
    /// the prompt opens instead.
    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.close_confirmed {
            return;
        }
        if self.editor.dirty && self.editor.pending.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.editor.pending = Some(Pending::Close);
        } else if self.editor.dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
    }

    /// Files dropped onto any lumen window become sources.
    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if !dropped.is_empty() {
            self.editor.add_files(&dropped);
            self.panels.welcome = false;
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

    #[allow(clippy::too_many_lines)]
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        let now = Instant::now();
        let typing = ctx.egui_wants_keyboard_input();
        let k = ctx.input_mut(|i| Shortcuts {
            undo: i.consume_key(Modifiers::COMMAND, Key::Z),
            redo: i.consume_key(Modifiers::COMMAND, Key::Y)
                || i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
            save: i.consume_key(Modifiers::COMMAND, Key::S),
            save_as: i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S),
            open: i.consume_key(Modifiers::COMMAND, Key::O),
            new: i.consume_key(Modifiers::COMMAND, Key::N),
            duplicate: i.consume_key(Modifiers::COMMAND, Key::D),
            show: i.consume_key(Modifiers::NONE, Key::F10),
            escape: i.consume_key(Modifiers::NONE, Key::Escape),
            go: !typing && i.consume_key(Modifiers::NONE, Key::Space),
            delete: i.consume_key(Modifiers::NONE, Key::Delete)
                || i.consume_key(Modifiers::NONE, Key::Backspace),
            blackout: i.consume_key(Modifiers::NONE, Key::B),
            identify: i.consume_key(Modifiers::NONE, Key::I),
            grid: i.consume_key(Modifiers::NONE, Key::G),
            snap: i.consume_key(Modifiers::NONE, Key::S),
            lock: i.consume_key(Modifiers::NONE, Key::L),
            help: i.consume_key(Modifiers::NONE, Key::Questionmark)
                || i.consume_key(Modifiers::SHIFT, Key::Slash)
                || i.consume_key(Modifiers::NONE, Key::F1),
            reset_view: i.consume_key(Modifiers::NONE, Key::Num0),
            stage: i.consume_key(Modifiers::NONE, Key::T),
            gamma_down: i.consume_key(Modifiers::NONE, Key::OpenBracket),
            gamma_up: i.consume_key(Modifiers::NONE, Key::CloseBracket),
        });
        if k.undo && !typing {
            self.editor.undo();
        }
        if k.redo && !typing {
            self.editor.redo();
        }
        if k.save {
            self.save();
        }
        if k.save_as {
            self.save_as();
        }
        if k.open {
            self.open_dialog();
        }
        if k.new {
            self.request(Pending::New);
        }
        if k.show {
            if self.editor.show_mode {
                self.editor.exit_show_mode();
            } else {
                self.editor.enter_show_mode();
            }
        }
        if k.escape {
            if self.panels.help || self.panels.welcome {
                self.panels.help = false;
                self.panels.welcome = false;
            } else {
                self.editor.press_escape(now, Duration::from_secs(2));
            }
        }
        if typing {
            return;
        }
        if k.delete {
            self.editor.delete_selection();
        }
        if k.duplicate {
            if let Selection::Surface { id, .. } = self.editor.selection.clone() {
                self.editor.duplicate_surface(&id);
            }
        }
        if k.go {
            let clock = self.editor.sources.time();
            self.editor.cue_go_next(clock);
        }
        if k.blackout {
            let on = self.editor.project.blackout;
            self.editor
                .set_param("/show/blackout", lumen_core::params::ParamValue::Bool(!on));
            self.editor.commit();
        }
        if k.identify {
            self.editor.identify(now);
        }
        if k.grid && !self.editor.show_mode {
            self.canvas.show_grid = !self.canvas.show_grid;
        }
        if k.snap && !self.editor.show_mode {
            self.canvas.handles.snap = !self.canvas.handles.snap;
        }
        if k.lock && !self.editor.show_mode {
            if let Selection::Surface { id, .. } = self.editor.selection.clone() {
                let locked = self.editor.is_locked(&id);
                self.editor.set_locked(&id, !locked);
            }
        }
        if k.help {
            self.panels.help = !self.panels.help;
        }
        if k.stage && !self.editor.show_mode {
            self.canvas.stage = !self.canvas.stage;
        }
        if (k.gamma_down || k.gamma_up) && !self.editor.show_mode {
            if let Some(id) = self.canvas.output.clone() {
                let delta = if k.gamma_up { 0.05 } else { -0.05 };
                self.editor.adjust_blend_gamma(&id, delta);
            }
        }
        if k.reset_view {
            self.canvas.reset_view();
        }
    }

    /// Save, or Save As when the project has no file yet.
    pub fn save(&mut self) {
        if self.editor.path.is_some() {
            match self.editor.save() {
                Ok(()) => self.note_recent(),
                Err(e) => self.panels.error = Some(e),
            }
        } else {
            self.save_as();
        }
    }

    /// Save under a new name through the file dialog.
    pub fn save_as(&mut self) {
        let suggested = self.editor.path.clone();
        if let Some(path) = self.panels.dialogs.save_project(suggested.as_deref()) {
            match self.editor.save_as(&path) {
                Ok(()) => self.note_recent(),
                Err(e) => self.panels.error = Some(e),
            }
        }
    }

    /// Open through the file dialog.
    pub fn open_dialog(&mut self) {
        if let Some(path) = self.panels.dialogs.pick_project() {
            self.request(Pending::Open(path));
        }
    }

    /// Add media files through the file dialog.
    pub fn add_media_dialog(&mut self) {
        let files = self.panels.dialogs.pick_media();
        if !files.is_empty() {
            self.editor.add_files(&files);
        }
    }
}

/// Keys pressed this frame.
#[derive(Debug, Default, Clone, Copy)]
#[allow(clippy::struct_excessive_bools)]
struct Shortcuts {
    undo: bool,
    redo: bool,
    save: bool,
    save_as: bool,
    open: bool,
    new: bool,
    duplicate: bool,
    show: bool,
    escape: bool,
    go: bool,
    delete: bool,
    blackout: bool,
    identify: bool,
    grid: bool,
    snap: bool,
    lock: bool,
    help: bool,
    reset_view: bool,
    stage: bool,
    gamma_down: bool,
    gamma_up: bool,
}

impl LumenApp {
    /// Advances the clock, the sources and every controller. Returns the show clock.
    fn tick(&mut self, ctx: &egui::Context, now: Instant) -> f64 {
        self.frame_times.push_back(now);
        while self.frame_times.len() > 60 {
            self.frame_times.pop_front();
        }
        self.editor
            .sources
            .set_time(now.duration_since(self.started).as_secs_f64());
        self.editor.sources.sync(&self.editor.project);
        self.editor.sources.poll();
        let project_sources: Vec<String> = self
            .editor
            .project
            .sources
            .iter()
            .map(|s| s.id.clone())
            .collect();
        self.panels
            .thumbs
            .retain(|id| project_sources.iter().any(|s| s == id));
        let clock = self.editor.sources.time();
        if self.editor.poll_osc() > 0 {
            ctx.request_repaint();
        }
        self.editor.poll_midi(clock);
        self.editor.poll_remote(clock);
        if self.editor.tick_cues(clock) > 0 || self.editor.cues.is_fading() {
            ctx.request_repaint();
        }
        clock
    }

    /// One frame of the whole application: input, panels, canvas, output windows.
    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.heartbeat.beat();
        let now = Instant::now();
        let clock = self.tick(&ctx, now);
        self.editor.maybe_autosave(now);
        self.handle_close_request(&ctx);
        self.handle_dropped_files(&ctx);
        self.handle_shortcuts(&ctx);
        let title = self.editor.title();
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        if self.editor.show_mode {
            let fps = self.fps();
            panels::show_mode_screen(ui, &self.editor, fps);
        } else {
            let mut actions = panels::Actions::default();
            panels::top_bar(
                ui,
                &mut self.editor,
                &mut self.panels,
                &mut self.canvas,
                &mut actions,
            );
            panels::status_bar(ui, &self.editor, &self.canvas, self.fps(), now);
            panels::cue_panel(ui, &mut self.editor, &mut self.panels, clock);
            panels::left_panel(ui, &mut self.editor, &mut self.panels, &mut actions);
            panels::inspector(
                ui,
                &mut self.editor,
                &self.monitors,
                &mut self.panels,
                &mut actions,
            );
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(crate::theme::CANVAS_BG))
                .show(ui, |ui| {
                    if self.canvas.stage {
                        stage::show(
                            ui,
                            &mut self.editor,
                            &mut self.canvas,
                            &self.monitors,
                            self.surface_format,
                        );
                    } else {
                        canvas::show(ui, &mut self.editor, &mut self.canvas, self.surface_format);
                    }
                });
            panels::dialogs(
                &ctx,
                &mut self.editor,
                &mut self.panels,
                &self.monitors,
                &self.settings,
                &mut actions,
            );
            self.run_actions(actions, &ctx);
            self.drop_hover = ctx.input(|i| !i.raw.hovered_files.is_empty());
            if self.drop_hover {
                panels::drop_overlay(&ctx);
            }
        }
        outputs::show_output_windows(
            &ctx,
            &mut self.editor,
            &self.monitors,
            self.surface_format,
            &mut self.wall,
            self.canvas.handles.snap,
            if self.canvas.show_grid {
                self.canvas.grid_divisions
            } else {
                0
            },
        );
        self.persist_settings();
        if self.close_confirmed {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Keep live sources, identify flashes and the show-mode status fresh without
        // spinning when idle.
        if !self.editor.sources.live_ids().is_empty()
            || self.editor.show_mode
            || self.editor.identify_until.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    /// Runs the actions the panels asked for (they cannot borrow the app themselves).
    fn run_actions(&mut self, actions: panels::Actions, ctx: &egui::Context) {
        let _ = ctx;
        if actions.save {
            self.save();
        }
        if actions.save_as {
            self.save_as();
        }
        if actions.open {
            self.open_dialog();
        }
        if actions.new {
            self.request(Pending::New);
        }
        if let Some(path) = actions.open_path {
            self.request(Pending::Open(path));
        }
        if actions.add_media {
            self.add_media_dialog();
        }
        if actions.add_folder {
            if let Some(dir) = self.panels.dialogs.pick_folder() {
                self.editor.add_files(&[dir]);
            }
        }
        if let Some(path) = actions.ffmpeg_path {
            if path != self.settings.ffmpeg_path {
                self.settings.ffmpeg_path.clone_from(&path);
                self.settings.save();
                apply_ffmpeg_path(&path);
                self.panels.ffmpeg_ok = lumen_sources::video::Tools::from_env().available();
                self.editor.reload_sources();
            }
        }
        if let Some(port) = actions.midi_port {
            let r = self
                .editor
                .set_midi_port((!port.is_empty()).then_some(port.as_str()));
            if let Err(e) = r {
                self.panels.error = Some(e);
            } else {
                self.panels.error = None;
                self.settings.midi_port = port;
                self.settings.save();
            }
        }
        if let Some(target) = actions.feedback {
            match self.editor.set_feedback(&target) {
                Ok(()) => {
                    self.panels.error = None;
                    self.settings.osc_feedback = target;
                    self.settings.save();
                }
                Err(e) => self.panels.error = Some(e),
            }
        }
        if let Some(port) = actions.remote_port {
            match self.editor.set_remote_port(port) {
                Ok(()) => {
                    self.panels.error = None;
                    self.settings.remote_port = port;
                    self.settings.save();
                }
                Err(e) => self.panels.error = Some(e),
            }
        }
        if actions.create_outputs {
            self.editor
                .create_outputs_for_monitors(&self.monitors, true);
            self.panels.welcome = false;
        }
        if let Some(proceed) = actions.resolve_pending {
            if let Some(action) = self.editor.resolve_pending(proceed) {
                self.perform(action);
            }
        }
        if actions.skip_welcome {
            self.settings.skip_welcome = true;
            self.settings.save();
            self.panels.welcome = false;
        }
    }
}

impl eframe::App for LumenApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }
}

/// Points the video decoder at a user-chosen ffmpeg binary (empty restores the PATH lookup).
pub fn apply_ffmpeg_path(path: &str) {
    if path.trim().is_empty() {
        std::env::remove_var("LUMEN_FFMPEG");
    } else {
        std::env::set_var("LUMEN_FFMPEG", path.trim());
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
    lumen_render::Scene::build_with(
        project,
        output,
        &editor.sources,
        lumen_render::scene::DEFAULT_MESH_DENSITY,
        &mut editor.mask_cache,
        !editor.show_mode,
    )
    .ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
    fn scene_for_ignores_bad_outputs_and_drops_overlays_in_show_mode() {
        let mut e = Editor::new(demo_project(), None);
        assert!(scene_for(&mut e, "left").is_some());
        assert!(scene_for(&mut e, "ghost").is_none());
        e.set_overlay("left", Some(lumen_core::project::Overlay::Grid));
        let n = scene_for(&mut e, "left").unwrap().draws.len();
        e.enter_show_mode();
        assert_eq!(scene_for(&mut e, "left").unwrap().draws.len(), n - 1);
    }
}
