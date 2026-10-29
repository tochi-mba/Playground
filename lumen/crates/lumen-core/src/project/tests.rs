use super::*;
use crate::geom::Vec2;
use crate::params::{ParamAddress, ParamPatch};
use crate::undo::KeyedValues;
use proptest::prelude::*;

#[test]
fn demo_project_is_valid_and_round_trips() {
    let p = demo_project();
    assert_eq!(p.validate(), Vec::<Issue>::new());
    assert!(p.is_valid());
    let text = p.to_toml().unwrap();
    assert!(text.starts_with("schema_version = 1"), "{text}");
    let back = Project::from_toml(&text).unwrap();
    assert_eq!(back, p);
    assert_eq!(
        p.surfaces_for_output("right")
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["wall-right", "logo"]
    );
    assert!(p.surfaces_for_output("nope").is_empty());
    assert_eq!(p.source("grid").map(|s| s.id.as_str()), Some("grid"));
    assert!(p.output("missing").is_none());
}

#[test]
fn save_and_load_are_atomic_and_create_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("show.lumen.toml");
    let p = demo_project();
    p.save(&path).unwrap();
    assert_eq!(Project::load(&path).unwrap(), p);
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, ["show.lumen.toml"], "no temp file left behind");
    // Overwrite keeps working.
    let mut p2 = p.clone();
    p2.name = "renamed".into();
    p2.save(&path).unwrap();
    assert_eq!(Project::load(&path).unwrap().name, "renamed");
    // Saving into a path whose parent is a file fails cleanly.
    let bad = path.join("child.toml");
    assert!(matches!(p.save(&bad), Err(ProjectError::Io(_))));
    assert!(matches!(
        Project::load(&dir.path().join("none.toml")),
        Err(ProjectError::Io(_))
    ));
    // A bare file name saves into the current directory.
    let cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    p.save(Path::new("bare.toml")).unwrap();
    std::env::set_current_dir(cwd).unwrap();
    assert!(dir.path().join("bare.toml").exists());
}

#[test]
fn validate_reports_every_problem() {
    let mut p = demo_project();
    p.surfaces[0].output = "ghost".into();
    p.surfaces[0].source = "ghost".into();
    p.surfaces[0].opacity = 2.0;
    p.surfaces[1].masks.push(Mask::hole(vec![Vec2::ZERO]));
    p.outputs[0].width = 0;
    p.outputs[1].height = 20000;
    p.outputs.push(p.outputs[1].clone());
    p.sources[0].id = "bad id".into();
    if let WarpKind::Quad(q) = &mut p.surfaces[2].warp {
        q.corners[1] = q.corners[0];
    }
    p.schema_version = 99;
    let issues = p.validate();
    let msgs: Vec<String> = issues.iter().map(ToString::to_string).collect();
    let expect = [
        "error: schema_version 99 is newer than 1",
        "error: source id \"bad id\" must match",
        "error: duplicate output id \"right\"",
        "error: output \"left\" has a zero dimension",
        "warning: output \"right\" is larger than 16384px",
        "error: surface \"wall-left\" references unknown output \"ghost\"",
        "error: surface \"wall-left\" references unknown source \"ghost\"",
        "warning: surface \"wall-left\" opacity 2 is outside [0,1]",
        "error: surface \"wall-right\" mask 0 needs at least three finite points",
        "error: surface \"logo\" warp is invalid",
    ];
    for e in expect {
        assert!(
            msgs.iter().any(|m| m.starts_with(e)),
            "missing {e:?} in {msgs:#?}"
        );
    }
    assert!(!p.is_valid());
    let empty = Project::default();
    assert_eq!(
        empty.validate(),
        vec![Issue::warn("project has no outputs")]
    );
    assert!(empty.is_valid());
}

#[test]
fn migrates_schema_zero() {
    let legacy = r#"
name = "old show"
[[outputs]]
id = "a"
size = [1280, 720]
[[surfaces]]
id = "s"
output = "a"
source = "x"
alpha = 0.5
[[sources]]
id = "x"
kind = "solid"
color = [1.0, 0.0, 0.0, 1.0]
"#;
    let p = Project::from_toml(legacy).unwrap();
    assert_eq!(p.schema_version, 1);
    assert_eq!((p.outputs[0].width, p.outputs[0].height), (1280, 720));
    assert_eq!(p.surfaces[0].opacity, 0.5);
    assert!(p.is_valid());
    // Migration applies defaults for everything else.
    assert_eq!(p.outputs[0].blend, EdgeBlend::none());
    assert!(p.outputs[0].enabled);
    // A malformed size is dropped and then the schema rejects the missing width.
    let bad = "[[outputs]]\nid = \"a\"\nsize = [1]\n";
    assert!(matches!(
        Project::from_toml(bad),
        Err(ProjectError::Parse(_))
    ));
    // alpha never overrides an explicit opacity.
    let both = "[[surfaces]]\nid=\"s\"\noutput=\"a\"\nsource=\"x\"\nalpha=0.5\nopacity=0.9\n";
    assert_eq!(Project::from_toml(both).unwrap().surfaces[0].opacity, 0.9);
}

#[test]
fn rejects_newer_and_malformed_versions() {
    let err = Project::from_toml("schema_version = 2").unwrap_err();
    assert!(matches!(
        err,
        ProjectError::TooNew {
            found: 2,
            supported: 1
        }
    ));
    assert!(err.to_string().contains("newer"));
    let err = Project::from_toml("schema_version = \"one\"").unwrap_err();
    assert!(matches!(err, ProjectError::Parse(_)));
    assert!(Project::from_toml("schema_version = -1").is_err());
    assert!(Project::from_toml("this is not toml [").is_err());
    assert!(Project::from_toml("schema_version = 1\n[[outputs]]\nid = 5\n").is_err());
}

#[test]
fn params_get_and_set() {
    let mut p = demo_project();
    let set = |p: &mut Project, a: &str, v: ParamValue| {
        p.apply_patch(&ParamPatch {
            address: ParamAddress::parse(a).unwrap(),
            value: v,
        })
    };
    let get = |p: &Project, a: &str| p.get_param(&ParamAddress::parse(a).unwrap());

    set(&mut p, "/show/blackout", ParamValue::Bool(true)).unwrap();
    assert_eq!(get(&p, "/show/blackout"), Some(ParamValue::Bool(true)));
    set(&mut p, "/show/master", ParamValue::Number(7.0)).unwrap();
    assert_eq!(
        get(&p, "/show/master"),
        Some(ParamValue::Number(1.0)),
        "clamped"
    );

    set(
        &mut p,
        "/surface/logo/opacity",
        ParamValue::Text("0.25".into()),
    )
    .unwrap();
    assert_eq!(
        get(&p, "/surface/logo/opacity"),
        Some(ParamValue::Number(0.25))
    );
    set(&mut p, "/surface/logo/visible", ParamValue::Number(0.0)).unwrap();
    assert_eq!(
        get(&p, "/surface/logo/visible"),
        Some(ParamValue::Bool(false))
    );
    set(&mut p, "/surface/logo/z", ParamValue::Number(-3.0)).unwrap();
    assert_eq!(get(&p, "/surface/logo/z"), Some(ParamValue::Number(-3.0)));
    set(&mut p, "/surface/logo/corner/2/x", ParamValue::Number(0.77)).unwrap();
    assert_eq!(
        get(&p, "/surface/logo/corner/2/x"),
        Some(ParamValue::Number(0.77))
    );
    set(&mut p, "/surface/logo/corner/2/y", ParamValue::Number(0.66)).unwrap();
    assert_eq!(
        get(&p, "/surface/logo/corner/2/y"),
        Some(ParamValue::Number(0.66))
    );

    set(&mut p, "/output/left/enabled", ParamValue::Bool(false)).unwrap();
    assert_eq!(
        get(&p, "/output/left/enabled"),
        Some(ParamValue::Bool(false))
    );
    for (a, v, want) in [
        ("/output/left/brightness", 9.0, 4.0),
        ("/output/left/contrast", -1.0, 0.0),
        ("/output/left/gamma", 0.0, 0.2),
        ("/output/left/blend/gamma", 10.0, 4.0),
        ("/output/left/blend/left/width", 0.3, 0.3),
        ("/output/left/blend/right/width", 2.0, 1.0),
        ("/output/left/blend/top/width", 0.1, 0.1),
        ("/output/left/blend/bottom/width", 0.2, 0.2),
    ] {
        set(&mut p, a, ParamValue::Number(v)).unwrap();
        assert_eq!(get(&p, a), Some(ParamValue::Number(want)), "{a}");
    }

    // Unknown and bad.
    for a in [
        "/nope",
        "/show/nope",
        "/surface/ghost/opacity",
        "/surface/logo/nope",
        "/surface/logo/corner/9/x",
        "/surface/logo/corner/x/x",
        "/surface/logo/corner/1/z",
        "/output/ghost/enabled",
        "/output/left/nope",
        "/output/left/blend/diagonal/width",
    ] {
        assert!(
            matches!(
                set(&mut p, a, ParamValue::Number(1.0)),
                Err(ProjectError::UnknownParam(_))
            ),
            "{a}"
        );
        assert_eq!(get(&p, a), None, "{a}");
    }
    let err = set(
        &mut p,
        "/surface/logo/opacity",
        ParamValue::Text("loud".into()),
    )
    .unwrap_err();
    assert!(matches!(err, ProjectError::BadValue(..)));
    assert!(err.to_string().contains("cannot take value"));
    assert!(set(&mut p, "/show/blackout", ParamValue::Text("maybe".into())).is_err());

    // Corner params only exist on quads.
    p.surfaces[2].warp = WarpKind::Bezier(crate::warp::BezierMesh::flat(1, 1).unwrap());
    assert_eq!(get(&p, "/surface/logo/corner/0/x"), None);
    assert!(set(&mut p, "/surface/logo/corner/0/x", ParamValue::Number(0.0)).is_err());
}

#[test]
fn source_transport_params() {
    let mut p = demo_project();
    p.sources.push(Source {
        id: "clip".into(),
        kind: SourceKind::Video {
            path: "clip.mp4".into(),
            playback: Playback::default(),
        },
    });
    p.sources.push(Source {
        id: "gen".into(),
        kind: SourceKind::Generator {
            pattern: GeneratorKind::Plasma,
            speed: 1.0,
            color: [1.0; 4],
        },
    });
    let set = |p: &mut Project, a: &str, v: ParamValue| {
        p.apply_patch(&ParamPatch {
            address: ParamAddress::parse(a).unwrap(),
            value: v,
        })
    };
    let get = |p: &Project, a: &str| p.get_param(&ParamAddress::parse(a).unwrap());
    assert_eq!(get(&p, "/source/clip/play"), Some(ParamValue::Bool(true)));
    set(&mut p, "/source/clip/play", ParamValue::Bool(false)).unwrap();
    assert!(p.source("clip").unwrap().kind.playback().unwrap().paused);
    set(&mut p, "/source/clip/loop", ParamValue::Bool(false)).unwrap();
    assert_eq!(get(&p, "/source/clip/loop"), Some(ParamValue::Bool(false)));
    set(&mut p, "/source/clip/speed", ParamValue::Number(99.0)).unwrap();
    assert_eq!(
        get(&p, "/source/clip/speed"),
        Some(ParamValue::Number(16.0))
    );
    assert_eq!(
        get(&p, "/source/clip/restart"),
        Some(ParamValue::Bool(false))
    );
    set(&mut p, "/source/clip/restart", ParamValue::Bool(true)).unwrap();
    assert_eq!(
        p.source("clip")
            .unwrap()
            .kind
            .playback()
            .unwrap()
            .restart_token,
        1
    );
    assert!(set(&mut p, "/source/clip/nope", ParamValue::Bool(true)).is_err());
    assert!(
        set(&mut p, "/source/grid/play", ParamValue::Bool(true)).is_err(),
        "patterns have no transport"
    );
    assert_eq!(get(&p, "/source/grid/play"), None);
    assert!(set(&mut p, "/source/ghost/play", ParamValue::Bool(true)).is_err());
    set(&mut p, "/source/gen/speed", ParamValue::Number(2.5)).unwrap();
    assert_eq!(get(&p, "/source/gen/speed"), Some(ParamValue::Number(2.5)));
    assert!(set(&mut p, "/source/gen/play", ParamValue::Bool(true)).is_err());
    assert!(set(&mut p, "/source/clip/speed", ParamValue::Text("x".into())).is_err());
    let text = p.to_toml().unwrap();
    assert!(
        !text.contains("restart_token"),
        "restart token is never saved"
    );
    let back = Project::from_toml(&text).unwrap();
    assert_eq!(
        back.source("clip").unwrap().kind.playback().unwrap().speed,
        16.0
    );
    let kinds = [
        SourceKind::Video {
            path: "v".into(),
            playback: Playback::default(),
        },
        SourceKind::Sequence {
            path: "d".into(),
            fps: 25.0,
            playback: Playback::default(),
        },
        SourceKind::Generator {
            pattern: GeneratorKind::Noise,
            speed: 1.0,
            color: [1.0; 4],
        },
        SourceKind::Image { path: "i".into() },
        SourceKind::Spout {
            sender: String::new(),
        },
    ];
    assert_eq!(
        kinds.iter().map(SourceKind::label).collect::<Vec<_>>(),
        ["video", "sequence", "generator", "image", "spout"]
    );
    assert_eq!(kinds[1].path(), Some("d"));
    assert_eq!(kinds[2].path(), None);
    assert!(kinds[2].playback().is_none());
    let seq: Source = toml::from_str("id = \"s\"\nkind = \"sequence\"\npath = \"frames\"").unwrap();
    assert_eq!(
        seq.kind,
        SourceKind::Sequence {
            path: "frames".into(),
            fps: 25.0,
            playback: Playback::default()
        }
    );
    let g: Source =
        toml::from_str("id = \"g\"\nkind = \"generator\"\npattern = \"radar\"\n").unwrap();
    assert_eq!(
        g.kind,
        SourceKind::Generator {
            pattern: GeneratorKind::Radar,
            speed: 1.0,
            color: [1.0; 4]
        }
    );
    assert_eq!(GeneratorKind::ALL.len(), 6);
}

#[test]
fn keyed_values_feed_undo() {
    use crate::undo::{History, SetValue};
    let mut p = demo_project();
    let mut h: History<Project> = History::default();
    let key = "/surface/logo/opacity";
    let before = p.get_param(&ParamAddress::parse(key).unwrap()).unwrap();
    h.push(
        &mut p,
        Box::new(SetValue {
            key: key.into(),
            before: before.clone(),
            after: ParamValue::Number(0.1),
        }),
    );
    assert_eq!(p.surfaces[2].opacity, 0.1);
    h.undo(&mut p);
    assert_eq!(p.surfaces[2].opacity, 0.8);
    // Keys that no longer resolve are ignored rather than panicking.
    p.set_keyed("/surface/gone/opacity", &ParamValue::Number(0.5));
    p.set_keyed("not an address", &ParamValue::Number(0.5));
    assert_eq!(p, demo_project());
}

#[test]
fn rename_rewrites_references_and_validates() {
    let mut p = demo_project();
    p.rename_output("left", "stage-left").unwrap();
    assert_eq!(p.surfaces[0].output, "stage-left");
    assert!(p.output("stage-left").is_some());
    p.rename_source("grid", "pattern").unwrap();
    assert_eq!(
        p.surfaces.iter().filter(|s| s.source == "pattern").count(),
        2
    );
    p.rename_surface("logo", "badge").unwrap();
    assert!(p.surface("badge").is_some());
    assert!(p.is_valid());
    assert!(p.rename_surface("badge", "bad id").is_err());
    assert!(
        p.rename_surface("badge", "bars").is_err(),
        "taken by a source"
    );
    assert!(
        p.rename_surface("badge", "badge").is_ok(),
        "same id is fine"
    );
    assert!(p.rename_surface("ghost", "x").is_err());
    assert!(p.rename_output("ghost", "x").is_err());
    assert!(p.rename_source("ghost", "x").is_err());
    assert!(p.id_in_use("badge") && !p.id_in_use("free"));
}

#[test]
fn placed_rects_and_blend_suggestions() {
    use crate::project::Placement;
    let mut p = demo_project();
    assert!(p.placed_rects().is_empty());
    assert_eq!(p.suggest_blends(), 0);
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
    assert_eq!(p.placed_rects().len(), 2);
    assert_eq!(p.placed_rects()[1].1.x, 1728);
    assert_eq!(p.suggest_blends(), 2);
    assert!((p.outputs[0].blend.right.width - 0.1).abs() < 1e-9);
    assert!((p.outputs[1].blend.left.width - 0.1).abs() < 1e-9);
    assert_eq!(p.suggest_blends(), 0, "already matching");
    p.outputs[0].overlay = Some(Overlay::Grid);
    let text = p.to_toml().unwrap();
    assert!(text.contains("overlay = \"grid\""));
    assert!(!text.contains("overlay = \"none\""));
    let back = Project::from_toml(&text).unwrap();
    assert_eq!(back.outputs[0].overlay, Some(Overlay::Grid));
    assert_eq!(back.outputs[1].overlay, None);
}

#[test]
fn hand_written_files_need_only_the_fields_they_set() {
    let text = r#"
schema_version = 1
name = "minimal"
[[sources]]
id = "loop"
kind = "video"
path = "loop.mp4"
playback = { looping = true }
[[outputs]]
id = "o"
width = 1280
height = 720
[outputs.blend]
right = { width = 0.2 }
[[surfaces]]
id = "s"
output = "o"
source = "loop"
"#;
    let p = Project::from_toml(text).unwrap();
    let pb = p.sources[0].kind.playback().unwrap();
    assert!(pb.looping && !pb.paused);
    assert_eq!(pb.speed, 1.0);
    assert_eq!(p.outputs[0].blend.right.width, 0.2);
    assert_eq!(p.outputs[0].blend.gamma, 2.2);
    assert!(p.validate().iter().all(|i| !i.error), "{:?}", p.validate());
}

#[test]
fn fresh_ids_and_duplicates() {
    let mut p = demo_project();
    assert_eq!(p.fresh_id("new"), "new");
    assert_eq!(p.fresh_id("logo"), "logo-2");
    assert_eq!(p.fresh_id("bad id!"), "badid");
    assert_eq!(p.fresh_id("!!"), "item");
    let copy = p.duplicate_surface("logo").unwrap();
    assert_eq!(copy, "logo-copy");
    let c = p.surface("logo-copy").unwrap();
    assert_eq!(c.z, 2);
    assert_eq!(c.source, "bars");
    assert_eq!(p.duplicate_surface("logo").unwrap(), "logo-copy-2");
    assert!(p.duplicate_surface("ghost").is_none());
    assert!(p.is_valid());
}

#[test]
fn id_rules() {
    assert!(is_valid_id("wall-1_a"));
    assert!(!is_valid_id(""));
    assert!(!is_valid_id("a/b"));
    assert!(!is_valid_id("é"));
}

#[test]
fn serde_defaults_for_optional_fields() {
    let text = r#"
schema_version = 1
[[sources]]
id = "s"
kind = "spout"
[[outputs]]
id = "o"
width = 10
height = 10
[[surfaces]]
id = "a"
output = "o"
source = "s"
"#;
    let p = Project::from_toml(text).unwrap();
    assert_eq!(
        p.sources[0].kind,
        SourceKind::Spout {
            sender: String::new()
        }
    );
    let s = &p.surfaces[0];
    assert!(s.visible && s.opacity == 1.0 && s.z == 0 && s.masks.is_empty());
    assert_eq!(s.blend_mode, BlendMode::Normal);
    assert_eq!(s.crop, Crop::default());
    assert_eq!(s.warp, WarpKind::default());
    assert_eq!(p.outputs[0].color, ColorAdjust::default());
    assert_eq!(p.outputs[0].placement, None);
    assert_eq!(p.master, 1.0);
    assert!(!p.blackout);
    let img: Source = toml::from_str("id = \"i\"\nkind = \"image\"\npath = \"x.png\"").unwrap();
    assert_eq!(
        img.kind,
        SourceKind::Image {
            path: "x.png".into()
        }
    );
    let pl: Placement = toml::from_str("monitor = \"m\"").unwrap();
    assert_eq!(
        pl,
        Placement {
            monitor: "m".into(),
            x: 0,
            y: 0
        }
    );
    for pat in ["grid", "bars", "checker", "ramp"] {
        let s: Source = toml::from_str(&format!(
            "id = \"p\"\nkind = \"test_pattern\"\npattern = \"{pat}\""
        ))
        .unwrap();
        assert!(matches!(s.kind, SourceKind::TestPattern { .. }));
    }
}

proptest! {
    #[test]
    fn opacity_patches_always_land_in_range(v in -10.0f64..10.0) {
        let mut p = demo_project();
        p.apply_patch(&ParamPatch {
            address: ParamAddress::parse("/surface/logo/opacity").unwrap(),
            value: ParamValue::Number(v),
        }).unwrap();
        prop_assert!((0.0..=1.0).contains(&p.surfaces[2].opacity));
        prop_assert!(p.is_valid());
    }

    #[test]
    fn toml_round_trip_preserves_any_name(name in "[a-zA-Z0-9 _-]{0,20}") {
        let mut p = demo_project();
        p.name = name;
        let back = Project::from_toml(&p.to_toml().unwrap()).unwrap();
        prop_assert_eq!(back, p);
    }
}
