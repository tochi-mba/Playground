//! Pure, GPU-free foundations of the lumen projection mapper.
//!
//! Everything here is deterministic and runs on any platform, which is what lets the
//! geometry, blending and project format be tested exhaustively without a window or a GPU.
//! The crate is organised bottom-up:
//!
//! - [`geom`]: vectors, 3×3 matrices and homographies.
//! - [`warp`]: corner-pin quads and bicubic Bezier meshes that tessellate to [`warp::WarpMesh`].
//! - [`blend`]: soft-edge blending curves for overlapping projectors.
//! - [`mask`]: feathered polygon masks.
//! - [`params`]: typed parameter addresses shared by the editor, OSC and tests.
//! - [`project`]: the versioned, plain-text project file.
//! - [`undo`]: command history with drag coalescing.

#![forbid(unsafe_code)]

pub mod blend;
pub mod geom;
pub mod mask;
pub mod params;
pub mod project;
pub mod undo;
pub mod warp;

pub use geom::{Homography, Mat3, Vec2};
pub use project::Project;
