//! MIDI input through `midir`: a thin port layer over the pure mapping in `lumen_core`.

use lumen_core::midimap::{parse, MidiEvent};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// Errors from the MIDI layer.
#[derive(Debug, thiserror::Error)]
pub enum MidiError {
    /// The system MIDI service could not be reached.
    #[error("MIDI unavailable: {0}")]
    Init(String),
    /// No port with that name or index.
    #[error("MIDI port {0:?} not found")]
    NoPort(String),
    /// The port refused the connection.
    #[error("cannot open MIDI port {0:?}: {1}")]
    Connect(String, String),
}

/// Names of the input ports present right now.
pub fn ports() -> Result<Vec<String>, MidiError> {
    let input = midir::MidiInput::new("lumen").map_err(|e| MidiError::Init(e.to_string()))?;
    Ok(input
        .ports()
        .iter()
        .map(|p| input.port_name(p).unwrap_or_else(|_| "unknown".into()))
        .collect())
}

/// An open input port delivering decoded events.
pub struct MidiListener {
    name: String,
    rx: Receiver<MidiEvent>,
    _connection: midir::MidiInputConnection<()>,
}

impl std::fmt::Debug for MidiListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MidiListener")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl MidiListener {
    /// Opens the port whose name contains `name` (case-insensitive); an empty name opens the
    /// first port.
    pub fn open(name: &str) -> Result<Self, MidiError> {
        let mut input =
            midir::MidiInput::new("lumen").map_err(|e| MidiError::Init(e.to_string()))?;
        input.ignore(midir::Ignore::All);
        let ports = input.ports();
        let wanted = name.trim().to_ascii_lowercase();
        let port = ports
            .iter()
            .find(|p| {
                let n = input.port_name(p).unwrap_or_default().to_ascii_lowercase();
                wanted.is_empty() || n.contains(&wanted)
            })
            .cloned()
            .ok_or_else(|| MidiError::NoPort(name.to_string()))?;
        let port_name = input.port_name(&port).unwrap_or_else(|_| name.to_string());
        let (tx, rx) = channel();
        let connection = input
            .connect(
                &port,
                "lumen-in",
                move |_stamp, bytes, ()| {
                    if let Some(ev) = parse(bytes) {
                        let _ = tx.send(ev);
                    }
                },
                (),
            )
            .map_err(|e| MidiError::Connect(port_name.clone(), e.to_string()))?;
        log::info!("MIDI listening on {port_name}");
        Ok(Self {
            name: port_name,
            rx,
            _connection: connection,
        })
    }

    /// Port name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Events received since the last call.
    pub fn drain(&self) -> Vec<MidiEvent> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(e) => out.push(e),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn port_listing_and_missing_port() {
        // CI machines rarely have MIDI hardware; both outcomes are acceptable as long as the
        // errors are the typed ones.
        match ports() {
            Ok(list) => {
                let err = MidiListener::open("no such port name 12345").unwrap_err();
                assert!(matches!(err, MidiError::NoPort(_)), "{err}");
                assert!(err.to_string().contains("not found"));
                let _ = list;
            }
            Err(e) => assert!(matches!(e, MidiError::Init(_)), "{e}"),
        }
        assert!(MidiError::Connect("p".into(), "x".into())
            .to_string()
            .contains("cannot open"));
    }
}
