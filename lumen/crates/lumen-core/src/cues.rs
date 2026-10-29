//! Cues: stored looks, and the runner that crossfades between them.
//!
//! A cue is a snapshot of live parameters (opacities, visibility, levels, blackout,
//! transport). GO fades from the current values to the cue's values over the cue's fade time:
//! numbers interpolate, booleans switch half way. The runner is pure: it is fed the clock and
//! yields patches, so it is tested without a window.

use crate::params::{ParamAddress, ParamPatch, ParamValue};
use crate::project::Project;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A stored look.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cue {
    /// Unique id.
    pub id: String,
    /// Human name shown in the list.
    #[serde(default)]
    pub name: String,
    /// Crossfade duration in seconds.
    #[serde(default)]
    pub fade_seconds: f64,
    /// Parameter values, keyed by address.
    #[serde(default)]
    pub values: BTreeMap<String, ParamValue>,
}

/// Addresses a cue captures, for one project.
pub fn live_addresses(project: &Project) -> Vec<String> {
    let mut out = vec!["/show/blackout".to_string(), "/show/master".to_string()];
    for s in &project.surfaces {
        out.push(format!("/surface/{}/opacity", s.id));
        out.push(format!("/surface/{}/visible", s.id));
    }
    for o in &project.outputs {
        for p in ["enabled", "brightness", "contrast", "gamma"] {
            out.push(format!("/output/{}/{p}", o.id));
        }
    }
    for s in &project.sources {
        if s.kind.playback().is_some() {
            for p in ["play", "loop", "speed"] {
                out.push(format!("/source/{}/{p}", s.id));
            }
        }
    }
    out
}

/// Snapshot of every live parameter.
pub fn capture(project: &Project) -> BTreeMap<String, ParamValue> {
    live_addresses(project)
        .into_iter()
        .filter_map(|a| {
            let addr = ParamAddress::parse(&a).ok()?;
            project.get_param(&addr).map(|v| (a, v))
        })
        .collect()
}

/// Interpolates between two values at `t ∈ [0,1]`.
pub fn blend_value(from: &ParamValue, to: &ParamValue, t: f64) -> ParamValue {
    match (from, to) {
        (ParamValue::Number(a), ParamValue::Number(b)) => {
            ParamValue::Number(a + (b - a) * t.clamp(0.0, 1.0))
        }
        (ParamValue::Bool(a), ParamValue::Bool(b)) => {
            ParamValue::Bool(if t >= 0.5 { *b } else { *a })
        }
        (_, other) => other.clone(),
    }
}

/// A fade in progress.
#[derive(Debug, Clone, PartialEq)]
pub struct Fade {
    /// Values when the fade started.
    pub from: BTreeMap<String, ParamValue>,
    /// Target values.
    pub to: BTreeMap<String, ParamValue>,
    /// Clock time the fade started.
    pub start: f64,
    /// Seconds.
    pub duration: f64,
}

impl Fade {
    /// Progress at `now`, `0..=1`.
    pub fn progress(&self, now: f64) -> f64 {
        if self.duration <= 0.0 {
            1.0
        } else {
            ((now - self.start) / self.duration).clamp(0.0, 1.0)
        }
    }
}

/// Drives cue playback.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CueRunner {
    /// Index of the cue GO'd last.
    pub current: Option<usize>,
    /// Running fade.
    pub fade: Option<Fade>,
}

impl CueRunner {
    /// Starts fading to `cue` from the project's live values.
    pub fn go(&mut self, project: &Project, cue: &Cue, index: usize, now: f64) {
        let live = capture(project);
        let from = cue
            .values
            .keys()
            .filter_map(|k| live.get(k).map(|v| (k.clone(), v.clone())))
            .collect();
        self.fade = Some(Fade {
            from,
            to: cue.values.clone(),
            start: now,
            duration: cue.fade_seconds.max(0.0),
        });
        self.current = Some(index);
    }

    /// GO to the cue after the current one (wrapping), if any. Returns its index.
    pub fn go_next(&mut self, project: &Project, now: f64) -> Option<usize> {
        if project.cues.is_empty() {
            return None;
        }
        let index = self.current.map_or(0, |i| (i + 1) % project.cues.len());
        let cue = project.cues.get(index)?.clone();
        self.go(project, &cue, index, now);
        Some(index)
    }

    /// Patches to apply at `now`. Empty when no fade is running.
    pub fn tick(&mut self, now: f64) -> Vec<ParamPatch> {
        let Some(fade) = &self.fade else {
            return Vec::new();
        };
        let t = fade.progress(now);
        let mut patches = Vec::new();
        for (address, to) in &fade.to {
            let Ok(addr) = ParamAddress::parse(address) else {
                continue;
            };
            let value = match fade.from.get(address) {
                Some(from) => blend_value(from, to, t),
                None => to.clone(),
            };
            patches.push(ParamPatch {
                address: addr,
                value,
            });
        }
        if t >= 1.0 {
            self.fade = None;
        }
        patches
    }

    /// Cancels any running fade (values stay where they are).
    pub fn stop(&mut self) {
        self.fade = None;
    }

    /// True while a fade runs.
    pub fn is_fading(&self) -> bool {
        self.fade.is_some()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::project::{demo_project, Playback, Source, SourceKind};

    fn project_with_cues() -> Project {
        let mut p = demo_project();
        p.sources.push(Source {
            id: "clip".into(),
            kind: SourceKind::Video {
                path: "c.mp4".into(),
                playback: Playback::default(),
            },
        });
        let mut dark = capture(&p);
        dark.insert("/surface/logo/opacity".into(), ParamValue::Number(0.0));
        dark.insert("/show/blackout".into(), ParamValue::Bool(true));
        let mut bright = capture(&p);
        bright.insert("/surface/logo/opacity".into(), ParamValue::Number(1.0));
        p.cues = vec![
            Cue {
                id: "dark".into(),
                name: "Dark".into(),
                fade_seconds: 2.0,
                values: dark,
            },
            Cue {
                id: "bright".into(),
                name: "Bright".into(),
                fade_seconds: 0.0,
                values: bright,
            },
        ];
        p
    }

    #[test]
    fn capture_covers_every_live_address() {
        let p = project_with_cues();
        let addrs = live_addresses(&p);
        assert!(addrs.contains(&"/show/master".to_string()));
        assert!(addrs.contains(&"/surface/logo/visible".to_string()));
        assert!(addrs.contains(&"/output/left/gamma".to_string()));
        assert!(addrs.contains(&"/source/clip/speed".to_string()));
        assert!(
            !addrs.iter().any(|a| a.starts_with("/source/grid")),
            "patterns have no transport"
        );
        let snap = capture(&p);
        assert_eq!(snap.len(), addrs.len(), "every live address resolves");
        assert_eq!(snap["/surface/logo/opacity"], ParamValue::Number(0.8));
    }

    #[test]
    fn values_blend() {
        assert_eq!(
            blend_value(&ParamValue::Number(0.0), &ParamValue::Number(1.0), 0.25),
            ParamValue::Number(0.25)
        );
        assert_eq!(
            blend_value(&ParamValue::Number(0.0), &ParamValue::Number(1.0), 7.0),
            ParamValue::Number(1.0)
        );
        assert_eq!(
            blend_value(&ParamValue::Bool(false), &ParamValue::Bool(true), 0.49),
            ParamValue::Bool(false)
        );
        assert_eq!(
            blend_value(&ParamValue::Bool(false), &ParamValue::Bool(true), 0.5),
            ParamValue::Bool(true)
        );
        assert_eq!(
            blend_value(&ParamValue::Text("a".into()), &ParamValue::Number(2.0), 0.0),
            ParamValue::Number(2.0)
        );
    }

    #[test]
    fn runner_fades_numbers_and_switches_bools_at_midpoint() {
        let mut p = project_with_cues();
        let mut r = CueRunner::default();
        assert!(r.tick(0.0).is_empty());
        assert_eq!(r.go_next(&p, 10.0), Some(0));
        assert!(r.is_fading());
        let apply = |p: &mut Project, patches: Vec<ParamPatch>| {
            for patch in patches {
                p.apply_patch(&patch).unwrap();
            }
        };
        apply(&mut p, r.tick(10.5));
        assert!(
            (p.surface("logo").unwrap().opacity - 0.6).abs() < 1e-9,
            "quarter way: 0.8 -> 0.0"
        );
        assert!(!p.blackout, "bools hold until half way");
        apply(&mut p, r.tick(11.0));
        assert!(p.blackout);
        assert!(r.is_fading());
        apply(&mut p, r.tick(12.5));
        assert_eq!(p.surface("logo").unwrap().opacity, 0.0);
        assert!(!r.is_fading(), "fade finished");
        assert!(r.tick(13.0).is_empty());
        // Next cue has zero fade: lands immediately.
        assert_eq!(r.go_next(&p, 20.0), Some(1));
        apply(&mut p, r.tick(20.0));
        assert_eq!(p.surface("logo").unwrap().opacity, 1.0);
        assert!(!r.is_fading());
        assert_eq!(r.go_next(&p, 21.0), Some(0), "wraps around");
        r.stop();
        assert!(!r.is_fading());
        let empty = Project::default();
        assert_eq!(r.go_next(&empty, 0.0), None);
    }

    #[test]
    fn missing_from_values_and_bad_addresses_are_tolerated() {
        let mut p = demo_project();
        let mut values = BTreeMap::new();
        values.insert("/surface/ghost/opacity".into(), ParamValue::Number(0.5));
        values.insert("not an address".into(), ParamValue::Number(0.5));
        values.insert("/show/master".into(), ParamValue::Number(0.25));
        let cue = Cue {
            id: "c".into(),
            name: String::new(),
            fade_seconds: 1.0,
            values,
        };
        let mut r = CueRunner::default();
        r.go(&p, &cue, 0, 0.0);
        let patches = r.tick(0.5);
        assert_eq!(patches.len(), 2, "the unparseable address is skipped");
        for patch in patches {
            let _ = p.apply_patch(&patch);
        }
        assert!((p.master - 0.625).abs() < 1e-9);
        assert_eq!(
            Fade {
                from: BTreeMap::new(),
                to: BTreeMap::new(),
                start: 0.0,
                duration: 0.0
            }
            .progress(0.0),
            1.0
        );
        let text = p.to_toml().unwrap();
        assert!(text.contains("[[cues]]") || p.cues.is_empty());
    }
}
