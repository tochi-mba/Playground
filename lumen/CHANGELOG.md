# Changelog

All notable changes to lumen are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-10-29

### Added

- Corner-pin (homography) and bicubic Bezier mesh warps with perspective-correct sampling.
- Gamma-correct soft-edge blending on all four edges with linear, smoothstep and cosine
  curves; per-output brightness, contrast and gamma; master level and blackout.
- Feathered polygon masks (holes and windows) per surface; four blend modes; source crop.
- Sources: grid, bars, checker and ramp patterns, solid colours, PNG/JPEG images, and a
  Spout receiver interface behind the `spout` feature.
- Plain-text `.lumen.toml` projects with schema versioning, migration from the pre-release
  format, validation and atomic saves.
- egui editor with draggable handles, undo/redo with drag coalescing, inspector, object
  tree, and per-output borderless always-on-top windows.
- Locked show mode, autosave with recovery, render-loop watchdog with optional relaunch.
- OSC control over every live parameter.
- Headless CLI: `render`, `validate`, `migrate`, `new`.
- CPU reference renderer, wgpu renderer, golden-image tests and GPU-versus-oracle tests.
- CI on Linux and Windows with clippy, cargo-deny, coverage floors and a release workflow.

[Unreleased]: https://github.com/tochi-mba/playground/compare/lumen-v0.1.0...HEAD
[0.1.0]: https://github.com/tochi-mba/playground/releases/tag/lumen-v0.1.0
