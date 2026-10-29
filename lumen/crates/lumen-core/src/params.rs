//! Typed parameter addresses.
//!
//! Every live-controllable value in a project has an address like `/surface/wall/opacity` or
//! `/output/left/blend/right/width`. The editor, the OSC server and the tests all speak this
//! one vocabulary, so a control surface and a unit test can set the same thing the same way.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A parameter path. Segments are stored without slashes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ParamAddress {
    segments: Vec<String>,
}

/// Errors from parsing an address.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParamError {
    /// Missing leading slash or empty.
    #[error("address must start with '/' and have at least one segment")]
    Malformed,
    /// Illegal character in a segment.
    #[error("segment {0:?} may only contain [A-Za-z0-9_-]")]
    BadSegment(String),
}

impl ParamAddress {
    /// Parses an address string.
    pub fn parse(s: &str) -> Result<Self, ParamError> {
        let Some(rest) = s.strip_prefix('/') else {
            return Err(ParamError::Malformed);
        };
        if rest.is_empty() {
            return Err(ParamError::Malformed);
        }
        let mut segments = Vec::new();
        for seg in rest.split('/') {
            if seg.is_empty() {
                return Err(ParamError::Malformed);
            }
            if !seg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(ParamError::BadSegment(seg.to_string()));
            }
            segments.push(seg.to_string());
        }
        Ok(Self { segments })
    }

    /// Path segments.
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// Segment at `i` as a `&str`.
    pub fn seg(&self, i: usize) -> Option<&str> {
        self.segments.get(i).map(String::as_str)
    }
}

impl fmt::Display for ParamAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for s in &self.segments {
            write!(f, "/{s}")?;
        }
        Ok(())
    }
}

impl FromStr for ParamAddress {
    type Err = ParamError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<String> for ParamAddress {
    type Error = ParamError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s)
    }
}

impl From<ParamAddress> for String {
    fn from(a: ParamAddress) -> Self {
        a.to_string()
    }
}

/// A value carried by a control message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    /// Boolean toggle.
    Bool(bool),
    /// Number; integers arrive as floats too.
    Number(f64),
    /// Free text (e.g. a source path or a curve name).
    Text(String),
}

impl ParamValue {
    /// Coerces to a float: bools become 0/1, numeric strings parse, else `None`.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Self::Number(n) => Some(*n),
            Self::Text(t) => t.trim().parse().ok(),
        }
    }

    /// Coerces to a bool: numbers are true when non-zero, text parses `true`/`false`/`1`/`0`.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            Self::Number(n) => Some(*n != 0.0),
            Self::Text(t) => match t.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "on" => Some(true),
                "false" | "0" | "off" => Some(false),
                _ => None,
            },
        }
    }
}

/// One parameter change: an address and the value to set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamPatch {
    /// What to change.
    pub address: ParamAddress,
    /// New value.
    pub value: ParamValue,
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display_round_trip() {
        let a = ParamAddress::parse("/surface/wall-1/opacity").unwrap();
        assert_eq!(a.segments().len(), 3);
        assert_eq!(a.seg(1), Some("wall-1"));
        assert_eq!(a.seg(3), None);
        assert_eq!(a.to_string(), "/surface/wall-1/opacity");
        let b: ParamAddress = "/a/b".parse().unwrap();
        assert_eq!(b.seg(0), Some("a"));
        let s: String = b.clone().into();
        assert_eq!(s, "/a/b");
        assert_eq!(ParamAddress::try_from("/a/b".to_string()).unwrap(), b);
    }

    #[test]
    fn rejects_bad_addresses() {
        assert_eq!(ParamAddress::parse("surface/x"), Err(ParamError::Malformed));
        assert_eq!(ParamAddress::parse("/"), Err(ParamError::Malformed));
        assert_eq!(ParamAddress::parse("/a//b"), Err(ParamError::Malformed));
        assert_eq!(
            ParamAddress::parse("/a/b c"),
            Err(ParamError::BadSegment("b c".into()))
        );
        assert!(ParamError::BadSegment("x".into()).to_string().contains('x'));
        assert!(ParamAddress::try_from(String::new()).is_err());
    }

    #[test]
    fn serde_uses_the_string_form() {
        #[derive(Serialize, Deserialize)]
        struct Holder {
            a: ParamAddress,
        }
        let h: Holder = toml::from_str(r#"a = "/x/y""#).unwrap();
        assert_eq!(h.a.seg(1), Some("y"));
        assert_eq!(toml::to_string(&h).unwrap().trim(), r#"a = "/x/y""#);
        assert!(toml::from_str::<Holder>(r#"a = "bad""#).is_err());
    }

    #[test]
    fn value_coercions() {
        assert_eq!(ParamValue::Bool(true).as_f64(), Some(1.0));
        assert_eq!(ParamValue::Bool(false).as_f64(), Some(0.0));
        assert_eq!(ParamValue::Number(2.5).as_f64(), Some(2.5));
        assert_eq!(ParamValue::Text(" 3 ".into()).as_f64(), Some(3.0));
        assert_eq!(ParamValue::Text("x".into()).as_f64(), None);
        assert_eq!(ParamValue::Number(0.0).as_bool(), Some(false));
        assert_eq!(ParamValue::Number(0.1).as_bool(), Some(true));
        assert_eq!(ParamValue::Bool(true).as_bool(), Some(true));
        for (t, want) in [
            ("ON", Some(true)),
            ("off", Some(false)),
            ("1", Some(true)),
            ("maybe", None),
        ] {
            assert_eq!(ParamValue::Text(t.into()).as_bool(), want);
        }
    }

    #[test]
    fn patch_serialises_untagged_values() {
        let p: ParamPatch = toml::from_str(
            r#"address = "/a"
value = true"#,
        )
        .unwrap();
        assert_eq!(p.value, ParamValue::Bool(true));
        let p: ParamPatch = toml::from_str(
            r#"address = "/a"
value = 1.5"#,
        )
        .unwrap();
        assert_eq!(p.value, ParamValue::Number(1.5));
        let p: ParamPatch = toml::from_str(
            r#"address = "/a"
value = "hi""#,
        )
        .unwrap();
        assert_eq!(p.value, ParamValue::Text("hi".into()));
    }
}
