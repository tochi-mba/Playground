# lumen

Projection-mapping app: a Rust workspace with an egui editor, a wgpu renderer, a CPU
reference renderer that the GPU is tested against, and a headless CLI. Windows is the target
platform for shows; everything builds and tests on Linux too, which is how this container and
CI exercise it.

Cargo root is this folder, so run `cargo` from `lumen/`, not the repo root. The workflows live
at the repo root under `.github/workflows/` (`lumen-ci.yml`, `lumen-release.yml`) and the
Claude skill under `.claude/skills/lumen-render/`.

## Layout (paths relative to this folder)

- `crates/lumen-core/`: pure geometry (homographies, corner-pin quads, bicubic Bezier meshes),
  edge blending, masks, parameter addresses, the TOML project format with migrations and
  atomic saves, undo. No GPU, no I/O beyond the project file. Coverage floor 95 %.
- `crates/lumen-sources/`: RGBA frames, procedural test patterns, image loading, the Spout
  receiver behind the `spout` feature, and `SourceSet` (every source of a project, polled per
  frame).
- `crates/lumen-render/`: `scene` (project output → draw list), `software` (f64 reference
  rasteriser, the oracle), `gpu` (wgpu two-pass renderer, same maths), `compare` (diff
  metrics). WGSL shaders in `src/shaders/`. Golden tests in `tests/golden.rs`.
- `crates/lumen-control/`: OSC → parameter patches, UDP server thread.
- `crates/lumen-app/`: the `lumen` binary. `cli.rs` (render/validate/migrate/new),
  `editor/state.rs` (all edits, headless), `editor/{panels,canvas,outputs}.rs` (egui),
  `editor/ui_tests.rs` (egui_kittest drives the real UI), `autosave.rs`, `watchdog.rs`.
- `xtask/`: `cargo xtask ci` = the CI gate; `cargo xtask check-coverage FILE` enforces floors.
- `tests/golden/`: golden PNGs (256×144) and `cases/*.lumen.toml`.
- `examples/two-projector-demo.lumen.toml`: the demo project (`lumen new` writes the same).
- `docs/images/`: README renders, produced by `lumen render`.

## Build and test

- Toolchain is pinned in `rust-toolchain.toml`; `rust-version` in `Cargo.toml` is the MSRV CI
  checks.
- `cargo xtask ci` before every push. It runs fmt, clippy (`-D warnings`, pedantic), rustdoc,
  cargo-deny, all tests and the coverage floors (85 % workspace, 95 % core).
- GPU tests need a Vulkan adapter. In this container `apt-get install mesa-vulkan-drivers`
  provides lavapipe; run tests with `WGPU_BACKEND=vulkan`. `LUMEN_REQUIRE_GPU=1` turns a
  missing adapter into a failure (CI sets it on Linux).
- `UPDATE_GOLDENS=1` regenerates goldens; see the skill before doing that.
- The UI is tested through egui's accessibility tree, so every interactive widget needs an
  accessible name (label text fields with `labelled_by`). Unique button labels make tests
  simpler; avoid non-ASCII glyphs that the default font lacks.

## Conventions

- `#![forbid(unsafe_code)]` everywhere. Native Spout bindings, when they land, go in
  `lumen-sources/src/spout.rs` behind the feature with the forbid lifted only there.
- New live-controllable values get a parameter address in `lumen-core::project` (`get_param`
  and `apply_patch`) so OSC, undo and the inspector all work without extra code.
- Rendering changes must keep `gpu_matches_software_oracle` green. Change the oracle and the
  shader together; the oracle is the specification.
- Project schema changes bump `SCHEMA_VERSION` and add a `migrate_N_to_N+1` arm plus a test
  with a literal old file.
- Conventional-commit style subjects prefixed `lumen:`; keep `CHANGELOG.md` current.
