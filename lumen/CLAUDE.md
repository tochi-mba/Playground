# lumen

Projection-mapping app: a Rust workspace with an egui editor, a wgpu renderer, a CPU
reference renderer that the GPU is tested against, and a headless CLI. Windows is the target
platform for shows; everything builds and tests on Linux too, which is how this container and
CI exercise it.

Cargo root is this folder, so run `cargo` from `lumen/`, not the repo root. The workflows live
at the repo root under `.github/workflows/` (`lumen-ci.yml`, `lumen-release.yml`) and the
Claude skill under `.claude/skills/lumen-render/`. The user guide is `docs/manual.md`.

## Layout (paths relative to this folder)

- `crates/lumen-core/`: pure geometry (homographies, corner-pin quads, bicubic Bezier meshes),
  edge blending and the blend-width suggestion (`blend.rs`), masks, parameter addresses,
  cues (`cues.rs`: snapshots and the crossfade runner), the MIDI map (`midimap.rs`), undo,
  and the TOML project format in `project/` (`mod.rs` types and ids, `params.rs` get/apply,
  `validate.rs`, `migrate.rs`, `demo.rs`, `tests.rs`). No GPU, no I/O beyond the project
  file. Coverage floor 95 %.
- `crates/lumen-sources/`: RGBA frames, test patterns, calibration overlays (`overlay.rs`),
  images, video via an `ffmpeg` subprocess (`video.rs`), image sequences and GIFs, generators,
  the frame clock, the Spout receiver (`spout.rs`, native on Windows behind the `spout`
  feature, stub elsewhere), and `SourceSet` (every source of a project, polled per frame).
- `crates/lumen-render/`: `scene` (project output → draw list, overlays appended as a
  full-frame draw), `software` (f64 reference rasteriser, the oracle), `gpu` (wgpu two-pass
  renderer, same maths), `compare` (diff metrics). WGSL shaders in `src/shaders/`. Golden
  tests in `tests/golden.rs`.
- `crates/lumen-control/`: OSC server and OSC feedback, the midir MIDI listener, the phone
  remote (`web.rs`, tiny_http + `assets/remote.html`, QR modules).
- `crates/lumen-app/`: the `lumen` binary. `cli.rs` (render/validate/migrate/new),
  `settings.rs` (preferences file), `theme.rs` (visuals and icon), `autosave.rs`,
  `watchdog.rs`, and `editor/`: `state/` (all edits, headless: `mod.rs` core and params,
  `objects.rs` outputs/surfaces/sources, `control.rs` OSC/cues/MIDI/remote, `session.rs`
  files and show mode), `panels/` (egui: `mod.rs` menus/tree/status, `inspectors.rs`,
  `control.rs` cue panel and controller section, `dialogs.rs`), `canvas.rs`, `stage.rs`,
  `handles.rs`, `outputs.rs` (output windows), `thumbs.rs`, `ui_tests/` (egui_kittest drives
  the real UI).
- `xtask/`: `cargo xtask ci` = the CI gate; `check-coverage FILE` enforces floors;
  `file-lengths` rejects source files over 1000 lines.
- `tests/golden/`: golden PNGs (256×144) and `cases/*.lumen.toml`.
- `examples/two-projector-demo.lumen.toml`: the demo project (`lumen new` writes the same).
- `docs/images/`: README renders, produced by `lumen render`.

## Build and test

- Toolchain is pinned in `rust-toolchain.toml`; `rust-version` in `Cargo.toml` is the MSRV CI
  checks.
- `cargo xtask ci` before every push. It runs the file-length check, fmt, clippy (`-D
  warnings`, pedantic), rustdoc, cargo-deny, all tests and the coverage floors (85 %
  workspace, 95 % core).
- GPU tests need a Vulkan adapter. In this container `apt-get install mesa-vulkan-drivers`
  provides lavapipe; run tests with `WGPU_BACKEND=vulkan`. `LUMEN_REQUIRE_GPU=1` turns a
  missing adapter into a failure (CI sets it on Linux). Video tests use `ffmpeg` from the
  path (`LUMEN_REQUIRE_FFMPEG=1` on CI); MIDI needs `libasound2-dev` on Linux.
- `UPDATE_GOLDENS=1` regenerates goldens; see the skill before doing that.
- The UI is tested through egui's accessibility tree, so every interactive widget needs an
  accessible name (label text fields and drag values with `labelled_by`, combo boxes with
  `from_label`). Unique button labels make tests simpler; avoid non-ASCII glyphs that the
  default font lacks. Combo options are clicked by label after clicking the combo's label.
- The Spout native path only compiles on Windows (MSVC, x64); CI's Windows job builds it.
  It cannot be run here, so keep `spout.rs`'s native module small and obviously correct.

## Conventions

- `#![forbid(unsafe_code)]` everywhere, including the Spout receiver (the `spout2` crate's
  DirectX CPU path is safe to call).
- No source file over 1000 lines; split into a module directory by concern.
- New live-controllable values get a parameter address in `lumen-core::project` (`get_param`
  and `apply_patch`) so OSC, MIDI, cues, undo and the inspector all work without extra code.
- Rendering changes must keep `gpu_matches_software_oracle` green. Change the oracle and the
  shader together; the oracle is the specification.
- Project schema changes bump `SCHEMA_VERSION` and add a `migrate_N_to_N+1` arm plus a test
  with a literal old file. Optional fields with serde defaults do not need a bump.
- Conventional-commit style subjects prefixed `lumen:`; keep `CHANGELOG.md` current.
