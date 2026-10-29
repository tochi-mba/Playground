---
name: lumen-render
description: Render a lumen project headlessly to PNG, run or update the golden-image tests, and read a GPU-vs-oracle mismatch. Use after changing anything under lumen/ that touches geometry, blending, sources, shaders or the project format, or when asked what a project looks like.
---

# Rendering and golden images for lumen

Everything runs from `lumen/` (the Cargo workspace root). No window is needed: `lumen render`
and the tests use a headless wgpu adapter, falling back to the CPU oracle when there is none.

## Render a project to PNG

```bash
cd lumen
cargo run -q -p lumen-app -- render examples/two-projector-demo.lumen.toml --size 640x360 --out /tmp/frames
cargo run -q -p lumen-app -- render path/to/show.lumen.toml -o left --out /tmp/left.png --software
```

`--software` forces the f64 reference renderer; without it the GPU is used when an adapter
exists. In this container set `WGPU_BACKEND=vulkan` so wgpu picks Mesa's lavapipe.

## Run the golden tests

```bash
WGPU_BACKEND=vulkan LUMEN_REQUIRE_GPU=1 cargo test -p lumen-render --test golden
```

- `software_matches_goldens` compares the oracle with `tests/golden/<case>.<output>.png`.
- `gpu_matches_software_oracle` compares the wgpu renderer with the oracle
  (`Thresholds::GPU` in `crates/lumen-render/src/compare.rs`).
- Cases live in `tests/golden/cases/*.lumen.toml`; the demo project is a case too. Cases
  cover warps, blend modes, post, blackout, the animated generators at a fixed time and the
  calibration overlays (`overlays.lumen.toml`, one output per overlay kind).

On failure the test writes `<golden>.actual.png` and `<golden>.diff.png` (white = different)
next to the golden. Read both with the Read tool before deciding whether the change is
intended.

## Update goldens

Only when the rendering change is intended:

```bash
UPDATE_GOLDENS=1 cargo test -p lumen-render --test golden software_matches_goldens
```

Commit the regenerated PNGs with the code change and say in the commit message why the image
changed. Never update goldens to make a GPU mismatch go away: that test compares two renderers
of the same scene, so a mismatch is a bug in one of them.

## Add a case

1. Write `tests/golden/cases/<name>.lumen.toml` (any valid project; keep sources to built-in
   patterns or solids so the case needs no files).
2. Run the goldens with `UPDATE_GOLDENS=1` once to create `<name>.<output>.png`.
3. Look at the PNG, then commit both files.

## Full local gate

`cargo xtask ci` runs the file-length check (no source file over 1000 lines), fmt, clippy,
docs, cargo-deny, every test and the coverage floors, the same as
`.github/workflows/lumen-ci.yml`. Video tests need `ffmpeg` on the path (`/usr/bin/ffmpeg`
in this container; `LUMEN_REQUIRE_FFMPEG=1` makes a missing binary a failure, as CI does).
