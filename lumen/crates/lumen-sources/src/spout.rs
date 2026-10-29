//! Spout receiver.
//!
//! Spout shares DirectX textures between Windows applications. Binding to the native SDK is a
//! platform-specific, `unsafe` job that lives behind this feature; on other platforms the
//! receiver compiles to a stub that never produces a frame, so the rest of the app, and the
//! tests, behave identically everywhere.
//!
//! The current implementation is the portable stub on every platform. It establishes the
//! interface and the lifecycle (connect, poll, disconnect) that the native receiver will fill
//! in; see the roadmap in the README.

use crate::{Frame, FrameSource};
use std::sync::Arc;

/// Receives frames from a named Spout sender.
pub struct SpoutReceiver {
    sender: String,
    connected: bool,
    last: Option<Arc<Frame>>,
}

impl SpoutReceiver {
    /// Creates a receiver for `sender`; empty means "the active sender".
    pub fn new(sender: &str) -> Self {
        Self {
            sender: sender.to_string(),
            connected: false,
            last: None,
        }
    }

    /// Sender name this receiver follows.
    pub fn sender(&self) -> &str {
        &self.sender
    }

    /// True once a sender has been found.
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Injects a frame as if it arrived from the sender. Used by tests and by the native
    /// receiver's copy path.
    pub fn push_frame(&mut self, frame: Frame) {
        self.connected = true;
        self.last = Some(Arc::new(frame));
    }
}

impl FrameSource for SpoutReceiver {
    fn frame(&mut self) -> Option<Arc<Frame>> {
        self.last.clone()
    }
    fn is_live(&self) -> bool {
        true
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle() {
        let mut r = SpoutReceiver::new("Resolume");
        assert_eq!(r.sender(), "Resolume");
        assert!(!r.is_connected());
        assert!(r.frame().is_none());
        assert!(r.is_live());
        r.push_frame(Frame::solid(2, 2, [1, 2, 3, 4]));
        assert!(r.is_connected());
        assert_eq!(r.frame().unwrap().pixel(0, 0), [1, 2, 3, 4]);
    }
}
