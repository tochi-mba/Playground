# lumen manual

This is the user guide. The README has the two-minute start; this page covers everything
else, in the order a show comes together.

## 1. Outputs and projectors

An **output** is one projector. Create them from the welcome screen (*Create N output(s) with a test
grid* makes one per secondary display, placed and full size) or with *+ output* in the tree.

- **Placement** ties an output to a monitor. *Fill monitor …* sets size and position from the
  OS; *Use current window position* takes the preview window where you dragged it. Placed
  outputs open borderless and always on top. Unplaced ones open as draggable previews.
- **Identify** (`I`) flashes every output's id, size and a colour on its projector for three
  seconds.
- **Edit on the wall**: while not in show mode, output windows draw the same handles as the
  canvas and accept the same drags and keys, so you can calibrate while looking at the
  surface. *View → Handles on output windows* turns it off. The cursor hides after two seconds
  of stillness on an output.
- **Stage view** (`T`) draws every placed output to scale on the virtual desktop with monitor
  outlines; overlaps are highlighted with their size in pixels. Click selects, drag moves the
  placement.
- **Colour**: brightness, contrast and gamma per output.

## 2. Surfaces and warps

A **surface** puts a source on an output. Add one with *+ surface* (an output must be
selected or exist).

- **Corner pin**: drag the four corners. Shift-drag moves the whole surface. Arrow keys nudge
  the active handle by one output pixel; Shift ×10, Alt ×0.1. The status bar shows the active
  handle in output pixels.
- **Bezier mesh**: *Convert to Bezier mesh* (choose columns and rows), then drag the control
  points. *Reset to flat mesh* and *Back to corner pin* are one click.
- **Canvas**: scroll to zoom, middle-drag or Ctrl-drag to pan, `0` fits. `G` toggles a grid in
  output space, `S` snapping to edges, centre and grid lines.
- **Align** menu: flip horizontal/vertical, rotate 90°, fit to output, centre, match the
  source's aspect.
- **Lock** (`L`) hides the handles so a finished surface cannot be nudged by accident.
  **Duplicate** (Ctrl+D), **Forward/Backward** change draw order, rename in the inspector,
  right-click a tree item or the canvas for the same actions.
- **Masks**: *Add mask* creates a feathered polygon (hole or window); drag its points on the
  canvas.
- **Opacity**, **blend mode** (normal, add, multiply, screen) and **crop** (drag the rectangle
  on the preview) are in the inspector.

## 3. Sources

*Add media…* opens a file dialog; *Add frame folder…* adds a numbered image sequence; or drop
files anywhere on the window.

| Kind | Notes |
|---|---|
| Test pattern | Grid, bars, checker, ramp. The grid has a crosshair and a circle to reveal aspect errors |
| Solid | Colour with alpha |
| Image | PNG, JPEG |
| Video | Any format `ffmpeg` decodes. If `ffmpeg` is not on the path, set it in the show inspector (saved in settings) |
| Sequence | Folder of numbered PNG/JPEG frames, or a GIF, with a frame rate |
| Generator | Plasma, noise, colour cycle, strobe, radar sweep, grid pulse; speed and tint |
| Spout | Windows builds: any Spout sender by name (empty = the active sender) |

Video, sequences and generators share a **transport**: play/pause, loop, speed, restart. They
are also parameters (`/source/<id>/play`, `/loop`, `/speed`, `/restart`) so cues, OSC and MIDI
drive them.

## 4. Blending and calibration overlays

Where two projectors overlap, each must fade out across the overlap so the light adds up to
one projector's worth.

1. Place both outputs (section 1) so their rectangles overlap on the desktop by the amount
   the projectors actually overlap on the wall.
2. Press **Suggest blends** in the show inspector. lumen sets left/right (or top/bottom) blend
   widths from the overlap.
3. Put the **Blend check** overlay on both outputs: flat white, so any band or seam is the
   blend's fault. Adjust the widths and curves until the seam is invisible.
4. **Gamma match**: press *Gamma match steps* on both outputs (grey steps overlay), then press
   `[` or `]` on the output shown in the canvas until the steps match across the seam. The
   blend gamma is the projector's gamma; 2.2 is typical.

Overlays are per output (grid, crosshair, colour bars, grey steps, blend check), saved in the
project, and never shown in show mode.

## 5. Cues and show control

A **cue** is a snapshot of every live parameter (opacities, visibility, master, blackout,
output levels, blend widths, transport) plus a fade time.

- **Store cue** in the cue panel at the bottom captures the current look after the selected
  cue. **Update cue** overwrites the selected cue with the live look. Cues can be reordered,
  renamed, given a new fade time and deleted.
- **GO** (`Space`, the GO button, the cue's own button, `/cue/go`, `/cue/<id>/go`, a MIDI
  button, the phone) fades to the next cue: numbers interpolate, booleans switch half way.
  **Stop fade** freezes a fade where it is.
- **Blackout** (`B`) and **master** are always live and are captured by cues too.

### OSC

`--osc-port 9000` (default; `0` disables). One message per parameter, the first argument is
the value; a message with no arguments means `true`.

```text
/show/blackout bool           /show/master 0..1
/surface/<id>/opacity 0..1    /surface/<id>/visible bool      /surface/<id>/z int
/surface/<id>/corner/<0-3>/x|y 0..1 (quads)
/output/<id>/enabled bool     /output/<id>/brightness|contrast|gamma
/output/<id>/blend/<left|right|top|bottom>/width 0..0.5    /output/<id>/blend/gamma
/source/<id>/play bool        /source/<id>/loop bool    /source/<id>/speed    /source/<id>/restart
/cue/go                       /cue/<id>/go              /cue/stop
```

**OSC feedback**: type `host:port` under *OSC feedback* in the show inspector and *Apply*.
Every change (from the UI, cues, MIDI or the remote) is sent back as the same address and
value, so a TouchOSC or Lemur layout follows the app.

### MIDI learn

1. Show inspector → *Refresh MIDI ports*, pick one, *Open MIDI port* (saved in settings).
2. Type the address to drive (or press *MIDI learn opacity* in a surface inspector, which
   fills it in) and choose **Learn fader** (0–127 scaled to the parameter), **Learn button**
   (sends `true` on press, for GO and restart) or **Learn switch** (`true` on press, `false` on
   release, for blackout and visibility).
3. Move the control. The mapping is saved in the project; *Forget* removes it.

### Phone remote

*Start remote* in the show inspector (or `lumen show … --remote-port 8080`) serves a page on
the LAN; scan the QR code with a phone. The page has blackout, master, output levels,
surface opacities and the cue list with GO. It is plain HTML and JavaScript, no app, no
internet. The JSON API behind it: `GET /api/state`, `POST /api/param {address, value}`,
`POST /api/cue/go {id?}`.

## 6. Show mode

`F10`, *Show mode* in the status bar, or `lumen show project.lumen.toml`. Editing is locked,
every enabled output opens where it is placed, OSC/MIDI/remote stay live, overlays are
hidden, autosave pauses, the editor window shows status only. Escape twice leaves.

`--restart-on-stall 5` relaunches the process if the render loop stops beating for five
seconds. Autosaves go next to the project; on the next open lumen offers to recover one.

## 7. Settings and files

Preferences live in `settings.toml` in the OS config directory (`%APPDATA%\lumen` on
Windows; `LUMEN_CONFIG_DIR` overrides): recent files, OSC port, MIDI port, OSC feedback
target, remote port, grid, snap, edit-on-wall, autosave interval, ffmpeg path and whether to
skip the welcome screen.

Projects are TOML with a `schema_version`; older files are migrated on load and
`lumen migrate` rewrites them. Media paths are stored relative to the project file when
possible. `lumen validate` reports every broken reference or degenerate warp.

## 8. Keyboard

| Key | Action |
|---|---|
| Drag handle / Shift+drag | Move a point / the whole surface or mask |
| Arrows (+Shift, +Alt) | Nudge 1 px (×10, ×0.1) |
| Scroll, middle-drag, `0` | Zoom, pan, fit |
| `G` / `S` | Grid / snapping |
| `L`, Ctrl+D, Delete | Lock, duplicate, delete |
| Ctrl+Z / Ctrl+Y | Undo / redo (a drag is one step) |
| Ctrl+N / Ctrl+O / Ctrl+S / Ctrl+Shift+S | New / open / save / save as |
| `I` | Identify outputs |
| `T` | Stage view |
| `[` / `]` | Blend gamma of the shown output |
| `B` | Blackout |
| `Space` | GO |
| `F10`, Escape Escape | Show mode in, out |
| `?` | This list, in the app |
