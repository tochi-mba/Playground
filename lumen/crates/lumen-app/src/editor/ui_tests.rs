//! Headless tests of the real editor UI, driven through egui's accessibility tree.

#![allow(clippy::unwrap_used, clippy::float_cmp, clippy::too_many_lines)]

use super::canvas::to_screen;
use super::{Launch, LumenApp, MonitorInfo, Selection};
use crate::editor::Editor;
use eframe::egui::{self, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use lumen_core::geom::Vec2;
use lumen_core::project::demo_project;
use lumen_core::warp::WarpKind;

fn monitors() -> Vec<MonitorInfo> {
    vec![MonitorInfo {
        name: "DISPLAY1".into(),
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
        scale: 1.0,
    }]
}

fn app(editor: Editor) -> LumenApp {
    let launch = Launch {
        project: None,
        osc_port: 0,
        show: false,
        restart_on_stall: None,
    };
    LumenApp::new(editor, &launch, monitors(), wgpu::TextureFormat::Rgba8Unorm)
}

fn harness(editor: Editor) -> Harness<'static, LumenApp> {
    let mut h = Harness::builder()
        .with_size(egui::Vec2::new(1400.0, 900.0))
        .build_ui_state(|ui, app: &mut LumenApp| app.frame(ui), app(editor));
    settle(&mut h);
    h
}

/// Runs a few frames. Show mode and live sources repaint continuously, so a fixed number of
/// steps is used instead of waiting for the UI to settle.
fn settle(h: &mut Harness<'static, LumenApp>) {
    h.run_steps(3);
}

fn click(h: &mut Harness<'static, LumenApp>, label: &str) {
    h.get_by_label(label).click();
    settle(h);
}

#[test]
fn panels_render_and_selection_drives_the_inspector() {
    let mut h = harness(Editor::new(demo_project(), None));
    assert!(h.query_by_label("Outputs").is_some());
    assert!(h.query_by_label("Sources").is_some());
    assert!(
        h.query_by_label("Show").is_some(),
        "no selection shows the show inspector"
    );

    click(&mut h, "left");
    assert_eq!(h.state().editor.selection, Selection::Output("left".into()));
    assert!(h.query_by_label("Output left").is_some());

    click(&mut h, "logo");
    assert_eq!(
        h.state().editor.selection,
        Selection::Surface {
            id: "logo".into(),
            mask: None
        }
    );
    assert!(h.query_by_label("Surface logo").is_some());
    assert!(
        h.state().canvas.last_rect.is_some(),
        "canvas drew the logo's output"
    );
    assert_eq!(h.state().canvas.output.as_deref(), Some("right"));
}

#[test]
fn surface_inspector_buttons_edit_the_project() {
    let mut h = harness(Editor::new(demo_project(), None));
    click(&mut h, "wall-left");
    click(&mut h, "+ rectangle");
    assert_eq!(
        h.state()
            .editor
            .project
            .surface("wall-left")
            .unwrap()
            .masks
            .len(),
        1
    );
    assert_eq!(
        h.state().editor.selection,
        Selection::Surface {
            id: "wall-left".into(),
            mask: Some(0)
        }
    );
    click(&mut h, "Add point");
    assert_eq!(
        h.state().editor.project.surface("wall-left").unwrap().masks[0]
            .points
            .len(),
        5
    );
    click(&mut h, "hole (unchecked = window)");
    assert!(!h.state().editor.project.surface("wall-left").unwrap().masks[0].subtract);
    click(&mut h, "Delete mask");
    assert!(h
        .state()
        .editor
        .project
        .surface("wall-left")
        .unwrap()
        .masks
        .is_empty());

    click(&mut h, "Convert to Bezier mesh");
    assert!(matches!(
        h.state().editor.project.surface("wall-left").unwrap().warp,
        WarpKind::Bezier(_)
    ));
    click(&mut h, "Reset to flat mesh");
    click(&mut h, "Back to corner pin");
    assert_eq!(
        h.state().editor.project.surface("wall-left").unwrap().warp,
        WarpKind::default()
    );
    click(&mut h, "Reset warp");
    click(&mut h, "Visible");
    assert!(
        !h.state()
            .editor
            .project
            .surface("wall-left")
            .unwrap()
            .visible
    );
    click(&mut h, "Visible");
    assert!(
        h.state()
            .editor
            .project
            .surface("wall-left")
            .unwrap()
            .visible
    );

    // Blend mode and source combos (combo boxes expose their selection as a value).
    h.get_by_value("Normal").click();
    settle(&mut h);
    click(&mut h, "Screen");
    assert_eq!(
        h.state()
            .editor
            .project
            .surface("wall-left")
            .unwrap()
            .blend_mode,
        lumen_core::project::BlendMode::Screen
    );
    h.get_all_by_role_and_label(egui::accesskit::Role::ComboBox, "Source")
        .next()
        .unwrap()
        .click();
    settle(&mut h);
    // The popup entry and the source list both say "bars"; the popup entry is the last one.
    h.get_all_by_label("bars").last().unwrap().click();
    settle(&mut h);
    assert_eq!(
        h.state()
            .editor
            .project
            .surface("wall-left")
            .unwrap()
            .source,
        "bars"
    );

    click(&mut h, "Delete surface");
    assert!(h.state().editor.project.surface("wall-left").is_none());
    assert_eq!(h.state().editor.selection, Selection::None);
    assert!(h.state().editor.history.can_undo());
    click(&mut h, "Undo");
    assert!(h.state().editor.project.surface("wall-left").is_some());
    click(&mut h, "Redo");
    assert!(h.state().editor.project.surface("wall-left").is_none());
}

#[test]
fn output_inspector_places_and_opens_windows() {
    let mut h = harness(Editor::new(demo_project(), None));
    click(&mut h, "left");
    click(&mut h, "Open window");
    assert_eq!(h.state().editor.open_outputs, ["left"]);
    settle(&mut h);
    assert!(
        h.state().editor.window_positions.contains_key("left"),
        "embedded output viewport reported its rect"
    );
    click(&mut h, "Use current window position");
    assert!(h
        .state()
        .editor
        .project
        .output("left")
        .unwrap()
        .placement
        .is_some());
    click(&mut h, "Unplace (back to preview window)");
    assert!(h
        .state()
        .editor
        .project
        .output("left")
        .unwrap()
        .placement
        .is_none());
    click(&mut h, "Fill monitor DISPLAY1 (1920x1080 at 0,0)");
    let o = h.state().editor.project.output("left").unwrap().clone();
    assert_eq!(
        o.placement.as_ref().map(|p| p.monitor.as_str()),
        Some("DISPLAY1")
    );
    settle(&mut h);
    click(&mut h, "Close window");
    assert!(h.state().editor.open_outputs.is_empty());
    click(&mut h, "Enabled in show mode");
    assert!(!h.state().editor.project.output("left").unwrap().enabled);
    // Curve combo for the left edge.
    h.get_all_by_value("Smooth").next().unwrap().click();
    settle(&mut h);
    click(&mut h, "Cosine");
    assert_eq!(
        h.state()
            .editor
            .project
            .output("left")
            .unwrap()
            .blend
            .left
            .curve,
        lumen_core::blend::BlendCurve::Cosine
    );
    click(&mut h, "Delete output");
    assert!(h.state().editor.project.output("left").is_none());
    assert!(h
        .state()
        .editor
        .project
        .surfaces
        .iter()
        .all(|s| s.output != "left"));
}

#[test]
fn tree_adds_and_removes_things() {
    let mut h = harness(Editor::new(demo_project(), None));
    h.get_all_by_label("+ surface").next().unwrap().click();
    settle(&mut h);
    assert_eq!(h.state().editor.project.surfaces.len(), 4);
    click(&mut h, "+ output");
    assert_eq!(h.state().editor.project.outputs.len(), 3);
    // Source menu.
    click(&mut h, "+ source");
    if h.query_by_label("Pattern: Checker").is_some() {
        click(&mut h, "Pattern: Checker");
        assert!(h
            .state()
            .editor
            .project
            .sources
            .iter()
            .any(|s| s.id == "pattern"));
    }
    // Unused sources can be removed.
    h.state_mut()
        .editor
        .add_source(lumen_core::project::SourceKind::Solid { color: [1.0; 4] });
    settle(&mut h);
    let before = h.state().editor.project.sources.len();
    h.get_all_by_label("×").last().unwrap().click();
    settle(&mut h);
    assert_eq!(h.state().editor.project.sources.len(), before - 1);
    // Output window toggle in the tree.
    h.get_all_by_label("win").next().unwrap().click();
    settle(&mut h);
    assert_eq!(h.state().editor.open_outputs.len(), 1);
    // Visibility checkbox in the tree (first surface row).
    let boxes: Vec<_> = h.get_all_by_role(egui::accesskit::Role::CheckBox).collect();
    assert!(!boxes.is_empty());
}

#[test]
fn show_inspector_adds_sources_and_toggles_blackout() {
    let mut h = harness(Editor::new(demo_project(), None));
    h.state_mut().panels.image_path = "nope.png".into();
    click(&mut h, "Add image");
    assert!(h
        .state()
        .editor
        .project
        .sources
        .iter()
        .any(|s| s.id == "image"));
    assert!(h.state().editor.status.contains("failed"));
    click(&mut h, "Add Spout receiver");
    assert!(h
        .state()
        .editor
        .project
        .sources
        .iter()
        .any(|s| s.id == "spout"));
    h.get_all_by_label("Blackout").next().unwrap().click();
    settle(&mut h);
    assert!(h.state().editor.project.blackout);
}

#[test]
fn canvas_drag_moves_a_corner_and_keys_nudge_it() {
    let mut h = harness(Editor::new(demo_project(), None));
    click(&mut h, "wall-left");
    let rect = h.state().canvas.last_rect.unwrap();
    let corner = h
        .state()
        .editor
        .project
        .surface("wall-left")
        .unwrap()
        .warp
        .clone();
    let WarpKind::Quad(q) = corner else { panic!() };
    let start = to_screen(rect, q.corners[2]);
    let end = start + egui::Vec2::new(-40.0, -30.0);
    h.hover_at(start);
    settle(&mut h);
    h.drag_at(start);
    settle(&mut h);
    h.hover_at(start + egui::Vec2::new(-10.0, -10.0));
    settle(&mut h);
    h.hover_at(end);
    settle(&mut h);
    h.drop_at(end);
    settle(&mut h);
    let WarpKind::Quad(moved) = h
        .state()
        .editor
        .project
        .surface("wall-left")
        .unwrap()
        .warp
        .clone()
    else {
        panic!()
    };
    let want = super::canvas::to_norm(rect, end);
    assert!(
        moved.corners[2].distance(want) < 0.01,
        "{:?} vs {want:?}",
        moved.corners[2]
    );
    assert_eq!(
        h.state().editor.history.undo_len(),
        1,
        "one drag is one undo step"
    );

    // Click a handle to make it active, then nudge with the keyboard.
    let pos = to_screen(rect, moved.corners[2]);
    h.hover_at(pos);
    h.drag_at(pos);
    h.drop_at(pos);
    settle(&mut h);
    h.key_press(Key::ArrowRight);
    settle(&mut h);
    let WarpKind::Quad(nudged) = h
        .state()
        .editor
        .project
        .surface("wall-left")
        .unwrap()
        .warp
        .clone()
    else {
        panic!()
    };
    assert!(nudged.corners[2].x > moved.corners[2].x);

    // Shift-drag on empty canvas moves everything.
    let mid = to_screen(rect, Vec2::new(0.5, 0.5));
    h.event_modifiers(egui::Event::PointerMoved(mid), Modifiers::SHIFT);
    h.event_modifiers(
        egui::Event::PointerButton {
            pos: mid,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::SHIFT,
        },
        Modifiers::SHIFT,
    );
    settle(&mut h);
    h.event_modifiers(
        egui::Event::PointerMoved(mid + egui::Vec2::new(20.0, 0.0)),
        Modifiers::SHIFT,
    );
    settle(&mut h);
    h.event_modifiers(
        egui::Event::PointerButton {
            pos: mid + egui::Vec2::new(20.0, 0.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::SHIFT,
        },
        Modifiers::SHIFT,
    );
    settle(&mut h);
    let WarpKind::Quad(shifted) = h
        .state()
        .editor
        .project
        .surface("wall-left")
        .unwrap()
        .warp
        .clone()
    else {
        panic!()
    };
    assert!(shifted.corners[0].x > nudged.corners[0].x);

    // Masks get handles too.
    h.state_mut().editor.add_mask("wall-left");
    settle(&mut h);
    let m = h.state().editor.project.surface("wall-left").unwrap().masks[0].clone();
    let p0 = to_screen(rect, m.points[0]);
    h.hover_at(p0);
    h.drag_at(p0);
    settle(&mut h);
    h.hover_at(p0 + egui::Vec2::new(15.0, 15.0));
    settle(&mut h);
    h.drop_at(p0 + egui::Vec2::new(15.0, 15.0));
    settle(&mut h);
    let moved_mask = &h.state().editor.project.surface("wall-left").unwrap().masks[0];
    assert!(moved_mask.points[0].x > m.points[0].x);

    // Bezier handles draw and drag as well.
    h.state_mut().editor.selection = Selection::Surface {
        id: "wall-left".into(),
        mask: None,
    };
    h.state_mut().editor.set_warp(
        "wall-left",
        WarpKind::Bezier(lumen_core::warp::BezierMesh::flat(2, 2).unwrap()),
    );
    h.state_mut().editor.commit();
    settle(&mut h);
    let p = to_screen(rect, Vec2::new(0.5, 0.5));
    h.hover_at(p);
    h.drag_at(p);
    settle(&mut h);
    h.hover_at(p + egui::Vec2::new(5.0, 5.0));
    settle(&mut h);
    h.drop_at(p + egui::Vec2::new(5.0, 5.0));
    settle(&mut h);
    let WarpKind::Bezier(b) = h
        .state()
        .editor
        .project
        .surface("wall-left")
        .unwrap()
        .warp
        .clone()
    else {
        panic!()
    };
    assert!(b
        .control_points
        .iter()
        .any(|c| (c.x - 0.5).abs() > 1e-6 && (c.x - 0.5).abs() < 0.1));
}

#[test]
fn show_mode_screen_and_shortcuts() {
    let mut h = harness(Editor::new(demo_project(), None));
    click(&mut h, "Show mode (F10)");
    assert!(h.state().editor.show_mode);
    assert!(h.query_by_label("SHOW MODE").is_some());
    assert_eq!(
        h.state().editor.open_outputs.len(),
        2,
        "every enabled output window opened"
    );
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

    // Undo/redo/save/delete shortcuts.
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
    h.key_press(Key::Delete);
    settle(&mut h);
    assert!(h.state().editor.project.surface("logo").is_none());
    h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    settle(&mut h);
    assert!(
        h.state().panels.path_dialog.is_some(),
        "save without a path opens Save As"
    );
}

#[test]
fn file_dialogs_open_save_and_recover() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ui.lumen.toml");
    let mut h = harness(Editor::new(demo_project(), None));
    click(&mut h, "Save As…");
    h.state_mut().panels.path_text = path.display().to_string();
    click(&mut h, "Save file");
    assert!(path.exists());
    assert!(h.state().panels.path_dialog.is_none());
    assert_eq!(h.state().editor.path.as_deref(), Some(path.as_path()));

    click(&mut h, "Open…");
    h.state_mut().panels.path_text = dir.path().join("missing.toml").display().to_string();
    click(&mut h, "Open file");
    assert!(h.state().panels.error.is_some(), "error stays visible");
    assert!(h.state().panels.path_dialog.is_some());
    click(&mut h, "Cancel");
    assert!(h.state().panels.path_dialog.is_none());
    click(&mut h, "Open…");
    h.state_mut().panels.path_text = path.display().to_string();
    click(&mut h, "Open file");
    assert_eq!(h.state().editor.path.as_deref(), Some(path.as_path()));
    click(&mut h, "Save");
    assert!(!h.state().editor.dirty);

    // Recovery prompt.
    let auto = crate::autosave::autosave_path(&path);
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
    click(&mut h, "New");
    assert!(h.state().editor.path.is_none());
}

#[test]
fn starting_in_show_mode_and_osc_port_zero() {
    let launch = Launch {
        project: None,
        osc_port: 0,
        show: true,
        restart_on_stall: Some(60.0),
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
    assert!(h.state().fps() >= 0.0);
    h.run_steps(3);
    assert!(h.state().fps() > 0.0);
}

#[test]
fn empty_project_canvas_asks_for_an_output() {
    let h = harness(Editor::new(lumen_core::project::Project::default(), None));
    assert!(h
        .query_by_label("Add an output to start mapping.")
        .is_some());
    assert!(h.state().canvas.last_rect.is_none());
}
