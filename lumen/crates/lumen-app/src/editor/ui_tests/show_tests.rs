//! Show mode, files, welcome, drops, sources and media.

#![allow(clippy::unwrap_used, clippy::float_cmp, clippy::too_many_lines)]

use super::super::{Launch, LumenApp, Pending, Selection};
use super::*;
use crate::editor::Editor;
use eframe::egui::{self, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use lumen_core::project::demo_project;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn show_mode_screen_and_shortcuts() {
    let mut h = harness(Editor::new(demo_project(), None));
    click(&mut h, "Show mode");
    assert!(h.state().editor.show_mode);
    assert!(h.query_by_label("SHOW MODE").is_some());
    assert_eq!(h.state().editor.open_outputs.len(), 2);
    h.key_press(Key::Escape);
    settle(&mut h);
    assert!(h.state().editor.show_mode);
    h.key_press(Key::Escape);
    settle(&mut h);
    assert!(!h.state().editor.show_mode);
    h.key_press(Key::F10);
    settle(&mut h);
    assert!(h.state().editor.show_mode);
    h.key_press(Key::F10);
    settle(&mut h);
    assert!(!h.state().editor.show_mode);
    // Show mode opened both output windows; close them so the embedded windows do not
    // cover the tree in this headless harness.
    h.state_mut().editor.open_outputs.clear();
    settle(&mut h);

    click(&mut h, "logo");
    h.state_mut().editor.set_param(
        "/surface/logo/opacity",
        lumen_core::params::ParamValue::Number(0.1),
    );
    h.state_mut().editor.commit();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut h);
    assert_eq!(
        h.state().editor.project.surface("logo").unwrap().opacity,
        0.8
    );
    h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    settle(&mut h);
    assert_eq!(
        h.state().editor.project.surface("logo").unwrap().opacity,
        0.1
    );
    h.key_press_modifiers(Modifiers::COMMAND, Key::D);
    settle(&mut h);
    assert!(h.state().editor.project.surface("logo-copy").is_some());
    h.key_press(Key::L);
    settle(&mut h);
    assert!(h.state().editor.is_locked("logo-copy"));
    h.key_press(Key::Delete);
    settle(&mut h);
    assert!(h.state().editor.project.surface("logo-copy").is_none());
    h.key_press(Key::B);
    settle(&mut h);
    assert!(h.state().editor.project.blackout);
    h.key_press(Key::I);
    settle(&mut h);
    assert!(h.state().editor.identify_until.is_some());
    h.key_press(Key::Questionmark);
    settle(&mut h);
    assert!(h.state().panels.help);
    assert!(h.query_by_label("Keyboard shortcuts").is_some());
    h.key_press(Key::Escape);
    settle(&mut h);
    assert!(!h.state().panels.help);
}

#[test]
fn file_flows_through_dialogs_and_unsaved_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ui.lumen.toml");
    let mut h = harness(Editor::new(demo_project(), None));
    // Ctrl+S on an untitled project goes through Save As with the scripted dialog.
    fake(&mut h, |d| d.save = Some(path.clone()));
    h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    settle(&mut h);
    assert!(path.exists());
    assert_eq!(h.state().editor.path.as_deref(), Some(path.as_path()));
    assert!(h.state().settings.recent.contains(&path));
    // A cancelled dialog does nothing.
    fake(&mut h, |_| {});
    menu(&mut h, "File", "Save As…  (Ctrl+Shift+S)");
    assert!(h.state().panels.error.is_none());
    // Open a missing file: error dialog.
    fake(&mut h, |d| {
        d.project = Some(dir.path().join("missing.toml"));
    });
    menu(&mut h, "File", "Open…  (Ctrl+O)");
    assert!(h.query_by_label("Problem").is_some());
    click(&mut h, "Dismiss");
    // Dirty + New: prompt, cancel keeps everything.
    h.state_mut()
        .editor
        .set_param("/show/master", lumen_core::params::ParamValue::Number(0.3));
    h.state_mut().editor.commit();
    menu(&mut h, "File", "New  (Ctrl+N)");
    assert!(h.query_by_label("Unsaved changes").is_some());
    click(&mut h, "Cancel");
    assert_eq!(h.state().editor.project.master, 0.3);
    // Prompt, discard: new project.
    h.key_press_modifiers(Modifiers::COMMAND, Key::N);
    settle(&mut h);
    click(&mut h, "Discard changes");
    assert!(h.state().editor.path.is_none());
    assert!(
        h.state().panels.welcome,
        "a new project shows the welcome screen"
    );
    click(&mut h, "Close");
    // Open the saved file through the dialog; then dirty + open another with "Save and continue".
    fake(&mut h, |d| d.project = Some(path.clone()));
    h.key_press_modifiers(Modifiers::COMMAND, Key::O);
    settle(&mut h);
    assert_eq!(h.state().editor.path.as_deref(), Some(path.as_path()));
    h.state_mut()
        .editor
        .set_param("/show/master", lumen_core::params::ParamValue::Number(0.6));
    h.state_mut().editor.commit();
    let other = dir.path().join("other.lumen.toml");
    demo_project().save(&other).unwrap();
    fake(&mut h, |d| d.project = Some(other.clone()));
    h.key_press_modifiers(Modifiers::COMMAND, Key::O);
    settle(&mut h);
    click(&mut h, "Save and continue");
    assert_eq!(h.state().editor.path.as_deref(), Some(other.as_path()));
    assert_eq!(
        lumen_core::project::Project::load(&path).unwrap().master,
        0.6,
        "saved before switching"
    );
    // Recovery prompt.
    let auto = crate::autosave::autosave_path(&other);
    let mut changed = demo_project();
    changed.master = 0.4;
    changed.save(&auto).unwrap();
    h.state_mut().editor.recovery = Some(auto.clone());
    settle(&mut h);
    click(&mut h, "Recover");
    assert_eq!(h.state().editor.project.master, 0.4);
    h.state_mut().editor.recovery = Some(auto.clone());
    settle(&mut h);
    click(&mut h, "Discard");
    assert!(!auto.exists());
    // Close with unsaved changes: cancelled, prompt, confirm.
    h.state_mut().editor.dirty = true;
    h.state_mut().request(Pending::Close);
    settle(&mut h);
    assert!(h.query_by_label("Unsaved changes").is_some());
    click(&mut h, "Discard changes");
    assert!(h.state().close_confirmed);
}

#[test]
fn welcome_screen_creates_outputs_and_remembers_recent() {
    let dir = tempfile::tempdir().unwrap();
    let recent = dir.path().join("old.lumen.toml");
    demo_project().save(&recent).unwrap();
    let mut h = harness(Editor::new(lumen_core::project::Project::default(), None));
    h.state_mut().settings.push_recent(&recent);
    h.state_mut().panels.welcome = true;
    settle(&mut h);
    assert!(h.query_by_label("Welcome to lumen").is_some());
    click(&mut h, "Create 1 output(s) with a test grid");
    assert_eq!(h.state().editor.project.outputs.len(), 1);
    assert_eq!(
        h.state().editor.project.outputs[0]
            .placement
            .as_ref()
            .unwrap()
            .monitor,
        "DISPLAY2"
    );
    assert!(!h.state().panels.welcome);
    h.state_mut().panels.welcome = true;
    settle(&mut h);
    click(&mut h, "old.lumen.toml");
    assert!(
        h.query_by_label("Unsaved changes").is_some(),
        "the fresh outputs are unsaved"
    );
    click(&mut h, "Discard changes");
    assert_eq!(h.state().editor.path.as_deref(), Some(recent.as_path()));
    h.state_mut().panels.welcome = true;
    settle(&mut h);
    click(&mut h, "Don't show again");
    assert!(h.state().settings.skip_welcome);
    assert!(!h.state().panels.welcome);
    menu(&mut h, "File", "Welcome screen");
    assert!(h.state().panels.welcome);
    click(&mut h, "Start from the demo");
    assert!(!h.state().panels.welcome);
}

#[derive(Debug)]
struct TestDrop(PathBuf);

impl egui::DroppedFile for TestDrop {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

#[test]
fn dropped_files_become_sources_and_hover_shows_an_overlay() {
    let dir = tempfile::tempdir().unwrap();
    let img = dir.path().join("drop.png");
    image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 1, 1, 255]))
        .save(&img)
        .unwrap();
    let mut h = harness(Editor::new(demo_project(), None));
    click(&mut h, "right");
    h.input_mut().hovered_files.push(egui::HoveredFile {
        path: Some(img.clone()),
        mime: String::new(),
    });
    h.step();
    assert!(h.state().drop_hover);
    h.input_mut().hovered_files.clear();
    h.input_mut().dropped_files.push(Arc::new(TestDrop(img)));
    h.step();
    settle(&mut h);
    assert!(h.state().editor.project.source("drop").is_some());
    assert_eq!(
        h.state()
            .editor
            .project
            .surface("drop-surface")
            .unwrap()
            .output,
        "right"
    );
}

#[test]
fn view_menu_toggles_and_output_windows_carry_handles() {
    let mut h = harness(Editor::new(demo_project(), None));
    menu(&mut h, "View", "Grid  (G)");
    assert!(h.state().canvas.show_grid);
    menu(&mut h, "View", "Handles on output windows");
    assert!(!h.state().editor.edit_on_wall);
    menu(&mut h, "View", "Handles on output windows");
    menu(&mut h, "View", "Fit view  (0)");
    menu(&mut h, "View", "Keyboard shortcuts  (?)");
    assert!(h.state().panels.help);
    click(&mut h, "Close");
    // Settings persisted the toggles.
    let saved = crate::settings::Settings::load();
    assert!(saved.show_grid);
    click(&mut h, "wall-left");
    menu(&mut h, "Edit", "Duplicate surface  (Ctrl+D)");
    assert!(h.state().editor.project.surface("wall-left-copy").is_some());
    menu(&mut h, "Edit", "Delete  (Del)");
    assert!(h.state().editor.project.surface("wall-left-copy").is_none());
    menu(&mut h, "Edit", "Undo  (Ctrl+Z)");
    menu(&mut h, "Edit", "Redo  (Ctrl+Y)");
    click(&mut h, "Identify");
    h.get_all_by_label("Blackout").next().unwrap().click();
    settle(&mut h);
    assert!(h.state().editor.project.blackout);
    // An open output window with a selected surface draws the on-wall handles.
    click(&mut h, "wall-left");
    h.state_mut().editor.toggle_output_window("left");
    h.state_mut().editor.identify(std::time::Instant::now());
    settle(&mut h);
    assert!(h.state().wall.contains_key("left"));
}

#[test]
fn starting_in_show_mode_and_osc_port_zero() {
    config_dir();
    let launch = Launch {
        project: None,
        osc_port: 0,
        show: true,
        restart_on_stall: Some(60.0),
        remote_port: None,
    };
    let app = LumenApp::new(
        Editor::new(demo_project(), None),
        &launch,
        monitors(),
        wgpu::TextureFormat::Rgba8Unorm,
    );
    let mut h = Harness::builder()
        .with_size(egui::Vec2::new(800.0, 600.0))
        .build_ui_state(|ui, app: &mut LumenApp| app.frame(ui), app);
    settle(&mut h);
    assert!(h.state().editor.show_mode);
    assert!(h.state().editor.osc.is_none());
    h.run_steps(3);
    assert!(h.state().fps() > 0.0);
}

#[test]
fn empty_project_canvas_asks_for_an_output() {
    let h = harness(Editor::new(lumen_core::project::Project::default(), None));
    assert!(h
        .query_by_label("Add an output to start mapping (or drop an image here).")
        .is_some());
    assert!(h.state().canvas.last_rect.is_none());
}

#[test]
fn source_inspector_transport_and_generator_controls() {
    let dir = tempfile::tempdir().unwrap();
    let seq = dir.path().join("frames");
    std::fs::create_dir(&seq).unwrap();
    for i in 0..3 {
        image::RgbaImage::from_pixel(4, 2, image::Rgba([i * 40, 0, 0, 255]))
            .save(seq.join(format!("f{i}.png")))
            .unwrap();
    }
    let mut h = harness(Editor::new(demo_project(), None));
    // Folder dialog adds a sequence source and a surface on the first output.
    fake(&mut h, |d| d.folder = Some(seq.clone()));
    click(&mut h, "+ source");
    click(&mut h, "Image sequence folder…");
    assert!(h.state().editor.project.source("frames").is_some());
    assert!(
        !h.state().panels.thumbs.is_empty(),
        "thumbnails are built for sources with frames"
    );
    click(&mut h, "frames");
    assert_eq!(
        h.state().editor.selection,
        Selection::Source("frames".into())
    );
    assert!(h.query_by_label("Source frames").is_some());
    click(&mut h, "Pause");
    assert!(
        h.state()
            .editor
            .project
            .source("frames")
            .unwrap()
            .kind
            .playback()
            .unwrap()
            .paused
    );
    click(&mut h, "Play");
    assert!(
        !h.state()
            .editor
            .project
            .source("frames")
            .unwrap()
            .kind
            .playback()
            .unwrap()
            .paused
    );
    click(&mut h, "Loop");
    assert!(
        !h.state()
            .editor
            .project
            .source("frames")
            .unwrap()
            .kind
            .playback()
            .unwrap()
            .looping
    );
    click(&mut h, "Restart");
    assert_eq!(
        h.state()
            .editor
            .project
            .source("frames")
            .unwrap()
            .kind
            .playback()
            .unwrap()
            .restart_token,
        1
    );
    assert!(h.query_by_label("Delete source").is_some());

    // Generators from the menu, with the pattern combo.
    click(&mut h, "+ source");
    click(&mut h, "Generator: Plasma");
    assert!(h.state().editor.project.source("generator").is_some());
    click(&mut h, "generator");
    h.get_by_value("Plasma").click();
    settle(&mut h);
    click(&mut h, "Radar");
    assert!(matches!(
        h.state().editor.project.source("generator").unwrap().kind,
        lumen_core::project::SourceKind::Generator {
            pattern: lumen_core::project::GeneratorKind::Radar,
            ..
        }
    ));
    assert!(h
        .state()
        .editor
        .sources
        .live_ids()
        .contains(&"generator".to_string()));
    // Unused generator can be deleted from its inspector.
    click(&mut h, "Delete source");
    assert!(h.state().editor.project.source("generator").is_none());
    assert_eq!(h.state().editor.selection, Selection::None);
    // A used source cannot be deleted (status says why).
    h.state_mut().editor.selection = Selection::Source("grid".into());
    h.state_mut().editor.delete_selection();
    assert!(h.state().editor.project.source("grid").is_some());
    assert!(h.state().editor.status.contains("still used"));
}

#[test]
fn crop_preview_drags_and_resets() {
    let mut h = harness(Editor::new(demo_project(), None));
    click(&mut h, "wall-right");
    // The crop box of wall-right is full frame; find the preview by its caption and drag the
    // bottom-right handle inward.
    let caption = h.get_by_label_contains("double-click to reset");
    let caption_rect = caption.rect();
    // The preview sits just above the caption and is 280 px wide.
    let preview = egui::Rect::from_min_size(
        egui::Pos2::new(
            caption_rect.min.x,
            caption_rect.min.y - 280.0 * 1080.0 / 1920.0 - 4.0,
        ),
        egui::Vec2::new(280.0, 280.0 * 1080.0 / 1920.0),
    );
    let br = preview.max;
    drag(
        &mut h,
        br - egui::Vec2::new(2.0, 2.0),
        br - egui::Vec2::new(80.0, 40.0),
    );
    let initial = demo_project().surface("wall-right").unwrap().crop;
    let crop = h.state().editor.project.surface("wall-right").unwrap().crop;
    assert!(crop.w < initial.w || crop.h < initial.h, "{crop:?}");
    assert_eq!(
        h.state().editor.history.undo_len(),
        1,
        "one drag, one undo step"
    );
    // Move the box.
    let centre = preview.center();
    drag(&mut h, centre, centre + egui::Vec2::new(10.0, 0.0));
    let moved = h.state().editor.project.surface("wall-right").unwrap().crop;
    assert!(moved.x > crop.x, "{moved:?} vs {crop:?}");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut h);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut h);
    assert_eq!(
        h.state().editor.project.surface("wall-right").unwrap().crop,
        initial
    );
}

#[test]
fn media_dialog_accepts_video_when_ffmpeg_exists() {
    let tools = lumen_sources::video::Tools::from_env();
    let mut h = harness(Editor::new(demo_project(), None));
    assert_eq!(h.state().panels.ffmpeg_ok, tools.available());
    if !tools.available() {
        eprintln!("skipping video part: no ffmpeg");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("clip.mp4");
    let ok = std::process::Command::new(&tools.ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=32x18:rate=5:duration=1",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&clip)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    click(&mut h, "right");
    fake(&mut h, |d| d.media = vec![clip.clone()]);
    menu(&mut h, "File", "Add media…");
    assert!(matches!(
        h.state().editor.project.source("clip").unwrap().kind,
        lumen_core::project::SourceKind::Video { .. }
    ));
    assert_eq!(
        h.state()
            .editor
            .project
            .surface("clip-surface")
            .unwrap()
            .output,
        "right"
    );
    // Frames arrive over the next steps.
    for _ in 0..100 {
        if h.state().editor.sources.frame("clip").is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.step();
    }
    assert!(
        h.state().editor.sources.frame("clip").is_some(),
        "the video produced a frame"
    );
    click(&mut h, "clip");
    assert!(h.query_by_label("Pause").is_some());
    assert!(
        h.query_by_label_contains("ffmpeg found").is_none(),
        "the show panel is not visible now"
    );
}
