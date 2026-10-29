//! Cue list, MIDI learn, OSC feedback and the phone remote through the real UI.

#![allow(clippy::unwrap_used, clippy::float_cmp, clippy::too_many_lines)]

use super::*;
use crate::editor::Editor;
use eframe::egui::{accesskit::Role, Key};
use egui_kittest::kittest::Queryable;
use lumen_core::midimap::{MidiEvent, MidiKey, MidiMode};
use lumen_core::params::ParamValue;
use lumen_core::project::demo_project;

#[test]
fn cue_panel_stores_fires_updates_and_reorders() {
    let mut h = harness(Editor::new(demo_project(), None));
    assert!(h
        .query_by_label("No cues yet. Set up a look, then Store cue. Space fades to the next one.")
        .is_some());

    let field = h.get_by_role_and_label(Role::TextInput, "Cue name");
    field.focus();
    field.type_text("opening");
    settle(&mut h);
    click(&mut h, "Store cue");
    assert_eq!(h.state().editor.project.cues.len(), 1);
    assert_eq!(h.state().editor.project.cues[0].name, "opening");
    assert_eq!(h.state().editor.project.cues[0].fade_seconds, 2.0);
    assert!(h.state().panels.cue_name.is_empty(), "the name box clears");

    // A second cue with blackout on, stored after the first.
    h.state_mut()
        .editor
        .set_param("/show/blackout", ParamValue::Bool(true));
    h.state_mut().editor.commit();
    click(&mut h, "Store cue");
    assert_eq!(h.state().editor.project.cues.len(), 2);
    let second = h.state().editor.project.cues[1].id.clone();
    assert_eq!(second, "cue");
    assert_eq!(
        h.state().editor.project.cues[1].values["/show/blackout"],
        ParamValue::Bool(true)
    );

    // GO the first cue by its button: blackout fades back off (fade 2 s, bools switch half way).
    h.state_mut().panels.cue_fade = 0.0;
    click(&mut h, "GO opening");
    assert_eq!(h.state().editor.cues.current, Some(0));
    assert!(h.query_by_label("fading").is_some());
    h.state_mut().editor.cues.fade.as_mut().unwrap().duration = 0.0;
    settle(&mut h);
    assert!(!h.state().editor.project.blackout);
    assert!(h
        .query_by_label("(current) 1 opening (opening) 2.0s")
        .is_some());

    // Space GOs the next cue.
    h.key_press(Key::Space);
    settle(&mut h);
    assert_eq!(h.state().editor.cues.current, Some(1));
    click(&mut h, "Stop fade");
    assert!(h.state().editor.cues.fade.is_none());

    // Update the selected cue with the live look, move it, delete it.
    h.state_mut()
        .editor
        .set_param("/show/master", ParamValue::Number(0.25));
    h.state_mut().editor.commit();
    click(&mut h, "Update cue");
    assert_eq!(
        h.state().editor.project.cues[1].values["/show/master"],
        ParamValue::Number(0.25)
    );
    click(&mut h, "Cue up");
    assert_eq!(h.state().editor.project.cues[0].id, second);
    assert_eq!(h.state().editor.selected_cue, Some(0));
    click(&mut h, "Cue down");
    assert_eq!(h.state().editor.project.cues[1].id, second);
    click(&mut h, "Delete cue");
    assert_eq!(h.state().editor.project.cues.len(), 1);
    assert_eq!(h.state().editor.project.cues[0].id, "opening");

    // The GO button in the header fires the next cue; Space while typing does not.
    click(&mut h, "GO");
    assert_eq!(h.state().editor.cues.current, Some(0));
    let field = h.get_by_role_and_label(Role::TextInput, "Cue name");
    field.focus();
    field.type_text(" ");
    settle(&mut h);
    h.key_press(Key::Space);
    settle(&mut h);
    assert_eq!(
        h.state().editor.cues.current,
        Some(0),
        "typing a space is not GO"
    );
    // Selecting a cue in the list.
    h.state_mut().editor.selected_cue = None;
    settle(&mut h);
    click(&mut h, "(current) 1 opening (opening) 2.0s");
    assert_eq!(h.state().editor.selected_cue, Some(0));
}

#[test]
fn control_section_learns_midi_and_runs_feedback_and_remote() {
    let mut h = harness(Editor::new(demo_project(), None));
    assert!(h.query_by_label("Control").is_some());
    assert!(h.query_by_label("MIDI off").is_some());
    click(&mut h, "Refresh MIDI ports");
    // Opening no port is fine; opening a nonexistent one reports an error.
    click(&mut h, "Open MIDI port");
    assert!(h.state().editor.midi.is_none());
    assert!(h.state().panels.error.is_none());
    h.state_mut().panels.midi_port = "no such midi port".into();
    click(&mut h, "Open MIDI port");
    assert!(h.state().panels.error.is_some());
    assert!(h.state().editor.midi.is_none());
    h.state_mut().panels.midi_port.clear();
    click(&mut h, "Open MIDI port");
    assert!(h.state().panels.error.is_none());

    // Learn the master fader from the typed address, then feed a CC message.
    click(&mut h, "Learn fader");
    assert_eq!(
        h.state().editor.midi_learn,
        Some(("/show/master".to_string(), MidiMode::Fader))
    );
    assert!(h
        .query_by_label("move a control for /show/master")
        .is_some());
    click(&mut h, "Cancel learn");
    assert!(h.state().editor.midi_learn.is_none());
    click(&mut h, "Learn button");
    let cc = MidiEvent {
        key: MidiKey::Cc {
            channel: 0,
            controller: 7,
        },
        value: 127,
    };
    h.state_mut().editor.handle_midi_event(cc, 0.0);
    settle(&mut h);
    assert_eq!(h.state().editor.project.midi.mappings.len(), 1);
    assert!(h
        .query_by_label("last: Cc { channel: 0, controller: 7 } = 127")
        .is_some());
    click(&mut h, "Forget /show/master");
    assert!(h.state().editor.project.midi.mappings.is_empty());

    // The surface inspector has a one-click learn for opacity.
    click(&mut h, "logo");
    click(&mut h, "MIDI learn opacity");
    assert_eq!(
        h.state().editor.midi_learn,
        Some(("/surface/logo/opacity".to_string(), MidiMode::Fader))
    );
    h.state_mut().editor.handle_midi_event(cc, 0.0);
    h.state_mut()
        .editor
        .handle_midi_event(MidiEvent { value: 0, ..cc }, 0.0);
    settle(&mut h);
    assert_eq!(
        h.state().editor.project.surface("logo").unwrap().opacity,
        0.0
    );

    // OSC feedback: a bad target errors, a loopback target is accepted and saved.
    h.state_mut().editor.selection = Selection::None;
    settle(&mut h);
    h.state_mut().panels.feedback_target = "not a host".into();
    click(&mut h, "Apply feedback");
    assert!(h.state().panels.error.is_some());
    assert!(h.state().editor.feedback.is_none());
    h.state_mut().panels.feedback_target = "127.0.0.1:1".into();
    click(&mut h, "Apply feedback");
    assert!(h.state().editor.feedback.is_some());
    assert_eq!(h.state().settings.osc_feedback, "127.0.0.1:1");
    assert!(h.query_by_label("sending to 127.0.0.1:1").is_some());
    h.state_mut().panels.feedback_target.clear();
    click(&mut h, "Apply feedback");
    assert!(h.state().editor.feedback.is_none());

    // Phone remote on an ephemeral port: the URL and a QR code appear, Stop removes them.
    assert!(h.query_by_label("remote off").is_some());
    h.state_mut().panels.remote_port = 0;
    click(&mut h, "Start remote");
    assert!(h.state().editor.remote.is_none(), "port 0 means off");
    let blocker = lumen_control::web::RemoteServer::bind(0).unwrap();
    let free = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    h.state_mut().panels.remote_port = free;
    click(&mut h, "Start remote");
    assert!(
        h.state().editor.remote.is_some(),
        "remote started on {free}"
    );
    assert_eq!(h.state().settings.remote_port, free);
    let url = h.state().editor.remote_url().unwrap();
    assert!(h.query_by_label(url.as_str()).is_some());
    assert!(h
        .query_by_label("Scan with a phone on the same Wi-Fi:")
        .is_some());
    h.state_mut().panels.remote_port = blocker.addr().port();
    click(&mut h, "Start remote");
    assert!(h.state().panels.error.is_some(), "a taken port is reported");
    click(&mut h, "Stop remote");
    assert!(h.state().editor.remote.is_none());
    assert!(h.query_by_label("remote off").is_some());
    drop(blocker);
}

#[test]
fn settings_open_controllers_and_show_launch_takes_a_remote_port() {
    let dir = config_dir();
    let free = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let settings = crate::settings::Settings {
        midi_port: "no such midi port".into(),
        osc_feedback: "127.0.0.1:1".into(),
        remote_port: free,
        ..crate::settings::Settings::default()
    };
    let mut a = app(Editor::new(demo_project(), None));
    a.apply_settings(settings);
    assert!(
        a.editor.midi.is_none(),
        "a missing port is logged, not fatal"
    );
    assert_eq!(a.panels.midi_port, "no such midi port");
    assert!(a.editor.feedback.is_some());
    assert!(a.editor.remote.is_some());
    assert_eq!(a.panels.remote_port, free);
    drop(a);

    let launch = Launch {
        project: None,
        osc_port: 0,
        show: true,
        restart_on_stall: None,
        remote_port: Some(free),
    };
    let app = LumenApp::new(
        Editor::new(demo_project(), None),
        &launch,
        monitors(),
        wgpu::TextureFormat::Rgba8Unorm,
    );
    assert!(app.editor.remote.is_some());
    let bad = Launch {
        remote_port: Some(app.editor.remote.as_ref().unwrap().addr().port()),
        ..launch
    };
    let second = LumenApp::new(
        Editor::new(demo_project(), None),
        &bad,
        monitors(),
        wgpu::TextureFormat::Rgba8Unorm,
    );
    assert!(second.editor.remote.is_none(), "a taken port is logged");
    let _ = dir;
}
