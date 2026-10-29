//! Schema migrations.

use super::{ProjectError, SCHEMA_VERSION};

/// Brings a raw TOML document up to [`SCHEMA_VERSION`] in place.
///
/// Each step is a pure function on the table; add a new arm when the schema changes and leave
/// the old arms alone so every historical file keeps loading.
pub fn migrate(doc: &mut toml::Table) -> Result<(), ProjectError> {
    loop {
        let version = match doc.get("schema_version") {
            None => 0,
            Some(toml::Value::Integer(v)) if *v >= 0 => *v as u32,
            Some(other) => {
                return Err(ProjectError::Parse(format!(
                    "schema_version must be a non-negative integer, got {other}"
                )))
            }
        };
        match version {
            0 => migrate_0_to_1(doc),
            v if v == SCHEMA_VERSION => return Ok(()),
            v if v > SCHEMA_VERSION => {
                return Err(ProjectError::TooNew {
                    found: v,
                    supported: SCHEMA_VERSION,
                })
            }
            // Every version below SCHEMA_VERSION has an arm above; this is unreachable by
            // construction but keeps the match exhaustive when a version is added.
            v => return Err(ProjectError::Parse(format!("no migration from schema {v}"))),
        }
    }
}

/// Schema 0 was the pre-release format: outputs stored `size = [w, h]` and surfaces used
/// `alpha` instead of `opacity`.
fn migrate_0_to_1(doc: &mut toml::Table) {
    if let Some(toml::Value::Array(outputs)) = doc.get_mut("outputs") {
        for o in outputs.iter_mut().filter_map(toml::Value::as_table_mut) {
            if let Some(toml::Value::Array(size)) = o.remove("size") {
                if let [w, h] = size.as_slice() {
                    o.insert("width".into(), w.clone());
                    o.insert("height".into(), h.clone());
                }
            }
        }
    }
    if let Some(toml::Value::Array(surfaces)) = doc.get_mut("surfaces") {
        for s in surfaces.iter_mut().filter_map(toml::Value::as_table_mut) {
            if let Some(a) = s.remove("alpha") {
                s.entry("opacity").or_insert(a);
            }
        }
    }
    doc.insert("schema_version".into(), toml::Value::Integer(1));
}
