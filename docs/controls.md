# Controls

The bindings below are the defaults in `crates/engine/src/input.rs` (`Bindings::default`). Keep this file in step with that table whenever a binding is added or changed.

## Movement and view

| Input | Action |
|---|---|
| W / S / A / D | Move forward / back / left / right |
| Mouse | Look (click the window to grab the cursor) |
| Left Shift | Sprint (run in walk mode, faster in fly mode) |
| Space | Up (fly mode); call a lift up |
| Left Ctrl | Down (fly mode); call a lift down |
| E | Interact: call a lift at an opening |
| F4 | Toggle fly / walk mode |
| P | Toggle photo mode: the player stands still, the camera roams within 40 m (WASD, Space, Ctrl, Shift), HUD hidden. PageUp / PageDown change FOV, `.` / `,` exposure (half stops, ±3) |
| B | Drop a beacon at your feet (saved to `save.toml`, or `SUPERSTRUCT_SAVE`) |
| `=` / `-` | Fly speed faster / slower |
| Esc | Release the cursor |

## Teleport (debug)

| Input | Action |
|---|---|
| F5 | Teleport to the nearest port |
| F6 | Teleport to a sanctuary (repeat to cycle) |

## Display and debug

| Input | Action |
|---|---|
| F1 | Toggle the overlay (timings, position, state) |
| F2 | Cycle vsync mode |
| F3 | Toggle debug boxes |
| F7 | Freeze culling (keeps the old frustum) |
| F11 | Toggle fullscreen |
| F12 | Screenshot to `out/`, with a `.toml` sidecar (pose, seed, `GEN_VERSION`, bands, overlay readings) that `SUPERSTRUCT_SHOT` replays |
| Tab | Toggle the altimeter and coordinates readout (absolute metres) |

## Stylization prototypes (P4.3.3)

| Input | Action |
|---|---|
| F8 | Toggle posterize |
| F9 | Toggle ordered dither |
| F10 | Toggle ink outlines |
| G | Toggle film grain |

## Environment switches

Set before launch (`scripts/win.sh run` forwards them):

| Variable | Effect |
|---|---|
| `SUPERSTRUCT_LOG` | Tracing filter |
| `SUPERSTRUCT_SETTINGS` | Settings file |
| `SUPERSTRUCT_AUTOSHOT=<frames>` | Save a screenshot after that many frames, then exit. `settle` (or `settle:<max frames>`) waits until streaming has been idle for 60 frames |
| `SUPERSTRUCT_SHOT=<sidecar.toml>` | Start from a screenshot's sidecar: seed, window size, FOV, bands, fly or walk, position and view |
| `SUPERSTRUCT_SHOT_NAME=<name>` | Save screenshots as `out/<name>.png` (and `.toml`), overwriting |
| `SUPERSTRUCT_OVERLAY=1` | Start with the overlay on |
| `SUPERSTRUCT_PHOTO=<stops>` | Start photo mode at frame 2 with that exposure |
| `SUPERSTRUCT_READOUT=1` | Start with the altimeter readout on |
| `SUPERSTRUCT_DEBUG_BOXES=1` | Start with debug boxes on |
| `SUPERSTRUCT_CELL=x,y,z` | Start beside that L2 cell |
| `SUPERSTRUCT_SECTOR=x,y,z` (+ `SUPERSTRUCT_LOCAL=x,y,z`) | Teleport to a sector and local position |
| `SUPERSTRUCT_YAW=<deg>`, `SUPERSTRUCT_PITCH=<deg>` | Start orientation with `SUPERSTRUCT_SECTOR` (yaw 0 looks along -z, 90 along -x, 180 along +z) |
| `SUPERSTRUCT_WALK=1` | Start in walk mode (default is fly) |
| `SUPERSTRUCT_AUTOFLY=<m/s>` | Fly level along the view at a constant speed |
| `SUPERSTRUCT_TRAM`, `SUPERSTRUCT_HITCH` | Tram and frame-hitch diagnostics |
| `SUPERSTRUCT_POST=posterize,dither,outlines,grain` or `all` | Start with stylization passes on |

Bindings can be changed in code with `Bindings::rebind`; a settings-file rebind is not implemented yet.
