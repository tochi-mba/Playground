use super::*;
use lumen_core::geom::Vec2;
use lumen_core::project::demo_project;
use lumen_core::warp::{BezierMesh, Quad};
use std::time::Duration;

fn editor() -> Editor {
    Editor::new(demo_project(), None)
}

#[test]
fn params_go_through_undo_with_coalescing() {
    let mut e = editor();
    assert!(e.set_param("/surface/logo/opacity", ParamValue::Number(0.5)));
    assert!(e.set_param("/surface/logo/opacity", ParamValue::Number(0.3)));
    assert_eq!(
        e.get_param("/surface/logo/opacity"),
        Some(ParamValue::Number(0.3))
    );
    assert_eq!(e.history.undo_len(), 1, "drag coalesced");
    assert!(e.dirty);
    e.commit();
    assert!(e.set_param("/surface/logo/opacity", ParamValue::Number(0.9)));
    assert_eq!(e.history.undo_len(), 2);
    assert!(e.undo());
    assert_eq!(
        e.get_param("/surface/logo/opacity"),
        Some(ParamValue::Number(0.3))
    );
    assert!(e.undo());
    assert_eq!(
        e.get_param("/surface/logo/opacity"),
        Some(ParamValue::Number(0.8))
    );
    assert!(!e.undo());
    assert!(e.redo());
    assert!(
        e.set_param("/surface/logo/opacity", ParamValue::Number(0.3)),
        "no-op when unchanged"
    );
    assert!(!e.set_param("/nope", ParamValue::Number(1.0)));
    assert!(!e.set_param("bad", ParamValue::Number(1.0)));
}

#[test]
fn geometry_commands_coalesce_and_revert() {
    let mut e = editor();
    let mut q = Quad::full_frame();
    q.set_corner(0, Vec2::new(0.1, 0.1));
    assert!(e.set_warp("wall-right", WarpKind::Quad(q)));
    q.set_corner(0, Vec2::new(0.2, 0.2));
    assert!(e.set_warp("wall-right", WarpKind::Quad(q)));
    assert_eq!(e.history.undo_len(), 1);
    assert!(
        e.set_warp("wall-right", WarpKind::Quad(q)),
        "identical warp is a no-op"
    );
    assert!(!e.set_warp("ghost", WarpKind::default()));
    e.commit();
    let mesh = BezierMesh::flat(2, 2).unwrap();
    assert!(e.set_warp("wall-right", WarpKind::Bezier(mesh.clone())));
    assert_eq!(e.history.undo_len(), 2);
    e.undo();
    assert_eq!(
        e.project.surface("wall-right").unwrap().warp,
        WarpKind::Quad(q)
    );
    e.undo();
    assert_eq!(
        e.project.surface("wall-right").unwrap().warp,
        WarpKind::default()
    );

    // Masks are a separate coalescing stream from warps.
    let m = vec![Mask::hole(vec![
        Vec2::ZERO,
        Vec2::new(1.0, 0.0),
        Vec2::new(1.0, 1.0),
    ])];
    assert!(e.set_masks("wall-right", m.clone()));
    assert!(
        e.set_masks("wall-right", m.clone()),
        "identical masks are a no-op"
    );
    assert!(!e.set_masks("ghost", m.clone()));
    assert!(e.set_warp("wall-right", WarpKind::Bezier(mesh)));
    assert!(e.set_masks("wall-right", vec![]));
    assert_eq!(
        e.history.undo_len(),
        3,
        "warp between two mask edits splits them"
    );
    e.undo();
    assert_eq!(e.project.surface("wall-right").unwrap().masks, m);
}

#[test]
fn structural_edits_are_single_snapshots() {
    let mut e = editor();
    e.add_output();
    assert_eq!(e.selection, Selection::Output("output".into()));
    assert_eq!(e.project.outputs.len(), 3);
    e.add_output();
    assert_eq!(e.project.outputs[3].id, "output-2");
    e.add_surface("output");
    assert_eq!(
        e.selection,
        Selection::Surface {
            id: "surface".into(),
            mask: None
        }
    );
    assert_eq!(e.project.surfaces.last().unwrap().output, "output");
    let n = e.history.undo_len();
    assert!(e.edit(|_| {}), "no change is still ok");
    assert_eq!(e.history.undo_len(), n);
    e.delete_selection();
    assert_eq!(e.project.surfaces.len(), 3);
    e.selection = Selection::Output("output".into());
    e.open_outputs.push("output".into());
    e.delete_selection();
    assert_eq!(e.project.outputs.len(), 3);
    assert!(e.open_outputs.is_empty());
    assert_eq!(e.selection, Selection::None);
    e.undo();
    assert_eq!(e.project.outputs.len(), 4);
    e.selection = Selection::None;
    e.delete_selection();
}

#[test]
fn masks_add_and_delete() {
    let mut e = editor();
    e.add_mask("wall-left");
    assert_eq!(
        e.selection,
        Selection::Surface {
            id: "wall-left".into(),
            mask: Some(0)
        }
    );
    assert_eq!(e.project.surface("wall-left").unwrap().masks.len(), 1);
    e.add_mask("ghost");
    e.delete_selection();
    assert!(e.project.surface("wall-left").unwrap().masks.is_empty());
    assert_eq!(
        e.selection,
        Selection::Surface {
            id: "wall-left".into(),
            mask: None
        }
    );
    e.selection = Selection::Surface {
        id: "wall-left".into(),
        mask: Some(7),
    };
    e.delete_selection();
    assert_eq!(
        e.selection,
        Selection::Surface {
            id: "wall-left".into(),
            mask: None
        }
    );
}

#[test]
fn sources_add_with_fresh_ids_and_reload() {
    let mut e = Editor::new(Project::default(), None);
    e.add_surface("main");
    assert_eq!(
        e.project.sources.len(),
        1,
        "a surface on an empty project gets a pattern"
    );
    let id = e.add_source(SourceKind::Solid {
        color: [1.0, 0.0, 0.0, 1.0],
    });
    assert_eq!(id, "solid");
    assert_eq!(
        e.add_source(SourceKind::Solid { color: [0.0; 4] }),
        "solid-2"
    );
    assert_eq!(
        e.add_source(SourceKind::Image {
            path: "missing.png".into()
        }),
        "image"
    );
    assert!(e.status.contains("failed to open"));
    assert_eq!(
        e.add_source(SourceKind::Spout {
            sender: String::new()
        }),
        "spout"
    );
    assert!(e.sources.frame("solid").is_some());
    assert_eq!(e.fresh_id("bad id"), "badid");
}

#[test]
fn show_mode_locks_edits_and_double_escape_exits() {
    let mut e = editor();
    e.enter_show_mode();
    assert!(e.show_mode);
    assert_eq!(e.open_outputs, ["left", "right"]);
    assert!(!e.set_param("/surface/logo/opacity", ParamValue::Number(0.1)));
    assert!(!e.set_warp("logo", WarpKind::default()));
    assert!(!e.set_masks("logo", vec![]));
    assert!(!e.edit(|p| p.name.clear()));
    assert!(!e.undo() && !e.redo());
    let t = Instant::now();
    assert!(!e.press_escape(t, Duration::from_secs(2)));
    assert!(
        !e.press_escape(t + Duration::from_secs(3), Duration::from_secs(2)),
        "too late, re-armed"
    );
    assert!(e.press_escape(t + Duration::from_secs(4), Duration::from_secs(2)));
    assert!(!e.show_mode);
    e.selection = Selection::Output("left".into());
    assert!(!e.press_escape(t, Duration::from_secs(2)));
    assert_eq!(
        e.selection,
        Selection::None,
        "escape clears the selection when editing"
    );
}

#[test]
fn save_load_autosave_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = editor();
    assert!(e.save().is_err(), "no path yet");
    let path = dir.path().join("show.lumen.toml");
    e.set_param("/show/master", ParamValue::Number(0.5));
    e.save_as(&path).unwrap();
    assert!(!e.dirty);
    assert_eq!(e.path.as_deref(), Some(path.as_path()));
    assert!(e.title().starts_with("show.lumen.toml"));
    let reopened = Editor::open(&path).unwrap();
    assert_eq!(reopened.project.master, 0.5);
    assert!(Editor::open(&dir.path().join("nope.toml")).is_err());

    // Autosave only when dirty and due.
    let now = Instant::now();
    assert!(!e.maybe_autosave(now + Duration::from_mins(1)), "clean");
    e.set_param("/show/master", ParamValue::Number(0.25));
    assert!(e.title().starts_with('*'));
    assert!(!e.maybe_autosave(now), "not due");
    assert!(e.maybe_autosave(now + Duration::from_mins(1)));
    let auto = autosave::autosave_path(&path);
    assert!(auto.exists());
    assert_eq!(Project::load(&auto).unwrap().master, 0.25);
    e.save().unwrap();
    assert!(!auto.exists(), "clean save discards the autosave");
    e.enter_show_mode();
    e.dirty = true;
    assert!(
        !e.maybe_autosave(now + Duration::from_mins(10)),
        "never in show mode"
    );
    let mut untitled = editor();
    untitled.dirty = true;
    assert!(
        !untitled.maybe_autosave(now + Duration::from_mins(10)),
        "no path"
    );
    let mut bad = editor();
    bad.path = Some(dir.path().join("missing-dir").join("x").join("y.toml"));
    std::fs::write(dir.path().join("missing-dir"), b"file").unwrap();
    bad.dirty = true;
    assert!(!bad.maybe_autosave(now + Duration::from_mins(10)));
    assert!(bad.status.contains("autosave failed"));
}

#[test]
fn osc_patches_apply_live() {
    let mut e = editor();
    assert_eq!(e.poll_osc(), 0, "no server");
    e.set_osc_port(0);
    assert!(e.osc.is_none());
    e.set_osc_port(0);
    // Bind to an ephemeral port by asking for 0 through the server directly.
    e.osc = Some(OscServer::bind("127.0.0.1:0").unwrap());
    let addr = e.osc.as_ref().unwrap().local_addr();
    let client = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let msg = rosc_packet("/show/blackout", 1.0);
    client.send_to(&msg, addr).unwrap();
    client
        .send_to(&rosc_packet("/surface/ghost/opacity", 1.0), addr)
        .unwrap();
    let mut applied = 0;
    for _ in 0..50 {
        applied += e.poll_osc();
        if applied > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(applied, 1);
    assert!(e.project.blackout);
    assert!(e.dirty);
    assert!(e
        .last_patch
        .as_deref()
        .unwrap()
        .starts_with("/show/blackout"));
}

fn rosc_packet(addr: &str, v: f32) -> Vec<u8> {
    // Hand-encode a float OSC message so the test does not need the rosc crate.
    fn pad(mut s: Vec<u8>) -> Vec<u8> {
        s.push(0);
        while !s.len().is_multiple_of(4) {
            s.push(0);
        }
        s
    }
    let mut out = pad(addr.as_bytes().to_vec());
    out.extend(pad(b",f".to_vec()));
    out.extend(v.to_be_bytes());
    out
}

#[test]
fn recovery_flow() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.lumen.toml");
    let mut e = editor();
    e.save_as(&path).unwrap();
    assert!(!e.recover(), "nothing pending");
    let auto = autosave::autosave_path(&path);
    let mut changed = demo_project();
    changed.master = 0.1;
    changed.save(&auto).unwrap();
    e.recovery = Some(auto.clone());
    assert!(e.recover());
    assert_eq!(e.project.master, 0.1);
    assert!(e.dirty);
    e.recovery = Some(auto.clone());
    e.discard_recovery();
    assert!(!auto.exists());
    e.recovery = Some(dir.path().join("missing.autosave"));
    assert!(!e.recover());
    assert!(e.status.contains("cannot recover"));
}

#[test]
fn multi_select_and_duplicate_and_lock() {
    let mut e = editor();
    e.select_surface("logo", false);
    e.select_surface("wall-right", true);
    assert_eq!(e.selected_surfaces(), ["wall-right", "logo"]);
    e.select_surface("wall-right", true);
    assert_eq!(
        e.selected_surfaces(),
        ["wall-right", "logo"],
        "re-selecting the primary is a no-op"
    );
    e.select_surface("logo", true);
    assert_eq!(
        e.selected_surfaces(),
        ["logo", "wall-right"],
        "swapped roles, still two"
    );
    e.select_surface("wall-left", false);
    assert_eq!(e.selected_surfaces(), ["wall-left"]);
    e.select_surface("logo", true);
    e.delete_selection();
    assert!(e.project.surface("logo").is_none() && e.project.surface("wall-left").is_none());
    assert_eq!(e.selection, Selection::None);
    assert!(e.extra_selected.is_empty());
    e.undo();
    assert_eq!(e.project.surfaces.len(), 3);

    let copy = e.duplicate_surface("logo").unwrap();
    assert_eq!(copy, "logo-copy");
    assert_eq!(
        e.selection,
        Selection::Surface {
            id: copy.clone(),
            mask: None
        }
    );
    assert!(e.duplicate_surface("ghost").is_none());
    e.set_locked(&copy, true);
    assert!(e.is_locked(&copy) && !e.is_locked("logo") && !e.is_locked("ghost"));
    e.enter_show_mode();
    assert!(e.duplicate_surface("logo").is_none());
}

#[test]
fn rename_updates_selection_and_windows() {
    let mut e = editor();
    e.selection = Selection::Surface {
        id: "logo".into(),
        mask: Some(0),
    };
    e.rename("logo", " badge ").unwrap();
    assert_eq!(
        e.selection,
        Selection::Surface {
            id: "badge".into(),
            mask: Some(0)
        }
    );
    assert!(e.project.surface("badge").is_some());
    e.open_outputs.push("left".into());
    e.selection = Selection::Output("left".into());
    e.rename("left", "L").unwrap();
    assert_eq!(e.selection, Selection::Output("L".into()));
    assert_eq!(e.open_outputs, ["L"]);
    assert_eq!(e.project.surfaces[0].output, "L");
    e.rename("grid", "g").unwrap();
    assert!(e.sources.frame("g").is_some());
    assert!(e.rename("g", "bad id").is_err());
    assert!(e.rename("ghost", "x").is_err());
    assert!(e.rename("g", "g").is_ok());
    assert!(e.undo(), "a rename is one undo step");
    assert!(e.project.source("grid").is_some());
    e.enter_show_mode();
    assert!(e.rename("grid", "x").is_err());
}

#[test]
fn ordering_and_alignment() {
    let mut e = editor();
    let order = |e: &Editor| {
        e.project
            .surfaces_for_output("right")
            .iter()
            .map(|s| s.id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(order(&e), ["wall-right", "logo"]);
    e.move_surface("logo", -1);
    assert_eq!(order(&e), ["logo", "wall-right"]);
    e.move_surface("logo", -1);
    assert_eq!(order(&e), ["logo", "wall-right"], "already at the bottom");
    e.send_surface("logo", true);
    assert_eq!(order(&e), ["wall-right", "logo"]);
    e.send_surface("logo", false);
    assert_eq!(order(&e), ["logo", "wall-right"]);
    e.move_surface("ghost", 1);

    for op in [
        AlignOp::FlipHorizontal,
        AlignOp::FlipVertical,
        AlignOp::RotateQuarter,
        AlignOp::Centre,
        AlignOp::MatchSourceAspect,
        AlignOp::FitOutput,
    ] {
        assert!(e.align("logo", op), "{op:?}");
        assert!(e.project.is_valid());
    }
    assert_eq!(e.project.surface("logo").unwrap().warp, WarpKind::default());
    assert!(!e.align("ghost", AlignOp::Centre));
    let mut orphan = editor();
    orphan.project.surfaces[2].output = "ghost".into();
    assert!(!orphan.align("logo", AlignOp::Centre));
    let mut no_frame = editor();
    no_frame.project.surfaces[2].source = "ghost".into();
    assert!(
        no_frame.align("logo", AlignOp::MatchSourceAspect),
        "falls back to the output aspect"
    );
}

#[test]
fn identify_and_monitor_outputs() {
    let mut e = Editor::new(Project::default(), None);
    let t = Instant::now();
    assert!(!e.identify_active(t));
    e.identify(t);
    assert!(e.identify_active(t + Duration::from_secs(1)));
    assert!(!e.identify_active(t + Duration::from_secs(4)));
    assert!(e.identify_until.is_none());

    let monitors = vec![
        crate::editor::MonitorInfo {
            name: "a".into(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
        },
        crate::editor::MonitorInfo {
            name: "b".into(),
            x: 1920,
            y: 0,
            width: 1280,
            height: 720,
            scale: 1.0,
        },
        crate::editor::MonitorInfo {
            name: "c".into(),
            x: 3200,
            y: 0,
            width: 0,
            height: 0,
            scale: 1.0,
        },
    ];
    let ids = e.create_outputs_for_monitors(&monitors, true);
    assert_eq!(ids, ["projector-1", "projector-2"]);
    assert_eq!(
        e.project.outputs[0].placement.as_ref().unwrap().monitor,
        "b"
    );
    assert_eq!(
        (e.project.outputs[1].width, e.project.outputs[1].height),
        (1, 1),
        "zero sizes are clamped"
    );
    assert_eq!(e.project.surfaces.len(), 2);
    assert_eq!(e.open_outputs, ids);
    assert_eq!(e.selection, Selection::Output("projector-1".into()));
    assert!(e.dirty && !e.history.can_undo());
    assert!(e.project.is_valid());
    let more = e.create_outputs_for_monitors(&monitors[..1], true);
    assert_eq!(
        more,
        ["projector-1-2"],
        "a single monitor is used even when skipping the primary"
    );
    assert!(e.create_outputs_for_monitors(&[], false).is_empty());
}

#[test]
fn files_become_sources_and_pending_actions() {
    let dir = tempfile::tempdir().unwrap();
    let img = dir.path().join("photo.PNG");
    image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
        .save(&img)
        .unwrap();
    let mut e = editor();
    e.path = Some(dir.path().join("show.lumen.toml"));
    e.selection = Selection::Output("right".into());
    assert_eq!(e.add_files(&[img.clone(), dir.path().join("notes.txt")]), 1);
    assert!(e.status.contains("unsupported") || e.status.contains("added"));
    let src = e.project.source("photo").unwrap();
    assert_eq!(
        src.kind,
        SourceKind::Image {
            path: "photo.PNG".into()
        },
        "stored relative to the project"
    );
    assert_eq!(
        e.selection,
        Selection::Surface {
            id: "photo-surface".into(),
            mask: None
        }
    );
    assert_eq!(e.project.surface("photo-surface").unwrap().output, "right");
    assert!(e.sources.frame("photo").is_some());
    assert!(matches!(
        source_kind_for(Path::new("x.mov")),
        Some(SourceKind::Video { .. })
    ));
    assert!(matches!(
        source_kind_for(Path::new("x.GIF")),
        Some(SourceKind::Sequence { .. })
    ));
    assert!(
        matches!(
            source_kind_for(dir.path()),
            Some(SourceKind::Sequence { .. })
        ),
        "folders are sequences"
    );
    assert!(source_kind_for(Path::new("x.txt")).is_none());
    assert!(source_kind_for(Path::new("noext")).is_none());
    let mut empty = Editor::new(Project::default(), None);
    assert_eq!(empty.add_files(&[img]), 1, "no output: source only");
    assert!(empty.project.surfaces.is_empty());

    assert!(!e.request(Pending::Close), "add_files dirtied the project");
    e.pending = None;
    e.dirty = false;
    assert!(e.request(Pending::New));
    e.dirty = true;
    assert!(!e.request(Pending::Open(dir.path().join("o.toml"))));
    assert!(e.pending.is_some());
    assert_eq!(e.resolve_pending(false), None);
    assert!(e.pending.is_none());
    e.request(Pending::Close);
    assert_eq!(e.resolve_pending(true), Some(Pending::Close));
}

#[test]
fn overlays_blend_suggestion_gamma_and_placement_moves() {
    use lumen_core::project::{Overlay, Placement};
    let mut e = editor();
    assert!(e.set_overlay("left", Some(Overlay::Crosshair)));
    assert_eq!(
        e.project.output("left").unwrap().overlay,
        Some(Overlay::Crosshair)
    );
    assert!(e.set_overlay("ghost", None), "unknown ids are a no-op edit");
    assert!(e.undo());
    assert_eq!(e.project.output("left").unwrap().overlay, None);

    assert_eq!(e.suggest_blends(), 0);
    assert_eq!(e.status, "no overlapping placed outputs");
    e.edit(|p| {
        p.outputs[0].placement = Some(Placement {
            monitor: "a".into(),
            x: 0,
            y: 0,
        });
        p.outputs[1].placement = Some(Placement {
            monitor: "b".into(),
            x: 1728,
            y: 0,
        });
    });
    assert_eq!(e.suggest_blends(), 2);
    assert!(e.status.starts_with("set 2 blend edges"));
    assert!((e.project.output("right").unwrap().blend.left.width - 0.1).abs() < 1e-9);

    let g0 = e.project.output("left").unwrap().blend.gamma;
    assert!(e.adjust_blend_gamma("left", 0.05));
    assert!((e.project.output("left").unwrap().blend.gamma - (g0 + 0.05)).abs() < 1e-9);
    assert!(e.adjust_blend_gamma("left", 10.0));
    assert_eq!(e.project.output("left").unwrap().blend.gamma, 3.0);
    assert!(!e.adjust_blend_gamma("ghost", 0.1));
    assert!(e.undo(), "each nudge is its own undo step");

    assert!(!e.move_placement("left", 0, 0));
    assert!(e.move_placement("left", 10, -5));
    let p = e.project.output("left").unwrap().placement.clone().unwrap();
    assert_eq!((p.x, p.y), (10, -5));
    assert!(
        e.move_placement("ghost", 1, 1),
        "unknown id is a no-op edit"
    );
    e.enter_show_mode();
    assert!(!e.set_overlay("left", Some(Overlay::Grid)));
    assert!(!e.adjust_blend_gamma("left", 0.1));
}

#[test]
fn output_window_toggle_and_misc() {
    let mut e = editor();
    e.toggle_output_window("left");
    assert_eq!(e.open_outputs, ["left"]);
    e.toggle_output_window("left");
    assert!(e.open_outputs.is_empty());
    assert_eq!(e.base_dir(), PathBuf::from("."));
    e.exit_show_mode();
    assert_eq!(e.status, "editing");
    assert_eq!(e.title(), "untitled - lumen");
    let s = Snapshot {
        before: Project::default(),
        after: demo_project(),
    };
    assert!(Command::<Project>::coalesce_key(&s).is_none());
    let mut g = SetGeometry {
        surface: "a".into(),
        what: GeometryKind::Warp,
        before: Geometry::Warp(WarpKind::default()),
        after: Geometry::Warp(WarpKind::default()),
    };
    assert!(!Command::<Project>::merge(&mut g, &s));
    let other = SetGeometry {
        surface: "b".into(),
        ..g.clone()
    };
    assert!(!Command::<Project>::merge(&mut g, &other));
    // Geometry commands on a vanished surface are harmless.
    let mut p = Project::default();
    g.apply(&mut p);
    g.revert(&mut p);
    assert_eq!(p, Project::default());
}
