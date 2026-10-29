//! Headless tests of the real editor UI, driven through egui's accessibility tree.

#![allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::too_many_lines,
    clippy::print_stderr
)]

use super::handles::Mapping;
use super::panels::FakeDialogs;
use super::{Launch, LumenApp, MonitorInfo, Selection};
use crate::editor::Editor;
use eframe::egui::{self, accesskit::Role, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use lumen_core::geom::Vec2;
use lumen_core::project::demo_project;
use lumen_core::warp::WarpKind;
use std::path::PathBuf;

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
        remote_port: None,
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

/// A primary click at a screen position (press, release, settle).
fn click_at(h: &mut Harness<'static, LumenApp>, pos: egui::Pos2) {
    h.hover_at(pos);
    settle(h);
    h.drag_at(pos);
    settle(h);
    h.drop_at(pos);
    settle(h);
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

mod control_tests;
mod show_tests;
