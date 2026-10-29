# Contributing to lumen

## Before you push

```bash
cd lumen
cargo xtask ci
```

That is the CI gate, run locally: `cargo fmt --check`, `cargo clippy --all-targets
--all-features -D warnings`, `cargo doc` with warnings denied, `cargo deny check`, every test,
and the coverage floors (85 % of lines across the workspace, 95 % in `lumen-core`). CI runs the
same on Linux (with a software Vulkan driver, so the GPU tests are mandatory there) and on
Windows.

## Tests

- Unit and property tests live next to the code (`proptest` for geometry and the project
  format).
- `crates/lumen-render/tests/golden.rs` renders every case under `tests/golden/cases/` with the
  CPU oracle and compares with the committed PNGs, then compares the wgpu renderer with the
  oracle. See `.claude/skills/lumen-render/SKILL.md` for updating goldens.
- `crates/lumen-app/tests/cli.rs` runs the binary as a subprocess.
- `crates/lumen-app/src/editor/ui_tests/` drives the real egui UI through its accessibility
  tree with `egui_kittest`. Give every interactive widget an accessible name, use ASCII
  labels (the default font has no arrows or icons) and unique button texts.

## Style

- Pedantic clippy is on; prefer fixing over allowing. Allow only with a reason that fits on
  one line.
- No source file over 1000 lines (`cargo xtask file-lengths` enforces it). Split by concern
  into a module directory, as `editor/state/` and `lumen-core/src/project/` are.
- No `unsafe` outside a future native Spout module.
- Public items are documented. Doc comments say what and why, not how.
- Commits: `lumen: <imperative summary>`, body explains the why. Keep `CHANGELOG.md` current
  under *Unreleased*.

## Releasing

1. Move *Unreleased* in `CHANGELOG.md` to a version heading and bump `version` in
   `Cargo.toml`.
2. Tag `lumen-vX.Y.Z` on `main` and push the tag. `lumen-release.yml` builds the Windows
   binary, zips it with the licences and examples, and publishes a GitHub Release with
   `SHA256SUMS.txt`.
