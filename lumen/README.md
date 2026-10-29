# lumen

Projection mapping that you can trust on show day.

lumen maps video sources onto physical surfaces through one or many projectors. It is a
native Rust application (egui editor, wgpu renderer) with a few deliberate differences from
the usual tools:

- **Provably correct geometry.** Corner-pin warps are true homographies with
  perspective-correct texture sampling; curved surfaces use bicubic Bezier meshes; projector
  overlaps are blended in linear light with gamma-correct falloffs. A CPU reference renderer is
  the specification, golden images are generated from it, and the GPU renderer is compared
  against it on every push.
- **Show-day reliability.** A locked show mode (double-Escape to leave), autosave with crash
  recovery, atomic project writes, a render-loop watchdog that can relaunch the show, and a
  startup self-check that opens every enabled output where the project says it belongs.
- **Plain-text projects.** `.lumen.toml` files are small, diffable and versioned with
  migrations, so a show lives happily in git.
- **Scriptable and testable.** `lumen render` draws any output to PNG without a window;
  OSC drives every live parameter; the whole app, UI included, is covered by headless tests.
- **Permissive and light.** MIT OR Apache-2.0, one executable, no runtime dependencies, no
  watermark.

| Two-projector demo, left output (blended right edge) | Right output (overlap, screen-mode logo with a feathered mask) | Bezier mesh on a curve |
|---|---|---|
| ![left](docs/images/left.png) | ![right](docs/images/right.png) | ![bezier](docs/images/bezier.png) |

## Install

Download `lumen-<version>-windows-x64.zip` from the repository's releases (tags `lumen-v*`),
or build from source:

```bash
cd lumen
cargo build --release -p lumen-app            # any platform
cargo build --release -p lumen-app --features spout   # Windows, Spout receiver
```

Requires a GPU with Vulkan or DirectX 12 support (wgpu picks DX12 on Windows).

## Use

```text
lumen                              open the editor with the demo project
lumen show.lumen.toml              open a project
lumen show show.lumen.toml         straight into locked show mode, outputs open
lumen render show.lumen.toml --out frames/     PNG per output, headless
lumen validate show.lumen.toml     check references and geometry (exit 1 on errors)
lumen migrate old.lumen.toml       rewrite in the current schema
lumen new my-show.lumen.toml       write the two-projector demo
```

### Editing

- **Outputs** are projectors: size, enabled-in-show, colour (brightness, contrast, gamma),
  four-edge blend (width, curve, blend gamma) and placement. An unplaced output opens as a
  draggable preview window; drag it onto the projector and press *Use current window
  position*, or pick *Fill monitor …*. Placed outputs open borderless, always on top, at full
  size.
- **Surfaces** place a source on an output: corner-pin (drag the four handles; Shift-drag moves
  all; arrow keys nudge the active handle, Shift for ×10) or a Bezier mesh (convert from a
  quad, then drag control points). Opacity, z-order, blend mode, source crop and feathered
  polygon masks (holes or windows) are in the inspector.
- **Sources**: built-in grid, bars, checker and ramp patterns, solid colours, PNG/JPEG images,
  and Spout senders (Windows builds with the `spout` feature).
- **Shortcuts**: Ctrl+Z / Ctrl+Y undo and redo (a whole drag is one step), Ctrl+S save, Delete
  removes the selection, F10 toggles show mode, Escape twice leaves show mode.

### Show mode

Editing is locked, every enabled output window opens, OSC stays live, autosave pauses, and the
editor window shows status only. `lumen show --restart-on-stall 5 show.lumen.toml` relaunches
the process if the render loop stops beating for five seconds.

### OSC

Default port 9000 (`--osc-port 0` disables). One message per parameter, first argument is the
value; a message with no arguments means `true`.

```text
/show/blackout            bool      /show/master               0..1
/surface/<id>/opacity     0..1      /surface/<id>/visible      bool
/surface/<id>/z           int       /surface/<id>/corner/<0-3>/x|y  0..1 (quads)
/output/<id>/enabled      bool      /output/<id>/brightness|contrast|gamma
/output/<id>/blend/<left|right|top|bottom>/width       /output/<id>/blend/gamma
```

### Project file

```toml
schema_version = 1
name = "Two projector demo"

[[sources]]
id = "grid"
kind = "test_pattern"
pattern = "grid"

[[outputs]]
id = "left"
width = 1920
height = 1080
[outputs.blend]
right = { width = 0.2, curve = "smooth" }
gamma = 2.2

[[surfaces]]
id = "wall-left"
output = "left"
source = "grid"
crop = { x = 0.0, y = 0.0, w = 0.6, h = 1.0 }
warp = { kind = "quad", corners = [{ x = 0.05, y = 0.08 }, { x = 0.98, y = 0.04 }, { x = 0.96, y = 0.95 }, { x = 0.03, y = 0.9 }] }
```

See `examples/two-projector-demo.lumen.toml` for the full demo.

## Development

```bash
cd lumen
cargo xtask ci          # fmt, clippy -D warnings, docs, cargo-deny, tests, coverage floors
```

Crates: `lumen-core` (geometry, blending, project format, undo), `lumen-sources`,
`lumen-render` (software oracle + wgpu), `lumen-control` (OSC), `lumen-app` (editor, show
player, CLI). `CLAUDE.md` describes the layout and conventions in more detail;
`CONTRIBUTING.md` the workflow.

Headless GPU tests need a Vulkan adapter; on Linux without a GPU install Mesa's lavapipe
(`mesa-vulkan-drivers`) and run with `WGPU_BACKEND=vulkan`.

## Roadmap

Video sources (FFmpeg or GStreamer behind a feature), native Spout receiver and sender, NDI,
MIDI learn, camera-assisted calibration, 3-D model mapping, DMX/Art-Net, output windows on
macOS and Linux with per-monitor DPI, LED pixel mapping.

## Licence

MIT OR Apache-2.0, at your option.
