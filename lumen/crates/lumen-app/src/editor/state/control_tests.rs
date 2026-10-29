use super::*;
use lumen_core::midimap::{MidiKey, MidiMode};
use lumen_core::project::demo_project;
use std::io::{Read, Write};
use std::time::Duration;

fn editor() -> Editor {
    Editor::new(demo_project(), None)
}

fn patch(a: &str, v: ParamValue) -> ParamPatch {
    ParamPatch {
        address: ParamAddress::parse(a).unwrap(),
        value: v,
    }
}

#[test]
fn cue_list_edits() {
    let mut e = editor();
    assert_eq!(e.store_cue("Opening", 1.5), "Opening");
    e.set_param("/surface/logo/opacity", ParamValue::Number(0.2));
    e.commit();
    assert_eq!(e.store_cue("", 0.0), "cue");
    assert_eq!(e.project.cues.len(), 2);
    assert_eq!(e.selected_cue, Some(1));
    assert_eq!(
        e.project.cues[1].values["/surface/logo/opacity"],
        ParamValue::Number(0.2)
    );
    assert!(e.move_cue(1, -1));
    assert_eq!(e.project.cues[0].id, "cue");
    assert_eq!(e.selected_cue, Some(0));
    assert!(!e.move_cue(0, -1));
    assert!(!e.move_cue(5, 1));
    e.set_param("/surface/logo/opacity", ParamValue::Number(0.9));
    e.commit();
    assert!(e.update_cue(0));
    assert_eq!(
        e.project.cues[0].values["/surface/logo/opacity"],
        ParamValue::Number(0.9)
    );
    assert!(!e.update_cue(9));
    e.cues.current = Some(0);
    assert!(e.delete_cue(0));
    assert_eq!(e.cues.current, None);
    assert_eq!(e.selected_cue, Some(0));
    assert!(e.delete_cue(0));
    assert_eq!(e.selected_cue, None);
    assert!(!e.delete_cue(0));
    assert!(e.undo());
    assert_eq!(e.project.cues.len(), 1);
}

#[test]
fn cue_playback_and_control_routing() {
    let mut e = editor();
    e.store_cue("A", 2.0);
    e.project.cues[0]
        .values
        .insert("/surface/logo/opacity".into(), ParamValue::Number(0.0));
    e.store_cue("B", 0.0);
    e.project.cues[1]
        .values
        .insert("/surface/logo/opacity".into(), ParamValue::Number(1.0));
    assert!(e.handle_control(&patch("/cue/go", ParamValue::Bool(true)), 10.0));
    assert_eq!(e.cues.current, Some(0));
    assert!(e.tick_cues(11.0) > 0);
    assert!((e.project.surface("logo").unwrap().opacity - 0.4).abs() < 1e-9);
    assert!(e.handle_control(&patch("/cue/stop", ParamValue::Bool(true)), 11.0));
    assert!(!e.cues.is_fading());
    assert!(e.handle_control(&patch("/cue/B/go", ParamValue::Bool(true)), 12.0));
    e.tick_cues(12.0);
    assert_eq!(e.project.surface("logo").unwrap().opacity, 1.0);
    assert!(!e.handle_control(&patch("/cue/ghost/go", ParamValue::Bool(true)), 12.0));
    assert!(e.handle_control(&patch("/show/master", ParamValue::Number(0.5)), 12.0));
    assert_eq!(e.project.master, 0.5);
    assert!(!e.handle_control(&patch("/nope", ParamValue::Number(0.5)), 12.0));
    assert!(e.cue_go(1, 13.0) && !e.cue_go(7, 13.0));
    assert_eq!(e.cue_go_next(14.0), Some(0), "wraps from the last cue");
    let mut empty = Editor::new(Project::default(), None);
    assert_eq!(empty.cue_go_next(0.0), None);
    assert_eq!(empty.tick_cues(0.0), 0);
}

#[test]
fn midi_learn_and_translation() {
    let mut e = editor();
    assert_eq!(e.poll_midi(0.0), 0, "no port");
    e.arm_midi_learn("/show/master", MidiMode::Fader);
    assert!(e.status.contains("/show/master"));
    let ev = MidiEvent {
        key: MidiKey::Cc {
            channel: 0,
            controller: 7,
        },
        value: 127,
    };
    assert_eq!(
        e.handle_midi_event(ev, 0.0),
        0,
        "the learning event itself maps, not applies"
    );
    assert_eq!(e.project.midi.mappings.len(), 1);
    assert!(e.midi_learn.is_none());
    let ev = MidiEvent {
        key: MidiKey::Cc {
            channel: 0,
            controller: 7,
        },
        value: 0,
    };
    assert_eq!(e.handle_midi_event(ev, 0.0), 1);
    assert_eq!(e.project.master, 0.0);
    assert_eq!(e.last_midi, Some(ev));
    e.arm_midi_learn("/cue/go", MidiMode::Button);
    e.store_cue("x", 0.0);
    let note = MidiEvent {
        key: MidiKey::Note {
            channel: 0,
            note: 36,
        },
        value: 100,
    };
    e.handle_midi_event(note, 0.0);
    assert_eq!(e.handle_midi_event(note, 1.0), 1, "GO fired");
    assert_eq!(e.cues.current, Some(0));
    assert!(e.set_midi_port(None).is_ok());
    let _ = e.set_midi_port(Some("definitely-no-such-port"));
    assert!(e.midi.is_none());
}

#[test]
fn feedback_and_remote() {
    let mut e = editor();
    assert!(e.set_feedback("   ").is_ok() && e.feedback.is_none());
    assert!(e.set_feedback("not an address").is_err());
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    e.set_feedback(&sock.local_addr().unwrap().to_string())
        .unwrap();
    e.set_param("/show/master", ParamValue::Number(0.3));
    let mut buf = [0u8; 256];
    let (n, _) = sock.recv_from(&mut buf).unwrap();
    assert!(n > 0, "the edit was echoed");
    e.handle_control(&patch("/show/master", ParamValue::Number(0.4)), 0.0);
    assert!(sock.recv_from(&mut buf).unwrap().0 > 0);

    assert!(e.set_remote_port(0).is_ok() && e.remote.is_none());
    assert_eq!(e.remote_url(), None);
    assert_eq!(e.poll_remote(0.0), 0);
    e.remote = Some(RemoteServer::bind(0).unwrap());
    let url = e.remote_url().unwrap();
    assert!(url.starts_with("http://"));
    let blocker = RemoteServer::bind(0).unwrap();
    assert!(
        e.set_remote_port(blocker.addr().port()).is_err(),
        "port already taken by another server"
    );
    drop(blocker);
    e.remote = Some(RemoteServer::bind(0).unwrap());
    e.store_cue("c", 0.0);
    assert_eq!(e.poll_remote(0.0), 0);
    // A command posted over HTTP reaches the editor on the next poll.
    let addr =
        std::net::SocketAddr::from(([127, 0, 0, 1], e.remote.as_ref().unwrap().addr().port()));
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    let body = r#"{"address":"/show/blackout","value":true}"#;
    write!(s, "POST /api/param HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut resp = String::new();
    s.read_to_string(&mut resp).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(e.poll_remote(1.0), 1);
    assert!(e.project.blackout);
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    write!(s, "POST /api/cue/go HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
    s.read_to_string(&mut resp).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(e.poll_remote(2.0), 1);
    assert_eq!(e.cues.current, Some(0));
}
