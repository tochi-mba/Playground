//! The phone remote: a tiny HTTP server with one static page and a JSON API.
//!
//! The app publishes a [`RemoteState`] snapshot each frame; the server answers `GET
//! /api/state` from it and turns `POST /api/param` and `POST /api/cue/go` into
//! [`RemoteCommand`]s the app drains. Vanilla JS, no CDN, works on any phone browser.

use lumen_core::params::{ParamAddress, ParamPatch, ParamValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// The page served at `/`.
pub const PAGE: &str = include_str!("../assets/remote.html");

/// What the page shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoteState {
    /// Show name.
    pub name: String,
    /// Live parameters.
    pub params: BTreeMap<String, ParamValue>,
    /// Cues in order.
    pub cues: Vec<RemoteCue>,
    /// Index of the current cue.
    pub current: Option<usize>,
}

/// A cue as the page sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteCue {
    /// Id.
    pub id: String,
    /// Name.
    pub name: String,
    /// Fade seconds.
    pub fade: f64,
}

/// Something the page asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum RemoteCommand {
    /// Set a parameter.
    Param(ParamPatch),
    /// GO a cue by id, or the next one when `None`.
    CueGo(Option<String>),
}

#[derive(Debug, Deserialize)]
struct ParamBody {
    address: String,
    value: ParamValue,
}

#[derive(Debug, Deserialize, Default)]
struct CueBody {
    #[serde(default)]
    id: Option<String>,
}

/// Parses a request into a command. Pure, so the routing is unit-tested.
pub fn route(
    method: &str,
    url: &str,
    body: &str,
    state: &RemoteState,
) -> (u16, &'static str, String) {
    let path = url.split('?').next().unwrap_or("");
    match (method, path) {
        ("GET", "/" | "/index.html") => (200, "text/html; charset=utf-8", PAGE.to_string()),
        ("GET", "/api/state") => (
            200,
            "application/json",
            serde_json::to_string(state).unwrap_or_else(|_| "{}".into()),
        ),
        ("POST", "/api/param") => match serde_json::from_str::<ParamBody>(body) {
            Ok(b) => match ParamAddress::parse(&b.address) {
                Ok(_) => (200, "application/json", "{\"ok\":true}".into()),
                Err(e) => (
                    400,
                    "application/json",
                    format!("{{\"error\":{:?}}}", e.to_string()),
                ),
            },
            Err(e) => (
                400,
                "application/json",
                format!("{{\"error\":{:?}}}", e.to_string()),
            ),
        },
        ("POST", "/api/cue/go") => {
            match serde_json::from_str::<CueBody>(if body.trim().is_empty() { "{}" } else { body })
            {
                Ok(_) => (200, "application/json", "{\"ok\":true}".into()),
                Err(e) => (
                    400,
                    "application/json",
                    format!("{{\"error\":{:?}}}", e.to_string()),
                ),
            }
        }
        _ => (404, "text/plain", "not found".into()),
    }
}

/// Commands a request produces (only for accepted POSTs).
pub fn commands(method: &str, url: &str, body: &str) -> Vec<RemoteCommand> {
    let path = url.split('?').next().unwrap_or("");
    match (method, path) {
        ("POST", "/api/param") => serde_json::from_str::<ParamBody>(body)
            .ok()
            .and_then(|b| {
                ParamAddress::parse(&b.address).ok().map(|a| {
                    RemoteCommand::Param(ParamPatch {
                        address: a,
                        value: b.value,
                    })
                })
            })
            .into_iter()
            .collect(),
        ("POST", "/api/cue/go") => {
            serde_json::from_str::<CueBody>(if body.trim().is_empty() { "{}" } else { body })
                .ok()
                .map(|b| RemoteCommand::CueGo(b.id))
                .into_iter()
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The running server.
pub struct RemoteServer {
    addr: SocketAddr,
    state: Arc<Mutex<RemoteState>>,
    rx: Receiver<RemoteCommand>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for RemoteServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteServer")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl RemoteServer {
    /// Binds `0.0.0.0:port` (`0` picks a free port) and serves on a background thread.
    pub fn bind(port: u16) -> Result<Self, String> {
        let server = tiny_http::Server::http(("0.0.0.0", port)).map_err(|e| e.to_string())?;
        let addr = match server.server_addr() {
            tiny_http::ListenAddr::IP(a) => a,
            #[cfg(unix)]
            tiny_http::ListenAddr::Unix(_) => return Err("unix socket".into()),
        };
        let state = Arc::new(Mutex::new(RemoteState::default()));
        let (tx, rx): (Sender<RemoteCommand>, Receiver<RemoteCommand>) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (state_t, stop_t) = (Arc::clone(&state), Arc::clone(&stop));
        let thread = std::thread::Builder::new()
            .name("lumen-remote".into())
            .spawn(move || {
                while !stop_t.load(Ordering::Relaxed) {
                    let Ok(Some(mut request)) = server.recv_timeout(Duration::from_millis(100))
                    else {
                        continue;
                    };
                    let mut body = String::new();
                    let _ = request
                        .as_reader()
                        .take(64 * 1024)
                        .read_to_string(&mut body);
                    let method = request.method().as_str().to_ascii_uppercase();
                    let url = request.url().to_string();
                    let snapshot = state_t.lock().map(|s| s.clone()).unwrap_or_default();
                    let (status, mime, text) = route(&method, &url, &body, &snapshot);
                    if status == 200 {
                        for c in commands(&method, &url, &body) {
                            let _ = tx.send(c);
                        }
                    }
                    let header = tiny_http::Header::from_bytes("Content-Type", mime).ok();
                    let mut response =
                        tiny_http::Response::from_string(text).with_status_code(status);
                    if let Some(h) = header {
                        response = response.with_header(h);
                    }
                    let _ = request.respond(response);
                }
            })
            .map_err(|e| e.to_string())?;
        log::info!("phone remote on http://{addr}");
        Ok(Self {
            addr,
            state,
            rx,
            stop,
            thread: Some(thread),
        })
    }

    /// Bound address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Publishes a new snapshot for the page.
    pub fn publish(&self, state: RemoteState) {
        if let Ok(mut s) = self.state.lock() {
            *s = state;
        }
    }

    /// Commands received since the last call.
    pub fn drain(&self) -> Vec<RemoteCommand> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(c) => out.push(c),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }
}

impl Drop for RemoteServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The LAN address other devices can reach this machine on, best effort.
pub fn lan_ip() -> Option<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    // Connecting a UDP socket sends nothing; it only selects the outbound interface.
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}

/// QR code modules for a URL (true = dark), row-major with its width.
pub fn qr_modules(text: &str) -> Option<(usize, Vec<bool>)> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let w = code.width();
    Some((
        w,
        code.to_colors()
            .into_iter()
            .map(|c| c == qrcode::Color::Dark)
            .collect(),
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpStream;

    fn http(addr: SocketAddr, method: &str, path: &str, body: &str) -> (u16, String) {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(s, "{method} {path} HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        let mut text = String::new();
        s.read_to_string(&mut text).unwrap();
        let status: u16 = text.split_whitespace().nth(1).unwrap().parse().unwrap();
        let body = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, body)
    }

    #[test]
    fn routing_is_pure() {
        let mut state = RemoteState::default();
        state
            .params
            .insert("/show/master".into(), ParamValue::Number(1.0));
        assert_eq!(route("GET", "/", "", &state).0, 200);
        assert!(route("GET", "/?x=1", "", &state)
            .2
            .contains("<title>lumen remote"));
        let (status, mime, json) = route("GET", "/api/state", "", &state);
        assert_eq!((status, mime), (200, "application/json"));
        assert!(json.contains("/show/master"));
        assert_eq!(
            route(
                "POST",
                "/api/param",
                r#"{"address":"/show/master","value":0.5}"#,
                &state
            )
            .0,
            200
        );
        assert_eq!(
            route(
                "POST",
                "/api/param",
                r#"{"address":"bad","value":0.5}"#,
                &state
            )
            .0,
            400
        );
        assert_eq!(route("POST", "/api/param", "not json", &state).0, 400);
        assert_eq!(route("POST", "/api/cue/go", "", &state).0, 200);
        assert_eq!(route("POST", "/api/cue/go", r#"{"id":"x"}"#, &state).0, 200);
        assert_eq!(route("POST", "/api/cue/go", "[1", &state).0, 400);
        assert_eq!(route("GET", "/nope", "", &state).0, 404);
        assert_eq!(
            commands(
                "POST",
                "/api/param",
                r#"{"address":"/show/blackout","value":true}"#
            ),
            vec![RemoteCommand::Param(ParamPatch {
                address: ParamAddress::parse("/show/blackout").unwrap(),
                value: ParamValue::Bool(true)
            })]
        );
        assert_eq!(
            commands("POST", "/api/cue/go", ""),
            vec![RemoteCommand::CueGo(None)]
        );
        assert_eq!(
            commands("POST", "/api/cue/go", r#"{"id":"c1"}"#),
            vec![RemoteCommand::CueGo(Some("c1".into()))]
        );
        assert!(commands("POST", "/api/param", "junk").is_empty());
        assert!(commands("GET", "/api/state", "").is_empty());
    }

    #[test]
    fn server_round_trip() {
        let server = RemoteServer::bind(0).unwrap();
        let addr = SocketAddr::from(([127, 0, 0, 1], server.addr().port()));
        let mut state = RemoteState {
            name: "Test show".into(),
            ..Default::default()
        };
        state
            .params
            .insert("/show/master".into(), ParamValue::Number(0.75));
        state.cues.push(RemoteCue {
            id: "c1".into(),
            name: "One".into(),
            fade: 2.0,
        });
        server.publish(state);
        let (status, page) = http(addr, "GET", "/", "");
        assert_eq!(status, 200);
        assert!(page.contains("lumen remote"));
        let (status, json) = http(addr, "GET", "/api/state", "");
        assert_eq!(status, 200);
        let parsed: RemoteState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "Test show");
        assert_eq!(parsed.cues[0].id, "c1");
        assert_eq!(
            http(
                addr,
                "POST",
                "/api/param",
                r#"{"address":"/show/master","value":0.25}"#
            )
            .0,
            200
        );
        assert_eq!(http(addr, "POST", "/api/cue/go", r#"{"id":"c1"}"#).0, 200);
        assert_eq!(http(addr, "POST", "/api/param", "nope").0, 400);
        assert_eq!(http(addr, "GET", "/missing", "").0, 404);
        std::thread::sleep(Duration::from_millis(100));
        let cmds = server.drain();
        assert_eq!(cmds.len(), 2, "{cmds:?}");
        assert!(matches!(&cmds[0], RemoteCommand::Param(p) if p.value == ParamValue::Number(0.25)));
        assert_eq!(cmds[1], RemoteCommand::CueGo(Some("c1".into())));
        assert!(server.drain().is_empty());
        assert!(format!("{server:?}").contains("RemoteServer"));
        drop(server);
        let taken = RemoteServer::bind(0).unwrap();
        assert!(RemoteServer::bind(taken.addr().port()).is_err());
    }

    #[test]
    fn helpers() {
        let (w, modules) = qr_modules("http://192.168.1.2:8080").unwrap();
        assert!(w >= 21);
        assert_eq!(modules.len(), w * w);
        assert!(modules.iter().any(|m| *m) && modules.iter().any(|m| !*m));
        assert!(
            qr_modules(&"x".repeat(10_000)).is_none(),
            "too long for any QR version"
        );
        let _ = lan_ip();
    }
}
