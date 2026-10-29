# lumen

Projection mapping that you can trust on show day, and that a first-timer can run in two
minutes.

lumen maps video, images, loops and live inputs onto physical surfaces through one or many
projectors. It is a native Rust application (egui editor, wgpu renderer) with a few deliberate
differences from the usual tools:

- **Easiest calibration.** Plug in the projectors, open lumen, click *Create outputs*: every
  display gets a full-size output with a grid. Drag corners on the canvas or directly on the
  wall, with zoom, snapping, numeric readouts and an *Identify* flash so four projectors never
  get mixed up. The stage view shows the whole wall to scale; *Suggest blends* sets the
  overlap fades for you; calibration overlays and a gamma-match tool hide the seams.
- **Provably correct geometry.** Corner-pin warps are true homographies with
  perspective-correct sampling; curved surfaces use bicubic Bezier meshes; overlaps blend in
  linear light with gamma-correct falloffs. A CPU reference renderer is the specification,
  golden images come from it, and the GPU renderer is compared against it on every push.
- **A real show, not a demo.** Video through `ffmpeg`, image sequences and GIFs, six animated
  generators, Spout senders on Windows; cues with crossfades; GO from the keyboard, OSC, a
  MIDI controller (learn any fader or button) or a phone on the Wi-Fi; OSC feedback so your
  TouchOSC layout follows the app.
- **Show-day reliability.** Locked show mode (double-Escape to leave), autosave with crash
  recovery, atomic project writes, a render-loop watchdog that can relaunch the show, and a
  startup self-check that opens every enabled output where the project says it belongs.
- **Plain-text projects.** `.lumen.toml` files are small, diffable and versioned with
  migrations, so a show lives happily in git. Cues and MIDI maps travel with it.
- **Scriptable, testable, free.** `lumen render` draws any output to PNG without a window; the
  whole app, UI included, is covered by headless tests. MIT OR Apache-2.0, one executable, no
  watermark, no subscription.

| Left output of the demo, blended right edge | Right output: overlap, screen-mode logo, feathered mask | Bezier mesh on a curve |
|---|---|---|
| ![left](docs/images/left.png) | ![right](docs/images/right.png) | ![bezier](docs/images/bezier.png) |

| Six generators at one instant | Grid overlay on a checker (never shown in show mode) |
|---|---|
| ![generators](docs/images/generators.png) | ![overlay](docs/images/overlay-grid.png) |

## Two-minute start

1. Download `lumen-<version>-windows-x64.zip` from the releases (tags `lumen-v*`), unzip,
   run `lumen.exe`. Or build: `cd lumen && cargo build --release -p lumen-app --features spout`.
2. The welcome screen lists your displays. Click **Create N output(s) with a test grid**: each
   secondary display gets a borderless output window showing an alignment grid.
3. Press **I** to flash each output's name on its projector. Drag the four corner handles on
   the canvas (or on the wall itself) until the grid sits on the surface. Scroll to zoom,
   **G** for a grid, **S** for snapping, arrows to nudge.
4. **Add media…** (or drop files on the window) and pick the source in the surface inspector.
5. Set up a look, press **Store cue**, change something, store another. **Space** fades
   between them. **F10** locks the show. Ctrl+S saves everything to one TOML file.

The full guide is [docs/manual.md](docs/manual.md).

## Use

```text
lumen                              open the editor (welcome screen, or the demo)
lumen show.lumen.toml              open a project
lumen show show.lumen.toml         straight into locked show mode, outputs open
lumen show show.lumen.toml --remote-port 8080 --osc-port 9000 --restart-on-stall 5
lumen render show.lumen.toml --out frames/     PNG per output, headless
lumen validate show.lumen.toml     check references and geometry (exit 1 on errors)
lumen migrate old.lumen.toml       rewrite in the current schema
lumen new my-show.lumen.toml       write the two-projector demo
```

### Objects

- **Outputs** are projectors: size, enabled-in-show, colour (brightness, contrast, gamma),
  four-edge blend (width, curve, blend gamma), placement on a monitor, and a calibration
  overlay (grid, crosshair, colour bars, grey steps, blend check).
- **Surfaces** place a source on an output: corner-pin or Bezier mesh, opacity, z-order,
  blend mode, crop (drag it on the preview) and feathered polygon masks. Lock, duplicate,
  rename, reorder, align (flip, rotate, fit, centre, match aspect).
- **Sources**: test patterns, solid colours, PNG/JPEG, video (any format `ffmpeg` reads),
  numbered image sequences, GIFs, generators (plasma, noise, colour cycle, strobe, radar,
  grid pulse) and Spout senders. Video, sequences and generators share a transport (play,
  loop, speed, restart).
- **Cues** snapshot every live parameter with a fade time. GO fades numbers and switches
  booleans half way.

### Control

| Way in | What it does |
|---|---|
| Keyboard | `Space` GO, `B` blackout, `I` identify, `T` stage view, `[`/`]` blend gamma, `?` the full list |
| OSC (`--osc-port`, default 9000) | `/show/blackout`, `/show/master`, `/surface/<id>/opacity`, `/output/<id>/blend/<edge>/width`, `/source/<id>/play`, `/cue/go`, `/cue/<id>/go`, `/cue/stop` … every live parameter |
| OSC feedback | Show inspector → *Feedback host:port*: every change is echoed back |
| MIDI | Show inspector → open a port, type an address (or press *MIDI learn opacity* on a surface), move a control. Fader, button and switch modes; saved in the project |
| Phone | *Start remote* (or `--remote-port 8080`), scan the QR code: blackout, master, levels, opacities and the cue list from any browser |

### Project file

```toml
schema_version = 1
name = "Two projector demo"

[[sources]]
id = "loop"
kind = "video"
path = "media/loop.mp4"
playback = { looping = true, speed = 1.0 }

[[outputs]]
id = "left"
width = 1920
height = 1080
placement = { monitor = '\\.\DISPLAY2', x = 1920, y = 0 }
[outputs.blend]
right = { width = 0.2, curve = "smooth" }
gamma = 2.2

[[surfaces]]
id = "wall-left"
output = "left"
source = "loop"
warp = { kind = "quad", corners = [{ x = 0.05, y = 0.08 }, { x = 0.98, y = 0.04 }, { x = 0.96, y = 0.95 }, { x = 0.03, y = 0.9 }] }

[[cues]]
id = "opening"
fade_seconds = 3.0
[cues.values]
"/show/master" = 1.0
"/surface/wall-left/opacity" = 1.0
```

See `examples/two-projector-demo.lumen.toml` for the full demo.

## Development

```bash
cd lumen
cargo xtask ci          # file lengths, fmt, clippy -D warnings, docs, cargo-deny, tests, coverage floors
```

Crates: `lumen-core` (geometry, blending, project format, cues, MIDI map, undo),
`lumen-sources` (patterns, images, video, sequences, generators, Spout), `lumen-render`
(software oracle + wgpu), `lumen-control` (OSC in and out, MIDI, the phone remote),
`lumen-app` (editor, show player, CLI). `CLAUDE.md` describes the layout and conventions;
`CONTRIBUTING.md` the workflow.

Headless GPU tests need a Vulkan adapter; on Linux without a GPU install Mesa's lavapipe
(`mesa-vulkan-drivers`) and run with `WGPU_BACKEND=vulkan`. Video tests need `ffmpeg`; MIDI
needs `libasound2-dev` to build on Linux.

Spout is compiled on Windows (MSVC, x64) from the vendored Spout2 SDK and needs a GPU at run
time; CI builds it, a projector proves it.

## Roadmap

NDI, Spout sending, camera-assisted calibration, 3-D model mapping, DMX/Art-Net, LED pixel
mapping, output windows on macOS and Linux with per-monitor DPI.

## Licence

MIT OR Apache-2.0, at your option. Spout2 (vendored by `spout2-rs`) is BSD-2-Clause.
