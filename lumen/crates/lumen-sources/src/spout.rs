//! Spout receiver.
//!
//! Spout shares DirectX textures between Windows applications (Resolume, `TouchDesigner`, OBS,
//! Unity and most VJ tools send it). On Windows this module drives the vendored Spout2 SDK
//! through the `spout2` crate's DirectX 11 receiver, which owns its own D3D11 device and hands
//! back CPU pixels, so no `unsafe` code and no graphics interop is needed here; the frame then
//! takes the same path as every other source. On other platforms, and in tests, the receiver
//! is a stub that never connects but keeps the same lifecycle, so the rest of the app behaves
//! identically everywhere.

use crate::{Frame, FrameSource};
use std::sync::Arc;

/// Receives frames from a named Spout sender.
pub struct SpoutReceiver {
    sender: String,
    connected: bool,
    last: Option<Arc<Frame>>,
    #[cfg(windows)]
    native: Option<native::Native>,
}

impl SpoutReceiver {
    /// Creates a receiver for `sender`; empty means "the active sender".
    pub fn new(sender: &str) -> Self {
        Self {
            sender: sender.to_string(),
            connected: false,
            last: None,
            #[cfg(windows)]
            native: native::Native::open(sender),
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

    /// Polls the native receiver.
    #[cfg(windows)]
    fn poll_native(&mut self) {
        if let Some(n) = &mut self.native {
            match n.poll() {
                Some(Some(frame)) => self.push_frame(frame),
                Some(None) => {}
                None => self.connected = false,
            }
        }
    }

    /// No native receiver on this platform.
    #[cfg(not(windows))]
    fn poll_native(&mut self) {
        let _ = &self.connected;
    }
}

impl FrameSource for SpoutReceiver {
    fn frame(&mut self, _t: f64) -> Option<Arc<Frame>> {
        self.poll_native();
        self.last.clone()
    }
    fn is_live(&self) -> bool {
        true
    }
}

/// DXGI format codes a sender can use; anything else is treated as BGRA, Spout's default.
const DXGI_FORMAT_R8G8B8A8_UNORM: u32 = 28;
const DXGI_FORMAT_R8G8B8A8_UNORM_SRGB: u32 = 29;

/// Converts the pixels Spout copies out of its staging texture into straight RGBA.
///
/// Spout copies the sender's texture bytes as they are, and most senders use
/// `DXGI_FORMAT_B8G8R8A8_UNORM`, so red and blue are swapped unless the sender's format says
/// it is already RGBA.
pub fn to_rgba(pixels: &mut [u8], sender_format: u32) {
    if matches!(
        sender_format,
        DXGI_FORMAT_R8G8B8A8_UNORM | DXGI_FORMAT_R8G8B8A8_UNORM_SRGB
    ) {
        return;
    }
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
}

#[cfg(windows)]
mod native {
    //! The Windows receiver. Never compiled in this repository's Linux CI beyond `cargo check`
    //! on the stub crate, so keep it small and obviously correct.
    use super::{to_rgba, Frame};

    pub struct Native {
        rx: spout2::dx::Receiver,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    }

    impl Native {
        pub fn open(sender: &str) -> Option<Self> {
            let name = (!sender.trim().is_empty()).then(|| sender.trim());
            match spout2::dx::Receiver::new(name) {
                Ok(rx) => Some(Self {
                    rx,
                    width: 64,
                    height: 64,
                    pixels: vec![0; 64 * 64 * 4],
                }),
                Err(e) => {
                    log::warn!("spout: {e}");
                    None
                }
            }
        }

        /// `None` when no sender is connected, `Some(None)` when connected without a new
        /// frame, `Some(Some(frame))` for a new frame.
        pub fn poll(&mut self) -> Option<Option<Frame>> {
            let connected = self
                .rx
                .receive_image(&mut self.pixels, self.width, self.height, false, false)
                .unwrap_or(false);
            if self.rx.is_updated() {
                let (w, h) = self.rx.sender_size();
                self.width = w.max(1);
                self.height = h.max(1);
                self.pixels = vec![0; (self.width * self.height * 4) as usize];
                return Some(None);
            }
            if !connected {
                return None;
            }
            if !self.rx.is_frame_new() {
                return Some(None);
            }
            let mut rgba = self.pixels.clone();
            to_rgba(&mut rgba, self.rx.sender_format());
            Some(Some(Frame {
                width: self.width,
                height: self.height,
                rgba,
            }))
        }
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
        assert!(r.frame(0.0).is_none());
        assert!(r.is_live());
        r.push_frame(Frame::solid(2, 2, [1, 2, 3, 4]));
        assert!(r.is_connected());
        assert_eq!(r.frame(0.0).unwrap().pixel(0, 0), [1, 2, 3, 4]);
        let mut active = SpoutReceiver::new("");
        assert_eq!(active.sender(), "");
        assert!(active.frame(0.0).is_none());
    }

    #[test]
    fn bgra_senders_are_swapped_and_rgba_senders_are_not() {
        let mut px = [1, 2, 3, 4, 5, 6, 7, 8];
        to_rgba(&mut px, DXGI_FORMAT_R8G8B8A8_UNORM);
        assert_eq!(px, [1, 2, 3, 4, 5, 6, 7, 8]);
        to_rgba(&mut px, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB);
        assert_eq!(px, [1, 2, 3, 4, 5, 6, 7, 8]);
        to_rgba(&mut px, 87);
        assert_eq!(px, [3, 2, 1, 4, 7, 6, 5, 8]);
        to_rgba(&mut px, 0);
        assert_eq!(px, [1, 2, 3, 4, 5, 6, 7, 8]);
    }
}
