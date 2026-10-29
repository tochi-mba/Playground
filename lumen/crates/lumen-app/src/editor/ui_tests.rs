//! Headless tests of the real editor UI, driven through egui's accessibility tree.

#![allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::too_many_lines,
    clippy::print_stderr
)]

use super::handles::Mapping;
use super::panels::FakeDialogs;
use super::{Launch, LumenApp, MonitorInfo, Pending, Selection};
use crate::editor::Editor;
use eframe::egui::{self, accesskit::Role, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use lumen_core::geom::Vec2;
use lumen_core::project::demo_project;
use lumen_core::warp::WarpKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn monitors() -> Vec<MonitorInfo> {
    vec![
        MonitorInfo {
            name: "DISPLAY1".into(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
        },
        MonitorInfo {
            name: "DISPLAY2".into(),
            x: 1920,
            y: 0,
            width: 1280,
            height: 720,
            scale: 1.0,
        },
    ]
}

fn config_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().unwrap());
    std::env::set_var("LUMEN_CONFIG_DIR", dir.path());
    dir.path().to_path_buf()
}

fn app(editor: Editor) -> LumenApp {
    config_dir();
    let launch = Launch {
        project: None,
        osc_port: 0,
        show: false,
        restart_on_stall: None,
    };
    let mut app = LumenApp::new(editor, &launch, monitors(), wgpu::TextureFormat::Rgba8Unorm);
    app.panels.dialogs = Box::new(FakeDialogs::default());
    app
}

fn harness(editor: Editor) -> Harness<'static, LumenApp> {
    let mut h = Harness::builder()
        .with_size(egui::Vec2::new(1400.0, 900.0))
        .build_ui_state(|ui, app: &mut LumenApp| app.frame(ui), app(editor));
    h.run_steps(3);
    h
}

/// Runs a few frames. Show mode and live sources repaint continuously, so a fixed number of
/// steps is used instead of waiting for the UI to settle.
fn settle(h: &mut Harness<'static, LumenApp>) {
    h.run_steps(3);
}

fn click(h: &mut Harness<'static, LumenApp>, label: &str) {
    // Inspectors scroll; bring the control into view before clicking it.
    h.get_by_label(label).scroll_to_me();
    settle(h);
    h.get_by_label(label).click();
    settle(h);
}

fn menu(h: &mut Harness<'static, LumenApp>, menu: &str, item: &str) {
    click(h, menu);
    click(h, item);
}

fn fake(h: &mut Harness<'static, LumenApp>, f: impl FnOnce(&mut FakeDialogs)) {
    let mut d = FakeDialogs::default();
    f(&mut d);
    h.state_mut().panels.dialogs = Box::new(d);
}

fn surface_warp(h: &Harness<'static, LumenApp>, id: &str) -> WarpKind {
    h.state().editor.project.surface(id).unwrap().warp.clone()
}

fn drag(h: &mut Harness<'static, LumenApp>, from: egui::Pos2, to: egui::Pos2) {
    h.hover_at(from);
    settle(h);
    h.drag_at(from);
    settle(h);
    h.hover_at(from + (to - from) * 0.5);
    settle(h);
    h.hover_at(to);
    settle(h);
    h.drop_at(to);
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
    assert!(h.state().canvas.last_rect.is_some());
    assert_eq!(h.state().canvas.output.as_deref(), Some("right"));
    // Shift-click extends the selection and the status bar says so.
    h.get_by_label("wall-right")
        .click_modifiers(Modifiers::SHIFT);
    settle(&mut h);
    assert_eq!(h.state().editor.selected_surfaces().len(), 2);
    assert!(h.query_by_label("2 surfaces selected").is_some());
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
    assert!(matches!(surface_warp(&h, "wall-left"), WarpKind::Bezier(_)));
    click(&mut h, "Reset to flat mesh");
    click(&mut h, "Back to corner pin");
    assert_eq!(surface_warp(&h, "wall-left"), WarpKind::default());
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
    h.get_all_by_role_and_label(Role::ComboBox, "Source")
        .next()
        .unwrap()
        .click();
    settle(&mut h);
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

    // Lock, duplicate, order, align.
    click(&mut h, "Locked");
    assert!(h.state().editor.is_locked("wall-left"));
    click(&mut h, "Locked");
    click(&mut h, "Duplicate");
    assert_eq!(
        h.state().editor.selection,
        Selection::Surface {
            id: "wall-left-copy".into(),
            mask: None
        }
    );
    click(&mut h, "Backward");
    click(&mut h, "Forward");
    h.state_mut().editor.set_warp(
        "wall-left-copy",
        WarpKind::Quad(lumen_core::warp::Quad {
            corners: [
                Vec2::new(0.1, 0.2),
                Vec2::new(0.9, 0.1),
                Vec2::new(0.8, 0.9),
                Vec2::new(0.2, 0.8),
            ],
        }),
    );
    h.state_mut().editor.commit();
    settle(&mut h);
    let before = surface_warp(&h, "wall-left-copy");
    menu(&mut h, "Align", "Flip horizontal");
    assert_ne!(surface_warp(&h, "wall-left-copy"), before);
    for item in [
        "Flip vertical",
        "Rotate 90°",
        "Centre",
        "Match source aspect",
        "Fit output",
    ] {
        menu(&mut h, "Align", item);
    }
    assert_eq!(surface_warp(&h, "wall-left-copy"), WarpKind::default());
    click(&mut h, "Reset warp");

    click(&mut h, "Delete surface");
    assert!(h.state().editor.project.surface("wall-left-copy").is_none());
    assert_eq!(h.state().editor.selection, Selection::None);
    click(&mut h, "Undo");
    assert!(h.state().editor.project.surface("wall-left-copy").is_some());
    click(&mut h, "Redo");
    assert!(h.state().editor.project.surface("wall-left-copy").is_none());
}

#[test]
fn inline_rename_through_the_inspector() {
    let mut h = harness(Editor::new(demo_project(), None));
    click(&mut h, "logo");
    let field = h.get_by_role_and_label(Role::TextInput, "Id");
    field.focus();
    field.type_text("-renamed");
    settle(&mut h);
    h.key_press(Key::Enter);
    settle(&mut h);
    assert!(
        h.state().editor.project.surface("logo-renamed").is_some(),
        "{:?}",
        h.state()
            .editor
            .project
            .surfaces
            .iter()
            .map(|s| &s.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        h.state().editor.selection,
        Selection::Surface {
            id: "logo-renamed".into(),
            mask: None
        }
    );
    // An invalid id is reported and nothing changes.
    let field = h.get_by_role_and_label(Role::TextInput, "Id");
    field.focus();
    field.type_text(" bad");
    settle(&mut h);
    h.key_press(Key::Enter);
    settle(&mut h);
    assert!(h.state().panels.error.is_some());
    assert!(h.query_by_label("Problem").is_some());
    click(&mut h, "Dismiss");
    assert!(h.state().panels.error.is_none());
}

#[test]
fn output_inspector_places_and_opens_windows() {
    let mut h = harness(Editor::new(demo_project(), None));
    click(&mut h, "left");
    click(&mut h, "Open window");
    assert_eq!(h.state().editor.open_outputs, ["left"]);
    settle(&mut h);
    assert!(h.state().editor.window_positions.contains_key("left"));
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
    click(&mut h, "Fill monitor DISPLAY2 (1280x720 at 1920,0)");
    let o = h.state().editor.project.output("left").unwrap().clone();
    assert_eq!(
        o.placement.as_ref().map(|p| p.monitor.as_str()),
        Some("DISPLAY2")
    );
    click(&mut h, "Identify on the projector");
    assert!(h.state().editor.identify_until.is_some());
    settle(&mut h);
    click(&mut h, "Close window");
    assert!(h.state().editor.open_outputs.is_empty());
    click(&mut h, "Enabled in show mode");
    assert!(!h.state().editor.project.output("left").unwrap().enabled);
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
}

#[test]
fn tree_adds_removes_and_offers_context_menus() {
    let mut h = harness(Editor::new(demo_project(), None));
    h.get_all_by_label("+ surface").next().unwrap().click();
    settle(&mut h);
    assert_eq!(h.state().editor.project.surfaces.len(), 4);
    click(&mut h, "+ output");
    assert_eq!(h.state().editor.project.outputs.len(), 3);
    click(&mut h, "+ source");
    click(&mut h, "Pattern: Checker");
    assert!(h
        .state()
        .editor
        .project
        .sources
        .iter()
        .any(|s| s.id == "pattern"));
    h.state_mut()
        .editor
        .add_source(lumen_core::project::SourceKind::Solid { color: [1.0; 4] });
    settle(&mut h);
    let before = h.state().editor.project.sources.len();
    h.get_all_by_label("×").last().unwrap().click();
    settle(&mut h);
    assert_eq!(h.state().editor.project.sources.len(), before - 1);
    h.get_all_by_label("win").next().unwrap().click();
    settle(&mut h);
    assert_eq!(h.state().editor.open_outputs.len(), 1);
    h.state_mut().editor.open_outputs.clear();
    settle(&mut h);

    // Surface context menu.
    h.get_by_label("logo").click_secondary();
    settle(&mut h);
    click(&mut h, "Duplicate  (Ctrl+D)");
    assert!(h.state().editor.project.surface("logo-copy").is_some());
    h.get_by_label("logo").click_secondary();
    settle(&mut h);
    click(&mut h, "Lock  (L)");
    assert!(h.state().editor.is_locked("logo"));
    h.get_by_label("logo [locked]").click_secondary();
    settle(&mut h);
    click(&mut h, "Unlock  (L)");
    assert!(!h.state().editor.is_locked("logo"));
    h.get_by_label("logo").click_secondary();
    settle(&mut h);
    click(&mut h, "Send to back");
    assert_eq!(
        h.state().editor.project.surfaces_for_output("right")[0].id,
        "logo"
    );
    h.get_by_label("logo").click_secondary();
    settle(&mut h);
    click(&mut h, "Bring to front");
    assert_eq!(
        h.state()
            .editor
            .project
            .surfaces_for_output("right")
            .last()
            .unwrap()
            .id,
        "logo"
    );
    h.get_by_label("logo").click_secondary();
    settle(&mut h);
    click(&mut h, "Reset warp to full frame");
    assert_eq!(surface_warp(&h, "logo"), WarpKind::default());
    h.get_by_label("logo").click_secondary();
    settle(&mut h);
    click(&mut h, "Delete");
    assert!(h.state().editor.project.surface("logo").is_none());

    // Output context menu.
    h.get_by_label("right").click_secondary();
    settle(&mut h);
    click(&mut h, "Add surface");
    h.get_by_label("right").click_secondary();
    settle(&mut h);
    click(&mut h, "Identify outputs");
    assert!(h.state().editor.identify_until.is_some());
    h.get_by_label("right").click_secondary();
    settle(&mut h);
    click(&mut h, "Delete output");
    assert!(h.state().editor.project.output("right").is_none());
}

#[test]
fn show_inspector_and_media_dialog() {
    let mut h = harness(Editor::new(demo_project(), None));
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
    let dir = tempfile::tempdir().unwrap();
    let img = dir.path().join("pic.png");
    image::RgbaImage::from_pixel(2, 2, image::Rgba([9, 9, 9, 255]))
        .save(&img)
        .unwrap();
    fake(&mut h, |d| d.media = vec![img.clone()]);
    click(&mut h, "Add media…");
    assert!(h
        .state()
        .editor
        .project
        .sources
        .iter()
        .any(|s| s.id == "pic"));
}

#[test]
fn canvas_drag_moves_a_corner_and_keys_nudge_it() {
    let mut h = harness(Editor::new(demo_project(), None));
    click(&mut h, "wall-left");
    h.state_mut().canvas.handles.snap = false;
    let rect = h.state().canvas.last_rect.unwrap();
    let map = Mapping { rect };
    let WarpKind::Quad(q) = surface_warp(&h, "wall-left") else {
        panic!()
    };
    let start = map.to_screen(q.corners[2]);
    let end = start + egui::Vec2::new(-40.0, -30.0);
    drag(&mut h, start, end);
    let WarpKind::Quad(moved) = surface_warp(&h, "wall-left") else {
        panic!()
    };
    let want = map.to_norm(end);
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

    // Click a handle to make it active, then nudge with the keyboard (plain, Shift, Alt).
    let pos = map.to_screen(moved.corners[2]);
    h.hover_at(pos);
    h.drag_at(pos);
    h.drop_at(pos);
    settle(&mut h);
    assert!(h.state().canvas.readout.is_some());
    h.key_press(Key::ArrowRight);
    settle(&mut h);
    let WarpKind::Quad(n1) = surface_warp(&h, "wall-left") else {
        panic!()
    };
    assert!(n1.corners[2].x > moved.corners[2].x);
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowRight);
    settle(&mut h);
    let WarpKind::Quad(n2) = surface_warp(&h, "wall-left") else {
        panic!()
    };
    assert!((n2.corners[2].x - n1.corners[2].x) > 5.0 * (n1.corners[2].x - moved.corners[2].x));
    h.key_press_modifiers(Modifiers::ALT, Key::ArrowDown);
    settle(&mut h);
    let WarpKind::Quad(n3) = surface_warp(&h, "wall-left") else {
        panic!()
    };
    assert!(
        n3.corners[2].y > n2.corners[2].y
            && (n3.corners[2].y - n2.corners[2].y) < (n1.corners[2].x - moved.corners[2].x)
    );

    // Shift-drag on empty canvas moves everything.
    let mid = map.to_screen(Vec2::new(0.5, 0.5));
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
    let WarpKind::Quad(shifted) = surface_warp(&h, "wall-left") else {
        panic!()
    };
    assert!(shifted.corners[0].x > n3.corners[0].x);

    // Masks get handles too.
    h.state_mut().editor.add_mask("wall-left");
    settle(&mut h);
    let m = h.state().editor.project.surface("wall-left").unwrap().masks[0].clone();
    let p0 = map.to_screen(m.points[0]);
    drag(&mut h, p0, p0 + egui::Vec2::new(15.0, 15.0));
    assert!(
        h.state().editor.project.surface("wall-left").unwrap().masks[0].points[0].x > m.points[0].x
    );

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
    let p = map.to_screen(Vec2::new(0.5, 0.5));
    drag(&mut h, p, p + egui::Vec2::new(5.0, 5.0));
    let WarpKind::Bezier(b) = surface_warp(&h, "wall-left") else {
        panic!()
    };
    assert!(b
        .control_points
        .iter()
        .any(|c| (c.x - 0.5).abs() > 1e-6 && (c.x - 0.5).abs() < 0.1));
}

#[test]
fn snapping_locking_and_click_to_select_on_the_canvas() {
    let mut h = harness(Editor::new(demo_project(), None));
    click(&mut h, "wall-right");
    let rect = h.state().canvas.last_rect.unwrap();
    let map = Mapping { rect };
    // Snap is on: dragging a corner close to the output edge lands exactly on it.
    h.state_mut().canvas.handles.snap = true;
    h.state_mut().editor.set_warp(
        "wall-right",
        WarpKind::Quad(lumen_core::warp::Quad {
            corners: [
                Vec2::new(0.1, 0.1),
                Vec2::new(0.9, 0.1),
                Vec2::new(0.9, 0.9),
                Vec2::new(0.1, 0.9),
            ],
        }),
    );
    h.state_mut().editor.commit();
    settle(&mut h);
    let start = map.to_screen(Vec2::new(0.1, 0.1));
    let target = map.to_screen(Vec2::new(0.005, 0.1));
    drag(&mut h, start, target);
    let WarpKind::Quad(q) = surface_warp(&h, "wall-right") else {
        panic!()
    };
    assert_eq!(q.corners[0].x, 0.0, "snapped to the left edge");

    // A locked surface ignores drags but still gets selected by clicking inside it.
    h.state_mut().editor.set_locked("wall-right", true);
    settle(&mut h);
    let before = surface_warp(&h, "wall-right");
    let c = map.to_screen(Vec2::new(0.9, 0.1));
    drag(&mut h, c, c + egui::Vec2::new(-30.0, 30.0));
    assert_eq!(surface_warp(&h, "wall-right"), before);
    // Click on the logo (drawn above wall-right) selects it.
    let logo_centre = map.to_screen(surface_warp(&h, "logo").centre());
    h.hover_at(logo_centre);
    h.drag_at(logo_centre);
    h.drop_at(logo_centre);
    settle(&mut h);
    assert_eq!(
        h.state().editor.selection,
        Selection::Surface {
            id: "logo".into(),
            mask: None
        }
    );
    // Grid toggles draw without breaking anything; zoom changes the rect.
    h.key_press(Key::G);
    settle(&mut h);
    assert!(h.state().canvas.show_grid);
    h.hover_at(rect.center());
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::Vec2::new(0.0, 120.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: Modifiers::NONE,
    });
    settle(&mut h);
    assert!(h.state().canvas.zoom > 1.0, "{}", h.state().canvas.zoom);
    assert!(h.state().canvas.last_rect.unwrap().width() > rect.width());
    h.key_press(Key::Num0);
    settle(&mut h);
    assert_eq!(h.state().canvas.zoom, 1.0);
    h.key_press(Key::S);
    settle(&mut h);
    assert!(!h.state().canvas.handles.snap);
}

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
