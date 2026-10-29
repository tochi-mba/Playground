//! MIDI messages mapped onto parameter addresses.
//!
//! The mapping table is part of the project so a controller layout travels with the show.
//! Parsing and translation are pure; the port layer lives in `lumen-control`.

use crate::params::{ParamAddress, ParamPatch, ParamValue};
use serde::{Deserialize, Serialize};

/// Which message a mapping listens to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MidiKey {
    /// Control change.
    Cc {
        /// Channel 0–15.
        channel: u8,
        /// Controller number.
        controller: u8,
    },
    /// Note on/off.
    Note {
        /// Channel 0–15.
        channel: u8,
        /// Note number.
        note: u8,
    },
}

/// A decoded MIDI message that mappings care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiEvent {
    /// What arrived.
    pub key: MidiKey,
    /// Controller value or note velocity, 0–127 (`0` for note off).
    pub value: u8,
}

/// Decodes a MIDI message (one message per call; running status not supported).
pub fn parse(bytes: &[u8]) -> Option<MidiEvent> {
    let (&status, rest) = bytes.split_first()?;
    let channel = status & 0x0F;
    match status & 0xF0 {
        0xB0 => Some(MidiEvent {
            key: MidiKey::Cc {
                channel,
                controller: *rest.first()? & 0x7F,
            },
            value: *rest.get(1)? & 0x7F,
        }),
        0x90 => Some(MidiEvent {
            key: MidiKey::Note {
                channel,
                note: *rest.first()? & 0x7F,
            },
            value: *rest.get(1)? & 0x7F,
        }),
        0x80 => Some(MidiEvent {
            key: MidiKey::Note {
                channel,
                note: *rest.first()? & 0x7F,
            },
            value: 0,
        }),
        _ => None,
    }
}

/// How a message turns into a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MidiMode {
    /// Scale 0–127 into `min..=max`.
    #[default]
    Fader,
    /// Send `true` on note on / value ≥ 64; nothing otherwise (cue GO, restart).
    Button,
    /// Send `true` on note on and `false` on note off / value < 64 (blackout, visibility).
    Switch,
}

/// One mapping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MidiMapping {
    /// Message to listen for.
    pub key: MidiKey,
    /// Parameter to drive.
    pub address: String,
    /// Interpretation.
    #[serde(default)]
    pub mode: MidiMode,
    /// Fader range start.
    #[serde(default)]
    pub min: f64,
    /// Fader range end.
    #[serde(default = "one")]
    pub max: f64,
}

fn one() -> f64 {
    1.0
}

/// The project's MIDI table.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MidiMap {
    /// Mappings in order; the first match for a key wins.
    #[serde(default)]
    pub mappings: Vec<MidiMapping>,
}

impl MidiMap {
    /// Adds or replaces the mapping for `key`.
    pub fn learn(&mut self, key: MidiKey, address: &str, mode: MidiMode) {
        self.mappings.retain(|m| m.key != key);
        self.mappings.push(MidiMapping {
            key,
            address: address.to_string(),
            mode,
            min: 0.0,
            max: 1.0,
        });
    }

    /// Removes mappings for an address.
    pub fn forget(&mut self, address: &str) {
        self.mappings.retain(|m| m.address != address);
    }

    /// Patches for an event. Buttons produce nothing on release.
    pub fn translate(&self, event: &MidiEvent) -> Vec<ParamPatch> {
        self.mappings
            .iter()
            .filter(|m| m.key == event.key)
            .filter_map(|m| {
                let address = ParamAddress::parse(&m.address).ok()?;
                let pressed = event.value >= 64
                    || (matches!(event.key, MidiKey::Note { .. }) && event.value > 0);
                let value = match m.mode {
                    MidiMode::Fader => {
                        ParamValue::Number(m.min + (m.max - m.min) * f64::from(event.value) / 127.0)
                    }
                    MidiMode::Button => {
                        if !pressed {
                            return None;
                        }
                        ParamValue::Bool(true)
                    }
                    MidiMode::Switch => ParamValue::Bool(pressed),
                };
                Some(ParamPatch { address, value })
            })
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn parsing() {
        assert_eq!(
            parse(&[0xB3, 7, 100]),
            Some(MidiEvent {
                key: MidiKey::Cc {
                    channel: 3,
                    controller: 7
                },
                value: 100
            })
        );
        assert_eq!(
            parse(&[0x90, 60, 127]),
            Some(MidiEvent {
                key: MidiKey::Note {
                    channel: 0,
                    note: 60
                },
                value: 127
            })
        );
        assert_eq!(
            parse(&[0x80, 60, 40]),
            Some(MidiEvent {
                key: MidiKey::Note {
                    channel: 0,
                    note: 60
                },
                value: 0
            })
        );
        assert_eq!(
            parse(&[0xB0, 0xFF, 0xFF]).unwrap().value,
            127,
            "data bytes are masked"
        );
        assert_eq!(parse(&[0xF8]), None, "clock is ignored");
        assert_eq!(parse(&[0xB0, 7]), None, "truncated");
        assert_eq!(parse(&[]), None);
    }

    #[test]
    fn translation_modes() {
        let mut map = MidiMap::default();
        map.learn(
            MidiKey::Cc {
                channel: 0,
                controller: 1,
            },
            "/show/master",
            MidiMode::Fader,
        );
        map.learn(
            MidiKey::Note {
                channel: 0,
                note: 36,
            },
            "/cue/go",
            MidiMode::Button,
        );
        map.learn(
            MidiKey::Note {
                channel: 0,
                note: 37,
            },
            "/show/blackout",
            MidiMode::Switch,
        );
        map.learn(
            MidiKey::Cc {
                channel: 0,
                controller: 2,
            },
            "bad address",
            MidiMode::Fader,
        );
        let fader = map.translate(&parse(&[0xB0, 1, 127]).unwrap());
        assert_eq!(fader.len(), 1);
        assert_eq!(fader[0].value, ParamValue::Number(1.0));
        assert_eq!(
            map.translate(&parse(&[0xB0, 1, 0]).unwrap())[0].value,
            ParamValue::Number(0.0)
        );
        assert!(
            map.translate(&parse(&[0xB0, 2, 10]).unwrap()).is_empty(),
            "unparseable address is dropped"
        );
        assert_eq!(
            map.translate(&parse(&[0x90, 36, 100]).unwrap())[0].value,
            ParamValue::Bool(true)
        );
        assert!(
            map.translate(&parse(&[0x80, 36, 0]).unwrap()).is_empty(),
            "button release sends nothing"
        );
        assert_eq!(
            map.translate(&parse(&[0x90, 37, 1]).unwrap())[0].value,
            ParamValue::Bool(true)
        );
        assert_eq!(
            map.translate(&parse(&[0x80, 37, 0]).unwrap())[0].value,
            ParamValue::Bool(false)
        );
        assert!(
            map.translate(&parse(&[0xB1, 1, 127]).unwrap()).is_empty(),
            "other channel"
        );
        // Re-learning a key replaces it; forgetting an address removes it.
        map.learn(
            MidiKey::Cc {
                channel: 0,
                controller: 1,
            },
            "/surface/logo/opacity",
            MidiMode::Fader,
        );
        assert_eq!(
            map.mappings
                .iter()
                .filter(|m| matches!(m.key, MidiKey::Cc { controller: 1, .. }))
                .count(),
            1
        );
        map.forget("/cue/go");
        assert!(!map.mappings.iter().any(|m| m.address == "/cue/go"));
        // Custom range.
        let i = map
            .mappings
            .iter()
            .position(|m| m.address == "/surface/logo/opacity")
            .unwrap();
        map.mappings[i].min = 2.0;
        map.mappings[i].max = 4.0;
        let key = map.mappings[i].key;
        let ev = MidiEvent { key, value: 127 };
        assert_eq!(map.translate(&ev)[0].value, ParamValue::Number(4.0));
        // Serde round trip with defaults.
        let text = toml::to_string(&map).unwrap();
        let back: MidiMap = toml::from_str(&text).unwrap();
        assert_eq!(back, map);
        let minimal: MidiMapping =
            toml::from_str("address = \"/x\"\n[key]\nkind = \"cc\"\nchannel = 0\ncontroller = 5\n")
                .unwrap();
        assert_eq!(minimal.mode, MidiMode::Fader);
        assert_eq!((minimal.min, minimal.max), (0.0, 1.0));
    }
}
