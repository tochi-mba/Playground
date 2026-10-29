//! Parameter addresses: reading and patching live values.

use super::{Project, ProjectError, SourceKind};
use crate::params::{ParamAddress, ParamPatch, ParamValue};
use crate::undo::KeyedValues;
use crate::warp::WarpKind;

impl Project {
    /// Reads the value at a parameter address, if it exists.
    pub fn get_param(&self, address: &ParamAddress) -> Option<ParamValue> {
        use ParamValue::{Bool, Number};
        let segs: Vec<&str> = address.segments().iter().map(String::as_str).collect();
        match segs.as_slice() {
            ["show", "blackout"] => Some(Bool(self.blackout)),
            ["show", "master"] => Some(Number(self.master)),
            ["surface", id, rest @ ..] => {
                let s = self.surface(id)?;
                match rest {
                    ["opacity"] => Some(Number(s.opacity)),
                    ["visible"] => Some(Bool(s.visible)),
                    ["z"] => Some(Number(f64::from(s.z))),
                    ["corner", n, axis] => {
                        let WarpKind::Quad(q) = &s.warp else {
                            return None;
                        };
                        let c = q.corners.get(n.parse::<usize>().ok()?)?;
                        match *axis {
                            "x" => Some(Number(c.x)),
                            "y" => Some(Number(c.y)),
                            _ => None,
                        }
                    }
                    _ => None,
                }
            }
            ["source", id, rest @ ..] => {
                let s = self.source(id)?;
                match (rest, &s.kind) {
                    (["speed"], SourceKind::Generator { speed, .. }) => {
                        Some(Number(f64::from(*speed)))
                    }
                    (["play"], k) => k.playback().map(|p| Bool(!p.paused)),
                    (["loop"], k) => k.playback().map(|p| Bool(p.looping)),
                    (["speed"], k) => k.playback().map(|p| Number(p.speed)),
                    (["restart"], k) => k.playback().map(|_| Bool(false)),
                    _ => None,
                }
            }
            ["output", id, rest @ ..] => {
                let o = self.output(id)?;
                match rest {
                    ["enabled"] => Some(Bool(o.enabled)),
                    ["brightness"] => Some(Number(o.color.brightness)),
                    ["contrast"] => Some(Number(o.color.contrast)),
                    ["gamma"] => Some(Number(o.color.gamma)),
                    ["blend", "gamma"] => Some(Number(o.blend.gamma)),
                    ["blend", edge, "width"] => {
                        let e = match *edge {
                            "left" => o.blend.left,
                            "right" => o.blend.right,
                            "top" => o.blend.top,
                            "bottom" => o.blend.bottom,
                            _ => return None,
                        };
                        Some(Number(e.width))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Applies a parameter change.
    pub fn apply_patch(&mut self, patch: &ParamPatch) -> Result<(), ProjectError> {
        let addr = patch.address.to_string();
        let unknown = || ProjectError::UnknownParam(addr.clone());
        let bad = || ProjectError::BadValue(addr.clone(), patch.value.clone());
        let num = || patch.value.as_f64().ok_or_else(bad);
        let boolean = || patch.value.as_bool().ok_or_else(bad);
        let segs: Vec<&str> = patch
            .address
            .segments()
            .iter()
            .map(String::as_str)
            .collect();
        match segs.as_slice() {
            ["show", "blackout"] => self.blackout = boolean()?,
            ["show", "master"] => self.master = num()?.clamp(0.0, 1.0),
            ["surface", id, rest @ ..] => {
                let s = self.surface_mut(id).ok_or_else(unknown)?;
                match rest {
                    ["opacity"] => s.opacity = num()?.clamp(0.0, 1.0),
                    ["visible"] => s.visible = boolean()?,
                    ["z"] => s.z = num()? as i32,
                    ["corner", n, axis] => {
                        let v = num()?;
                        let WarpKind::Quad(q) = &mut s.warp else {
                            return Err(unknown());
                        };
                        let i = n.parse::<usize>().map_err(|_| unknown())?;
                        let c = q.corners.get_mut(i).ok_or_else(unknown)?;
                        match *axis {
                            "x" => c.x = v,
                            "y" => c.y = v,
                            _ => return Err(unknown()),
                        }
                    }
                    _ => return Err(unknown()),
                }
            }
            ["source", id, rest @ ..] => {
                let s = self
                    .sources
                    .iter_mut()
                    .find(|s| s.id == *id)
                    .ok_or_else(unknown)?;
                if let (["speed"], SourceKind::Generator { speed, .. }) = (rest, &mut s.kind) {
                    *speed = num()?.clamp(0.0, 10.0) as f32;
                    return Ok(());
                }
                let pb = s.kind.playback_mut().ok_or_else(unknown)?;
                match rest {
                    ["play"] => pb.paused = !boolean()?,
                    ["loop"] => pb.looping = boolean()?,
                    ["speed"] => pb.speed = num()?.clamp(0.0, 16.0),
                    ["restart"] => pb.restart_token = pb.restart_token.wrapping_add(1),
                    _ => return Err(unknown()),
                }
            }
            ["output", id, rest @ ..] => {
                let o = self.output_mut(id).ok_or_else(unknown)?;
                match rest {
                    ["enabled"] => o.enabled = boolean()?,
                    ["brightness"] => o.color.brightness = num()?.clamp(0.0, 4.0),
                    ["contrast"] => o.color.contrast = num()?.clamp(0.0, 4.0),
                    ["gamma"] => o.color.gamma = num()?.clamp(0.2, 5.0),
                    ["blend", "gamma"] => o.blend.gamma = num()?.clamp(0.5, 4.0),
                    ["blend", edge, "width"] => {
                        let v = num()?.clamp(0.0, 1.0);
                        let e = match *edge {
                            "left" => &mut o.blend.left,
                            "right" => &mut o.blend.right,
                            "top" => &mut o.blend.top,
                            "bottom" => &mut o.blend.bottom,
                            _ => return Err(unknown()),
                        };
                        e.width = v;
                    }
                    _ => return Err(unknown()),
                }
            }
            _ => return Err(unknown()),
        }
        Ok(())
    }
}

impl KeyedValues<ParamValue> for Project {
    fn set_keyed(&mut self, key: &str, value: &ParamValue) {
        if let Ok(address) = ParamAddress::parse(key) {
            // Undo commands are built from values this project produced, so a failure here
            // means the target was deleted since; ignoring it is the right thing for undo.
            let _ = self.apply_patch(&ParamPatch {
                address,
                value: value.clone(),
            });
        }
    }
}
