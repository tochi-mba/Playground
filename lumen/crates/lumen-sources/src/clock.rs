//! Transport clock shared by every playable source.

use lumen_core::project::Playback;

/// Tracks a playhead position in seconds from the frame clock and a [`Playback`].
#[derive(Debug, Clone, PartialEq)]
pub struct Transport {
    /// Current playhead in seconds.
    pub position: f64,
    /// Transport settings in force.
    pub playback: Playback,
    last_time: Option<f64>,
    last_token: u32,
}

impl Transport {
    /// Starts at position zero with these settings.
    pub fn new(playback: Playback) -> Self {
        Self {
            position: 0.0,
            playback,
            last_time: None,
            last_token: playback.restart_token,
        }
    }

    /// Adopts new settings; a bumped restart token rewinds to zero.
    pub fn set_playback(&mut self, playback: Playback) -> bool {
        let restart = playback.restart_token != self.last_token;
        self.last_token = playback.restart_token;
        self.playback = playback;
        if restart {
            self.position = 0.0;
        }
        restart
    }

    /// Advances to frame-clock time `t` (seconds). Returns true when the position moved.
    pub fn advance(&mut self, t: f64) -> bool {
        let dt = match self.last_time {
            Some(prev) if t >= prev => t - prev,
            _ => 0.0,
        };
        self.last_time = Some(t);
        if self.playback.paused || dt <= 0.0 {
            return false;
        }
        self.position += dt * self.playback.speed.max(0.0);
        true
    }

    /// Wraps or clamps the position into `[0, duration)`. Returns true when it wrapped,
    /// meaning the source should start over.
    pub fn wrap(&mut self, duration: f64) -> bool {
        if duration <= 0.0 {
            self.position = 0.0;
            return false;
        }
        if self.position < duration {
            return false;
        }
        if self.playback.looping {
            self.position %= duration;
            true
        } else {
            self.position = duration - 1e-6;
            false
        }
    }

    /// Rewinds to the start.
    pub fn rewind(&mut self) {
        self.position = 0.0;
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn advances_with_speed_and_pause() {
        let mut t = Transport::new(Playback::default());
        assert!(!t.advance(1.0), "first call only sets the reference time");
        assert!(t.advance(1.5));
        assert_eq!(t.position, 0.5);
        t.playback.speed = 2.0;
        t.advance(2.0);
        assert_eq!(t.position, 1.5);
        t.playback.paused = true;
        assert!(!t.advance(5.0));
        assert_eq!(t.position, 1.5);
        t.playback.paused = false;
        assert!(!t.advance(4.0), "time going backwards is ignored");
        assert!(t.advance(4.5));
        assert_eq!(t.position, 2.5);
    }

    #[test]
    fn wrapping_and_restart() {
        let mut t = Transport::new(Playback::default());
        t.position = 7.5;
        assert!(t.wrap(5.0));
        assert_eq!(t.position, 2.5);
        assert!(!t.wrap(5.0));
        t.playback.looping = false;
        t.position = 9.0;
        assert!(!t.wrap(5.0));
        assert!(t.position < 5.0 && t.position > 4.9);
        t.position = 3.0;
        assert!(!t.wrap(0.0));
        assert_eq!(t.position, 0.0);
        t.position = 3.0;
        let mut pb = Playback::default();
        assert!(!t.set_playback(pb));
        assert_eq!(t.position, 3.0);
        pb.restart_token = 1;
        assert!(t.set_playback(pb));
        assert_eq!(t.position, 0.0);
        t.position = 2.0;
        t.rewind();
        assert_eq!(t.position, 0.0);
    }
}
