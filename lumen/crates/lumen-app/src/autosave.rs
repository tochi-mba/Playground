//! Crash-safe autosave.
//!
//! While a project is dirty the editor writes `<project>.autosave` on an interval, atomically.
//! On the next open, an autosave newer than the project file is offered for recovery and
//! deleted on a clean save.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Default interval between autosaves.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(10);

/// Sibling autosave path for a project file.
pub fn autosave_path(project: &Path) -> PathBuf {
    let mut name = project
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(".autosave");
    project.with_file_name(name)
}

/// Returns the autosave path when one exists and is newer than the project file (or the
/// project file does not exist at all).
pub fn recoverable(project: &Path) -> Option<PathBuf> {
    let auto = autosave_path(project);
    let auto_meta = std::fs::metadata(&auto).ok()?;
    match std::fs::metadata(project) {
        Ok(meta) => {
            let (Ok(a), Ok(p)) = (auto_meta.modified(), meta.modified()) else {
                return Some(auto);
            };
            (a > p).then_some(auto)
        }
        Err(_) => Some(auto),
    }
}

/// Removes the autosave after a clean save. Missing files are not an error.
pub fn discard(project: &Path) {
    let _ = std::fs::remove_file(autosave_path(project));
}

/// Timer that says when the next autosave is due.
#[derive(Debug, Clone)]
pub struct Autosave {
    interval: Duration,
    last: Instant,
}

impl Autosave {
    /// Creates a timer that first fires `interval` from now.
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: Instant::now(),
        }
    }

    /// True when an autosave should happen now. Only dirty projects are ever due.
    pub fn due(&self, dirty: bool, now: Instant) -> bool {
        dirty && now.duration_since(self.last) >= self.interval
    }

    /// Records that a save (auto or manual) happened at `now`.
    pub fn mark_saved(&mut self, now: Instant) {
        self.last = now;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use lumen_core::project::demo_project;

    #[test]
    fn paths() {
        assert_eq!(
            autosave_path(Path::new("/shows/a.lumen.toml")),
            PathBuf::from("/shows/a.lumen.toml.autosave")
        );
        assert_eq!(
            autosave_path(Path::new("a.toml")),
            PathBuf::from("a.toml.autosave")
        );
    }

    #[test]
    fn timer() {
        let mut a = Autosave::new(Duration::from_secs(10));
        let start = Instant::now();
        assert!(!a.due(true, start));
        assert!(!a.due(true, start + Duration::from_secs(9)));
        assert!(a.due(true, start + Duration::from_secs(11)));
        assert!(
            !a.due(false, start + Duration::from_secs(60)),
            "clean projects never autosave"
        );
        a.mark_saved(start + Duration::from_secs(11));
        assert!(!a.due(true, start + Duration::from_secs(15)));
        assert!(a.due(true, start + Duration::from_secs(22)));
    }

    #[test]
    fn recovery_detection() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("show.lumen.toml");
        assert_eq!(recoverable(&project), None, "nothing to recover");
        let auto = autosave_path(&project);
        demo_project().save(&auto).unwrap();
        assert_eq!(
            recoverable(&project),
            Some(auto.clone()),
            "autosave without a project file"
        );
        // Project saved after the autosave wins.
        std::thread::sleep(Duration::from_millis(20));
        demo_project().save(&project).unwrap();
        let newer = std::fs::metadata(&project).unwrap().modified().unwrap()
            > std::fs::metadata(&auto).unwrap().modified().unwrap();
        if newer {
            assert_eq!(recoverable(&project), None);
        }
        // Autosave written again after the project is offered.
        std::thread::sleep(Duration::from_millis(20));
        demo_project().save(&auto).unwrap();
        let newer = std::fs::metadata(&auto).unwrap().modified().unwrap()
            > std::fs::metadata(&project).unwrap().modified().unwrap();
        if newer {
            assert_eq!(recoverable(&project), Some(auto.clone()));
        }
        discard(&project);
        assert!(!auto.exists());
        discard(&project);
    }
}
