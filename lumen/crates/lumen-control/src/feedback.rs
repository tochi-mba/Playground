//! OSC feedback: every parameter change is echoed to a controller so its faders follow.

use lumen_core::params::{ParamPatch, ParamValue};
use rosc::{OscMessage, OscPacket, OscType};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};

/// Sends parameter values to one OSC destination.
#[derive(Debug)]
pub struct OscFeedback {
    socket: UdpSocket,
    target: SocketAddr,
}

impl OscFeedback {
    /// Creates a sender to `target` (for example `192.168.1.20:9001`).
    pub fn new(target: impl ToSocketAddrs) -> std::io::Result<Self> {
        let target = target
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| std::io::Error::other("no address"))?;
        let socket = UdpSocket::bind(if target.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        })?;
        Ok(Self { socket, target })
    }

    /// Destination.
    pub fn target(&self) -> SocketAddr {
        self.target
    }

    /// Encodes and sends one patch. Errors are logged, never fatal: feedback must not stop a show.
    pub fn send(&self, patch: &ParamPatch) {
        let packet = OscPacket::Message(OscMessage {
            addr: patch.address.to_string(),
            args: vec![to_osc(&patch.value)],
        });
        match rosc::encoder::encode(&packet) {
            Ok(bytes) => {
                if let Err(e) = self.socket.send_to(&bytes, self.target) {
                    log::debug!("OSC feedback to {}: {e}", self.target);
                }
            }
            Err(e) => log::debug!("OSC feedback encode: {e}"),
        }
    }
}

/// Parameter value as an OSC argument.
pub fn to_osc(v: &ParamValue) -> OscType {
    match v {
        ParamValue::Bool(b) => OscType::Bool(*b),
        ParamValue::Number(n) => OscType::Float(*n as f32),
        ParamValue::Text(t) => OscType::String(t.clone()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use lumen_core::params::ParamAddress;

    #[test]
    fn echoes_patches_over_loopback() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let fb = OscFeedback::new(receiver.local_addr().unwrap()).unwrap();
        assert_eq!(fb.target(), receiver.local_addr().unwrap());
        fb.send(&ParamPatch {
            address: ParamAddress::parse("/show/master").unwrap(),
            value: ParamValue::Number(0.5),
        });
        let mut buf = [0u8; 512];
        let (n, _) = receiver.recv_from(&mut buf).unwrap();
        let (_, packet) = rosc::decoder::decode_udp(&buf[..n]).unwrap();
        let OscPacket::Message(m) = packet else {
            panic!()
        };
        assert_eq!(m.addr, "/show/master");
        assert_eq!(m.args, vec![OscType::Float(0.5)]);
        assert_eq!(to_osc(&ParamValue::Bool(true)), OscType::Bool(true));
        assert_eq!(
            to_osc(&ParamValue::Text("x".into())),
            OscType::String("x".into())
        );
        assert!(OscFeedback::new("not an address").is_err());
    }
}
