//! Rendering for lumen: one scene description, two renderers.
//!
//! [`scene`] turns a project output into a flat list of draws. [`software`] rasterises that
//! scene on the CPU in `f64`: slow, exact and platform-independent, so it is the oracle the
//! golden images are made from. [`gpu`] renders the same scene with wgpu for real-time output,
//! and the tests prove it matches the oracle within a tight tolerance. [`compare`] holds the
//! image-difference metrics those tests use.

#![forbid(unsafe_code)]

pub mod compare;
pub mod gpu;
pub mod scene;
pub mod software;

pub use image::RgbaImage;
pub use scene::{Scene, SceneError};
