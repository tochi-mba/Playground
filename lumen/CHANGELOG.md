# Changelog

All notable changes to lumen are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Welcome screen that creates an output per detected display, recent files, native file
  dialogs, drag-and-drop of media, unsaved-changes prompts and a keyboard cheat sheet.
- Edit on the wall: output windows show the same handles as the canvas; identify flashes
  each output's name on its projector; cursor hides on idle outputs.
- Canvas zoom and pan, output-space grid with snapping, numeric readout of the active
  handle, fine and coarse nudges, align operations, duplicate, lock, rename, reorder and
  context menus.
- Video (`ffmpeg` subprocess), image sequences, GIFs and six animated generators with a
  shared transport (`/source/<id>/play|loop|speed|restart`); thumbnails and a draggable
  crop preview.
- Cues: stored looks with crossfades, GO on Space, a button, OSC (`/cue/go`,
  `/cue/<id>/go`, `/cue/stop`), MIDI or the phone remote.
- MIDI learn for any parameter (fader, button and switch modes), saved in the project.
- OSC feedback of every changed parameter to a `host:port`.
- Phone remote: `lumen show --remote-port 8080` serves a page with blackout, master,
  output levels, surface opacities and the cue list; the editor shows a QR code of it.
- Settings file (`settings.toml` in the OS config directory) for ports, grid, snap,
  autosave interval and the ffmpeg path.
- Stage view (`T`): every placed output drawn to scale on the virtual desktop with monitor
  outlines and the overlaps highlighted; click selects, drag moves the placement.
- "Suggest blends" sets left/right or top/bottom blend widths from the overlap of placed
  outputs.
- Calibration overlays per output (grid, crosshair, colour bars, grey steps, blend check),
  never shown in show mode, and a gamma match tool (`[` / `]` nudge the blend gamma).
- `cargo xtask file-lengths` (also in `cargo xtask ci`): no source file over 1000 lines.

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
