//! Controllers: OSC, cues, MIDI and the phone remote, routed through [`Editor`].

use super::Editor;
use lumen_control::feedback::OscFeedback;
use lumen_control::midi::MidiListener;
use lumen_control::web::{RemoteCommand, RemoteCue, RemoteServer, RemoteState};
use lumen_core::cues::{capture, Cue};
use lumen_core::midimap::{MidiEvent, MidiMode};
use lumen_core::params::ParamPatch;

impl Editor {
    /// Applies a patch from a controller (OSC, MIDI, the phone remote). Cue addresses
    /// (`/cue/go`, `/cue/<id>/go`, `/cue/stop`) are handled here; everything else goes to the
    /// project directly, without undo, and is echoed to OSC feedback.
    pub fn handle_control(&mut self, patch: &ParamPatch, now: f64) -> bool {
        let segs: Vec<&str> = patch
            .address
            .segments()
            .iter()
            .map(String::as_str)
            .collect();
        let ok = match segs.as_slice() {
            ["cue", "go"] => self.cue_go_next(now).is_some(),
            ["cue", "stop"] => {
                self.cues.stop();
                true
            }
            ["cue", id, "go"] => self.cue_go_id(id, now),
            _ => match self.project.apply_patch(patch) {
                Ok(()) => {
                    self.dirty = true;
                    if let Some(fb) = &self.feedback {
                        fb.send(patch);
                    }
                    true
                }
                Err(e) => {
                    log::debug!("control: {e}");
                    false
                }
            },
        };
        if ok {
            self.last_patch = Some(format!("{} = {:?}", patch.address, patch.value));
        }
        ok
    }

    /// Stores the live look as a new cue after the selected one and selects it.
    pub fn store_cue(&mut self, name: &str, fade_seconds: f64) -> String {
        let id = self.fresh_id(if name.trim().is_empty() {
            "cue"
        } else {
            name.trim()
        });
        let cue = Cue {
            id: id.clone(),
            name: name.trim().to_string(),
            fade_seconds: fade_seconds.max(0.0),
            values: capture(&self.project),
        };
        let at = self.selected_cue.map_or(self.project.cues.len(), |i| {
            (i + 1).min(self.project.cues.len())
        });
        self.edit(move |p| p.cues.insert(at, cue));
        self.selected_cue = Some(at);
        id
    }

    /// Overwrites a cue's values with the live look.
    pub fn update_cue(&mut self, index: usize) -> bool {
        if index >= self.project.cues.len() {
            return false;
        }
        let values = capture(&self.project);
        self.edit(move |p| {
            if let Some(c) = p.cues.get_mut(index) {
                c.values = values;
            }
        })
    }

    /// Deletes a cue.
    pub fn delete_cue(&mut self, index: usize) -> bool {
        if index >= self.project.cues.len() {
            return false;
        }
        self.edit(move |p| {
            p.cues.remove(index);
        });
        self.selected_cue = if self.project.cues.is_empty() {
            None
        } else {
            Some(index.min(self.project.cues.len() - 1))
        };
        if self.cues.current == Some(index) {
            self.cues.current = None;
        }
        true
    }

    /// Moves a cue up (`-1`) or down (`+1`) in the list.
    pub fn move_cue(&mut self, index: usize, delta: i32) -> bool {
        let len = self.project.cues.len() as i32;
        let to = index as i32 + delta;
        if index as i32 >= len || to < 0 || to >= len {
            return false;
        }
        let to = to as usize;
        self.edit(move |p| p.cues.swap(index, to));
        self.selected_cue = Some(to);
        true
    }

    /// GO a cue by index.
    pub fn cue_go(&mut self, index: usize, now: f64) -> bool {
        let Some(cue) = self.project.cues.get(index).cloned() else {
            return false;
        };
        self.cues.go(&self.project, &cue, index, now);
        self.selected_cue = Some(index);
        true
    }

    /// GO a cue by id.
    pub fn cue_go_id(&mut self, id: &str, now: f64) -> bool {
        match self.project.cues.iter().position(|c| c.id == id) {
            Some(i) => self.cue_go(i, now),
            None => false,
        }
    }

    /// GO the next cue (Space).
    pub fn cue_go_next(&mut self, now: f64) -> Option<usize> {
        let i = self.cues.go_next(&self.project, now)?;
        self.selected_cue = Some(i);
        Some(i)
    }

    /// Advances running fades, applying their values. Returns how many parameters moved.
    pub fn tick_cues(&mut self, now: f64) -> usize {
        let patches = self.cues.tick(now);
        let mut n = 0;
        for patch in &patches {
            if self.project.apply_patch(patch).is_ok() {
                n += 1;
                if let Some(fb) = &self.feedback {
                    fb.send(patch);
                }
            }
        }
        n
    }

    /// Opens (or closes, with an empty name) the MIDI input port.
    pub fn set_midi_port(&mut self, name: Option<&str>) -> Result<(), String> {
        self.midi = None;
        let Some(name) = name else { return Ok(()) };
        match MidiListener::open(name) {
            Ok(l) => {
                self.status = format!("MIDI: {}", l.name());
                self.midi = Some(l);
                Ok(())
            }
            Err(e) => {
                self.status = e.to_string();
                Err(e.to_string())
            }
        }
    }

    /// Arms learn mode: the next MIDI message maps to `address`.
    pub fn arm_midi_learn(&mut self, address: &str, mode: MidiMode) {
        self.midi_learn = Some((address.to_string(), mode));
        self.status = format!("move a control to map it to {address}");
    }

    /// Handles one MIDI event: completes learn mode, or applies the mapped patches.
    pub fn handle_midi_event(&mut self, event: MidiEvent, now: f64) -> usize {
        self.last_midi = Some(event);
        if let Some((address, mode)) = self.midi_learn.take() {
            let key = event.key;
            self.edit(move |p| p.midi.learn(key, &address, mode));
            self.status = "mapped".into();
            return 0;
        }
        let patches = self.project.midi.translate(&event);
        patches
            .iter()
            .filter(|p| self.handle_control(p, now))
            .count()
    }

    /// Drains the MIDI port.
    pub fn poll_midi(&mut self, now: f64) -> usize {
        let events = match &self.midi {
            Some(m) => m.drain(),
            None => return 0,
        };
        events
            .into_iter()
            .map(|e| self.handle_midi_event(e, now))
            .sum()
    }

    /// Sets (or clears) the OSC feedback destination, `host:port`.
    pub fn set_feedback(&mut self, target: &str) -> Result<(), String> {
        self.feedback = None;
        if target.trim().is_empty() {
            return Ok(());
        }
        let fb = OscFeedback::new(target.trim()).map_err(|e| format!("{target}: {e}"))?;
        self.feedback = Some(fb);
        Ok(())
    }

    /// Starts (or stops with `0`) the phone remote on `port`.
    pub fn set_remote_port(&mut self, port: u16) -> Result<(), String> {
        self.remote = None;
        if port == 0 {
            return Ok(());
        }
        let server = RemoteServer::bind(port)?;
        self.remote = Some(server);
        Ok(())
    }

    /// URL a phone on the same network can open, when the remote runs.
    pub fn remote_url(&self) -> Option<String> {
        let r = self.remote.as_ref()?;
        let ip = lumen_control::web::lan_ip()
            .map_or_else(|| "127.0.0.1".to_string(), |ip| ip.to_string());
        Some(format!("http://{ip}:{}", r.addr().port()))
    }

    /// Drains remote commands and publishes the current state to the page.
    pub fn poll_remote(&mut self, now: f64) -> usize {
        let commands = match &self.remote {
            Some(r) => r.drain(),
            None => return 0,
        };
        let mut n = 0;
        for c in commands {
            let ok = match c {
                RemoteCommand::Param(p) => self.handle_control(&p, now),
                RemoteCommand::CueGo(None) => self.cue_go_next(now).is_some(),
                RemoteCommand::CueGo(Some(id)) => self.cue_go_id(&id, now),
            };
            if ok {
                n += 1;
            }
        }
        if let Some(r) = &self.remote {
            r.publish(RemoteState {
                name: self.project.name.clone(),
                params: capture(&self.project),
                cues: self
                    .project
                    .cues
                    .iter()
                    .map(|c| RemoteCue {
                        id: c.id.clone(),
                        name: c.name.clone(),
                        fade: c.fade_seconds,
                    })
                    .collect(),
                current: self.cues.current,
            });
        }
        n
    }
}
