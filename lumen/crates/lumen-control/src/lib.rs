//! Remote control.
//!
//! An OSC message `/surface/wall/opacity 0.5` becomes a [`ParamPatch`] for the same address,
//! so anything that speaks OSC (`TouchOSC`, `QLab`, Max, a show controller) drives the same
//! parameters the editor and the tests use. Translation is a pure function; the UDP server is
//! a thin thread around it.

#![forbid(unsafe_code)]

use lumen_core::params::{ParamAddress, ParamPatch, ParamValue};
use rosc::{OscPacket, OscType};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

/// Errors from the control server.
#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    /// The UDP socket could not be bound.
    #[error("cannot bind OSC socket on {0}: {1}")]
    Bind(String, std::io::Error),
}

/// Converts one OSC argument into a parameter value.
///
/// Numbers of every width map to [`ParamValue::Number`]; strings and bools map to their
/// kinds; blobs, colours, MIDI, times and nested arrays are not meaningful here and return
/// `None`.
pub fn value_from_osc(arg: &OscType) -> Option<ParamValue> {
    match arg {
        OscType::Int(i) => Some(ParamValue::Number(f64::from(*i))),
        OscType::Float(f) => Some(ParamValue::Number(f64::from(*f))),
        OscType::Long(l) => Some(ParamValue::Number(*l as f64)),
        OscType::Double(d) => Some(ParamValue::Number(*d)),
        OscType::Bool(b) => Some(ParamValue::Bool(*b)),
        OscType::String(s) => Some(ParamValue::Text(s.clone())),
        OscType::Char(c) => Some(ParamValue::Text(c.to_string())),
        OscType::Nil => Some(ParamValue::Bool(false)),
        OscType::Inf => Some(ParamValue::Bool(true)),
        _ => None,
    }
}

/// Flattens a packet (message or nested bundle) into parameter patches.
///
/// A message with no arguments is treated as a trigger and becomes `true`, so a bare
/// `/show/blackout` from a button works. Messages whose address is not a valid parameter path
/// or whose first argument is unusable are skipped.
pub fn translate(packet: &OscPacket) -> Vec<ParamPatch> {
    let mut out = Vec::new();
    collect(packet, &mut out);
    out
}

fn collect(packet: &OscPacket, out: &mut Vec<ParamPatch>) {
    match packet {
        OscPacket::Bundle(b) => b.content.iter().for_each(|p| collect(p, out)),
        OscPacket::Message(m) => {
            let Ok(address) = ParamAddress::parse(&m.addr) else {
                log::debug!("ignoring OSC message with unusable address {:?}", m.addr);
                return;
            };
            let value = match m.args.first() {
                None => ParamValue::Bool(true),
                Some(arg) => {
                    let Some(v) = value_from_osc(arg) else {
                        log::debug!(
                            "ignoring OSC message {:?} with unsupported argument",
                            m.addr
                        );
                        return;
                    };
                    v
                }
            };
            out.push(ParamPatch { address, value });
        }
    }
}

/// Receives OSC over UDP on a background thread and hands out patches.
#[derive(Debug)]
pub struct OscServer {
    local_addr: SocketAddr,
    rx: Receiver<ParamPatch>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl OscServer {
    /// Binds `addr` (for example `0.0.0.0:9000`) and starts listening.
    pub fn bind(addr: impl ToSocketAddrs + std::fmt::Debug) -> Result<Self, ControlError> {
        let shown = format!("{addr:?}");
        let socket = UdpSocket::bind(addr).map_err(|e| ControlError::Bind(shown.clone(), e))?;
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .map_err(|e| ControlError::Bind(shown.clone(), e))?;
        let local_addr = socket
            .local_addr()
            .map_err(|e| ControlError::Bind(shown, e))?;
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("lumen-osc".into())
            .spawn(move || {
                let mut buf = vec![0u8; 65_536];
                while !stop_flag.load(Ordering::Relaxed) {
                    match socket.recv_from(&mut buf) {
                        Ok((len, from)) => match rosc::decoder::decode_udp(&buf[..len]) {
                            Ok((_, packet)) => {
                                for patch in translate(&packet) {
                                    if tx.send(patch).is_err() {
                                        return;
                                    }
                                }
                            }
                            Err(e) => log::debug!("bad OSC packet from {from}: {e}"),
                        },
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) => {}
                        Err(e) => {
                            log::warn!("OSC socket error: {e}");
                            std::thread::sleep(Duration::from_millis(50));
                        }
                    }
                }
            })
            .map_err(|e| ControlError::Bind("thread".into(), e))?;
        log::info!("OSC listening on {local_addr}");
        Ok(Self {
            local_addr,
            rx,
            stop,
            thread: Some(thread),
        })
    }

    /// Address the server is bound to; useful when port `0` was requested.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Takes every patch received since the last call.
    pub fn drain(&self) -> Vec<ParamPatch> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(p) => out.push(p),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }

    /// Blocks up to `timeout` for the next patch.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<ParamPatch> {
        self.rx.recv_timeout(timeout).ok()
    }
}

impl Drop for OscServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;
    use rosc::{OscBundle, OscMessage, OscTime};

    fn msg(addr: &str, args: Vec<OscType>) -> OscPacket {
        OscPacket::Message(OscMessage {
            addr: addr.into(),
            args,
        })
    }

    #[test]
    fn values_convert() {
        assert_eq!(
            value_from_osc(&OscType::Int(3)),
            Some(ParamValue::Number(3.0))
        );
        assert_eq!(
            value_from_osc(&OscType::Float(0.5)),
            Some(ParamValue::Number(0.5))
        );
        assert_eq!(
            value_from_osc(&OscType::Long(7)),
            Some(ParamValue::Number(7.0))
        );
        assert_eq!(
            value_from_osc(&OscType::Double(1.25)),
            Some(ParamValue::Number(1.25))
        );
        assert_eq!(
            value_from_osc(&OscType::Bool(true)),
            Some(ParamValue::Bool(true))
        );
        assert_eq!(
            value_from_osc(&OscType::String("x".into())),
            Some(ParamValue::Text("x".into()))
        );
        assert_eq!(
            value_from_osc(&OscType::Char('c')),
            Some(ParamValue::Text("c".into()))
        );
        assert_eq!(value_from_osc(&OscType::Nil), Some(ParamValue::Bool(false)));
        assert_eq!(value_from_osc(&OscType::Inf), Some(ParamValue::Bool(true)));
        assert_eq!(value_from_osc(&OscType::Blob(vec![1])), None);
        assert_eq!(
            value_from_osc(&OscType::Time(OscTime {
                seconds: 0,
                fractional: 0
            })),
            None
        );
    }

    #[test]
    fn translate_messages_and_bundles() {
        let patches = translate(&msg("/surface/wall/opacity", vec![OscType::Float(0.5)]));
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].address.to_string(), "/surface/wall/opacity");
        assert_eq!(patches[0].value, ParamValue::Number(0.5));

        let trigger = translate(&msg("/show/blackout", vec![]));
        assert_eq!(trigger[0].value, ParamValue::Bool(true));

        assert!(translate(&msg("bad address", vec![OscType::Int(1)])).is_empty());
        assert!(translate(&msg("/x", vec![OscType::Blob(vec![])])).is_empty());

        let bundle = OscPacket::Bundle(OscBundle {
            timetag: OscTime {
                seconds: 0,
                fractional: 0,
            },
            content: vec![
                msg("/a", vec![OscType::Int(1)]),
                OscPacket::Bundle(OscBundle {
                    timetag: OscTime {
                        seconds: 0,
                        fractional: 0,
                    },
                    content: vec![msg(
                        "/b",
                        vec![OscType::String("s".into()), OscType::Int(9)],
                    )],
                }),
            ],
        });
        let patches = translate(&bundle);
        assert_eq!(
            patches
                .iter()
                .map(|p| p.address.to_string())
                .collect::<Vec<_>>(),
            ["/a", "/b"]
        );
        assert_eq!(
            patches[1].value,
            ParamValue::Text("s".into()),
            "only the first argument counts"
        );
    }

    #[test]
    fn server_receives_real_packets() {
        let server = OscServer::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr();
        assert_ne!(addr.port(), 0);
        assert!(server.drain().is_empty());
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let packet =
            rosc::encoder::encode(&msg("/show/master", vec![OscType::Float(0.25)])).unwrap();
        client.send_to(&packet, addr).unwrap();
        let got = server.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(got.address.to_string(), "/show/master");
        assert_eq!(got.value, ParamValue::Number(0.25));
        // Garbage is ignored and does not kill the thread.
        client.send_to(b"\x00\x01garbage", addr).unwrap();
        client.send_to(&packet, addr).unwrap();
        assert!(server.recv_timeout(Duration::from_secs(5)).is_some());
        client.send_to(&packet, addr).unwrap();
        client.send_to(&packet, addr).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(server.drain().len(), 2);
        assert!(server.recv_timeout(Duration::from_millis(10)).is_none());
        drop(server);
    }

    #[test]
    fn bind_failure_is_reported() {
        let first = OscServer::bind("127.0.0.1:0").unwrap();
        let err = OscServer::bind("256.256.256.256:1").unwrap_err();
        assert!(err.to_string().contains("cannot bind OSC socket"));
        // Binding the same port twice fails on every platform we support.
        let dup = OscServer::bind(first.local_addr());
        assert!(dup.is_err() || cfg!(target_os = "windows"));
    }
}
