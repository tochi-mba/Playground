//! Referential and geometric checks.

use super::{is_valid_id, Issue, Project, SCHEMA_VERSION};
use crate::warp::Warp;
use std::collections::HashSet;

impl Project {
    #[allow(clippy::too_many_lines)]
    /// Checks referential integrity and geometry. Empty when the project is sound.
    pub fn validate(&self) -> Vec<Issue> {
        let mut issues = Vec::new();
        if self.schema_version > SCHEMA_VERSION {
            issues.push(Issue::err(format!(
                "schema_version {} is newer than {SCHEMA_VERSION}",
                self.schema_version
            )));
        }
        let mut seen = HashSet::new();
        for (what, ids) in [
            (
                "source",
                self.sources
                    .iter()
                    .map(|s| s.id.as_str())
                    .collect::<Vec<_>>(),
            ),
            (
                "output",
                self.outputs.iter().map(|o| o.id.as_str()).collect(),
            ),
            (
                "surface",
                self.surfaces.iter().map(|s| s.id.as_str()).collect(),
            ),
        ] {
            seen.clear();
            for id in ids {
                if !is_valid_id(id) {
                    issues.push(Issue::err(format!(
                        "{what} id {id:?} must match [A-Za-z0-9_-]+"
                    )));
                }
                if !seen.insert(id) {
                    issues.push(Issue::err(format!("duplicate {what} id {id:?}")));
                }
            }
        }
        seen.clear();
        for c in &self.cues {
            if !is_valid_id(&c.id) {
                issues.push(Issue::err(format!(
                    "cue id {:?} must match [A-Za-z0-9_-]+",
                    c.id
                )));
            }
            if !seen.insert(c.id.as_str()) {
                issues.push(Issue::err(format!("duplicate cue id {:?}", c.id)));
            }
        }
        for o in &self.outputs {
            if o.width == 0 || o.height == 0 {
                issues.push(Issue::err(format!(
                    "output {:?} has a zero dimension",
                    o.id
                )));
            }
            if o.width > 16384 || o.height > 16384 {
                issues.push(Issue::warn(format!(
                    "output {:?} is larger than 16384px",
                    o.id
                )));
            }
        }
        for s in &self.surfaces {
            if self.output(&s.output).is_none() {
                issues.push(Issue::err(format!(
                    "surface {:?} references unknown output {:?}",
                    s.id, s.output
                )));
            }
            if self.source(&s.source).is_none() {
                issues.push(Issue::err(format!(
                    "surface {:?} references unknown source {:?}",
                    s.id, s.source
                )));
            }
            if let Err(e) = s.warp.tessellate(1) {
                issues.push(Issue::err(format!(
                    "surface {:?} warp is invalid: {e}",
                    s.id
                )));
            }
            if !(0.0..=1.0).contains(&s.opacity) {
                issues.push(Issue::warn(format!(
                    "surface {:?} opacity {} is outside [0,1]",
                    s.id, s.opacity
                )));
            }
            for (i, m) in s.masks.iter().enumerate() {
                if !m.is_valid() {
                    issues.push(Issue::err(format!(
                        "surface {:?} mask {i} needs at least three finite points",
                        s.id
                    )));
                }
            }
        }
        if self.outputs.is_empty() {
            issues.push(Issue::warn("project has no outputs"));
        }
        issues
    }

    /// True when [`Project::validate`] reports no errors.
    pub fn is_valid(&self) -> bool {
        self.validate().iter().all(|i| !i.error)
    }
}
