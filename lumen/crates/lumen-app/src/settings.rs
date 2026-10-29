//! Per-user settings: recent projects and preferences that are not part of a show.
//!
//! Stored as TOML under the platform config directory (`LUMEN_CONFIG_DIR` overrides it, which
//! the tests use). Missing or unreadable settings fall back to defaults; saving is best effort.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Maximum entries in the recent list.
pub const MAX_RECENT: usize = 10;

/// User preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)]
pub struct Settings {
    /// Most recent first.
    pub recent: Vec<PathBuf>,
    /// OSC port used when none is given on the command line (`0` disables).
    pub osc_port: u16,
    /// Draw handles on the output windows while editing.
    pub edit_on_wall: bool,
    /// Show the alignment grid on the canvas.
    pub show_grid: bool,
    /// Snap handles to the grid, edges and centre.
    pub snap: bool,
    /// Grid divisions per axis.
    pub grid_divisions: u32,
    /// Seconds between autosaves.
    pub autosave_seconds: u64,
    /// Skip the welcome screen on launch.
    pub skip_welcome: bool,
    /// Explicit ffmpeg binary; empty means "search the PATH".
    pub ffmpeg_path: String,
    /// Phone remote port (`0` off).
    pub remote_port: u16,
    /// MIDI input port name to open at start; empty means none.
    pub midi_port: String,
    /// OSC feedback destination `host:port`; empty means off.
    pub osc_feedback: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            recent: Vec::new(),
            osc_port: 9000,
            edit_on_wall: true,
            show_grid: false,
            snap: true,
            grid_divisions: 10,
            autosave_seconds: 10,
            skip_welcome: false,
            ffmpeg_path: String::new(),
            remote_port: 0,
            midi_port: String::new(),
            osc_feedback: String::new(),
        }
    }
}

/// Where settings live.
pub fn settings_path() -> Option<PathBuf> {
    let dir = match std::env::var_os("LUMEN_CONFIG_DIR") {
        Some(d) => PathBuf::from(d),
        None => dirs::config_dir()?.join("lumen"),
    };
    Some(dir.join("settings.toml"))
}

impl Settings {
    /// Loads settings, falling back to defaults on any problem.
    pub fn load() -> Self {
        settings_path()
            .and_then(|p| Self::load_from(&p).ok())
            .unwrap_or_default()
    }

    /// Loads from a specific file.
    pub fn load_from(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        toml::from_str(&text).map_err(|e| e.to_string())
    }

    /// Saves to the default location. Errors are logged, never fatal.
    pub fn save(&self) {
        if let Some(p) = settings_path() {
            if let Err(e) = self.save_to(&p) {
                log::warn!("cannot save settings to {}: {e}", p.display());
            }
        }
    }

    /// Saves to a specific file, creating its folder.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    /// Moves `path` to the front of the recent list, dropping duplicates and the overflow.
    pub fn push_recent(&mut self, path: &Path) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(MAX_RECENT);
    }

    /// Recent entries that still exist on disk.
    pub fn existing_recent(&self) -> Vec<PathBuf> {
        self.recent.iter().filter(|p| p.exists()).cloned().collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_recent_rules() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg").join("settings.toml");
        let mut s = Settings::default();
        for i in 0..12 {
            s.push_recent(&dir.path().join(format!("show{i}.toml")));
        }
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert!(s.recent[0].ends_with("show11.toml"));
        s.push_recent(&dir.path().join("show5.toml"));
        assert!(s.recent[0].ends_with("show5.toml"));
        assert_eq!(
            s.recent
                .iter()
                .filter(|p| p.ends_with("show5.toml"))
                .count(),
            1
        );
        s.save_to(&path).unwrap();
        let back = Settings::load_from(&path).unwrap();
        assert_eq!(back, s);
        assert!(back.existing_recent().is_empty(), "none of the files exist");
        std::fs::write(dir.path().join("show5.toml"), "").unwrap();
        assert_eq!(back.existing_recent().len(), 1);
        assert!(Settings::load_from(&dir.path().join("missing.toml")).is_err());
        std::fs::write(&path, "not = [toml").unwrap();
        assert!(Settings::load_from(&path).is_err());
        assert!(Settings::default()
            .save_to(&dir.path().join("file-as-dir").join("x").join("s.toml"))
            .is_ok());
    }

    #[test]
    fn partial_files_fill_defaults() {
        let s: Settings = toml::from_str("osc_port = 1").unwrap();
        assert_eq!(s.osc_port, 1);
        assert!(s.snap && s.edit_on_wall);
        assert_eq!(s.grid_divisions, 10);
    }

    #[test]
    fn env_override_and_default_path() {
        let dir = tempfile::tempdir().unwrap();
        // Set the override only for this check; other tests do not touch the env.
        let prev = std::env::var_os("LUMEN_CONFIG_DIR");
        // SAFETY-free: std::env::set_var is safe in Rust 2021 single-threaded test context.
        std::env::set_var("LUMEN_CONFIG_DIR", dir.path());
        assert_eq!(settings_path().unwrap(), dir.path().join("settings.toml"));
        let mut s = Settings::load();
        s.osc_port = 7;
        s.save();
        assert_eq!(Settings::load().osc_port, 7);
        match prev {
            Some(v) => std::env::set_var("LUMEN_CONFIG_DIR", v),
            None => std::env::remove_var("LUMEN_CONFIG_DIR"),
        }
    }
}
