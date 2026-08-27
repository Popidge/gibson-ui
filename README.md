# GIBSON

"...i'm in this computer, right, so i'm lookin' around, i'm lookin' around, yknow, throwin' commands at it, i don't know where it is or what it does or anything... it's like-it's like *choice*, **it's just beautiful**..."

	- **Joey Pardella** *(as played by Jesse Bradford)*, *Hackers* (1995)

GIBSON is a cinematic file-system visualiser with a real Bash terminal and a simple file navigator, inspired by the UI of the Gibson Supercomputer in Iain Softley's 1995 film, Hackers.

The visualiser shows a bounded part of the directory tree as a city. Each directory is a tower. Each direct child is a storey.

The terminal and file navigator control the working directory. The visualiser shows each journey through the city.

This repository does not contain the Omarchy package or release installer.

## Current capabilities

GIBSON contains these working parts:

- A native Wayland window and a `wgpu` renderer.
- A real Bash PTY that reads the user `~/.bashrc` file.
- Bash working-directory reports through OSC 7.
- File-navigator control of Bash through a private Readline binding.
- ANSI foregrounds, backgrounds, attributes, Unicode text, scrollback, and a cursor.
- A fixed terminal cell grid for Nerd Font and Powerline prompts.
- A responsive file menu on the active tower face.
- Background file metadata and bounded directory previews.
- A bounded city with one ancestor tier and two descendant tiers.
- Stable directory positions during the application session.
- Direct flights for file-navigator changes.
- Complete directory routes for terminal changes.
- Reversible camera paths and distance-based flight times.
- Presentation-only orbit controls for the current tower.
- Live create, remove, rename, move, and modify effects.
- An ambient suspense track, flight sounds, and file-navigator sounds.
- Smooth audio fade when the application loses focus.
- A wide cockpit layout and a compact terminal drawer.
- Resizable panes with saved positions and visibility.
- A built-in settings screen with automatic file storage.
- Omarchy palette and background tracking.
- Four graphics-quality levels and four frame-rate modes.
- A compact visualiser HUD with live performance data.
- Optional frame-time logs for performance analysis.

The file navigator does not change file-system data. Use the terminal to create, rename, move, copy, or remove items.

## Requirements

The development build requires these components:

- An Omarchy or Linux Wayland session.
- A Vulkan-capable GPU and driver.
- Rust 1.94 or a compatible recent toolchain.
- Bash at `/bin/bash` for the test suite.
- Development files for Wayland and `xkbcommon`.

## Build and start

1. Build the release binary:

   ```bash
   cargo build --release
   ```

2. Start GIBSON at the current directory:

   ```bash
./target/release/gibson-ui .
   ```

Start GIBSON at a different directory:

```bash
./target/release/gibson-ui ~/Dev
```

Start GIBSON in fullscreen mode:

```bash
./target/release/gibson-ui --fullscreen .
```

Start GIBSON on an empty Hyprland workspace:

```bash
./target/release/gibson-ui --cockpit .
```

Show the direct children of `/`:

```bash
./target/release/gibson-ui --root
```

Remove the application frame limit:

```bash
./target/release/gibson-ui --uncapped --perf .
```

The uncapped mode requests an immediate or mailbox present mode. The driver can fall back to a supported mode.

## Controls

GIBSON starts with terminal control. Press `F2` to give control to the file navigator.

| Input | Result |
|---|---|
| `F2` | Change between terminal control and file-navigator control |
| `F3` | Show or hide the terminal pane |
| `F4` | Show or hide the file navigator |
| `F10` | Open or close the settings screen |
| `F11` | Change the fullscreen state |
| Up or Down | Select a file-navigator item |
| Left or Right | Orbit by one tower face |
| `R` | Return the orbit to the initial face |
| Page Up or Page Down | Move eight file-navigator items |
| Home or End | Select the first or last item |
| `Enter` on a directory | Change to the directory through a direct flight |
| `Enter` on a file | Open the file with the default desktop application |
| `Backspace` | Change to the parent directory |
| `Escape` | Give control to the terminal |
| Left click | Select the panel that receives input |
| Left drag on a splitter | Resize the cockpit panes |
| Mouse wheel | Change the selected file |

The settings screen uses Up and Down for selection. It uses Left and Right to change a value.

## Settings

GIBSON stores settings in `~/.config/gibson/gibson.toml`.

GIBSON creates this file after a setting changes. The default values do not require a file.

```toml
[graphics]
quality = "high"
frame_rate = "fps60"
floor_pulses = true
scanlines = true
motion_scale = 1.0
performance_log = false

[appearance]
follow_omarchy = true
backdrop = "tint"

[behaviour]
orbit_enabled = true
idle_sweep = true

[audio]
enabled = true
master_volume = 0.65
```

The backdrop accepts `off`, `tint`, or `wallpaper`. Wallpaper mode loads the current Omarchy background.

The frame rate accepts `fps30`, `fps60`, `fps120`, or `unlimited`.

## Omarchy integration

GIBSON reads the current palette from `~/.local/state/omarchy/current/theme/colors.toml`.

GIBSON reads the selected background through `~/.local/state/omarchy/current/background`.

The application examines these files every 750 ms. A theme change updates the running application without an installed hook.

GIBSON reads the terminal palette and font from the user Foot file. Included Foot files are supported.

The integration does not change files in `~/.config/omarchy`, `~/.config/foot`, or `/usr/share/omarchy`.

## File-system effects

The file-system watcher compares each new directory snapshot with the prior snapshot.

| Change | Visual response |
|---|---|
| Create or copy | A construction beam and a rising green storey pulse |
| Remove | A red storey collapses and expands |
| Rename or move | A magenta transfer marker moves between storeys |
| Modify | Two blue markers orbit the changed storey |

The final directory snapshot always defines the scene. An effect cannot replace file-system state.

## Performance controls

The `performance`, `balanced`, `high`, and `cinematic` levels set object and label limits.

The top HUD shows measured frame rate, CPU frame time, and visible scene counts. Narrow views use a shorter form.

Use `--perf` or enable `performance_log` to print frame statistics every two seconds.

The log contains frame rate, CPU frame time, scene time, label time, text time, and visible counts.

## Synchronisation model

Bash emits the current directory through OSC 7 at each prompt. GIBSON changes the file watch after it receives this path.

The file navigator writes an exact path to a user-only control file. A private Readline binding reads the path and calls Bash `cd`.

A file-navigator change uses one direct camera flight. A terminal change walks through the nearest common parent.

The same city path is used in both directions. Thus, a reverse journey retraces the first journey.

## Tests

Run the unit and integration tests:

```bash
cargo test
```

Run the strict static analysis:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

The suite covers these areas:

- Directory scans, live watches, and semantic snapshot differences.
- OSC paths, terminal ANSI styles, and Powerline backgrounds.
- Real Bash startup and two-way directory changes.
- Tower layout, storey order, occlusion, and camera paths.
- Orbit face selection and frame present modes.
- Settings limits, menu changes, and Omarchy palette parsing.
- Audio asset decoding and flight-cue selection.

## Source layout

| Module | Responsibility |
|---|---|
| `audio.rs` | Audio mixer, focus fade, music, and event sounds |
| `config.rs` | Persistent settings and the settings menu model |
| `filesystem.rs` | Directory scans, `inotify`, and semantic changes |
| `layout.rs` | Cockpit panes, splitters, and saved layout values |
| `main.rs` | Window events, input routing, and component coordination |
| `navigation.rs` | Transit and settled visual states |
| `navigator.rs` | File selection, metadata, previews, and file actions |
| `renderer.rs` | `wgpu`, camera state, theme layers, terminal cells, and text |
| `scene.rs` | Towers, storeys, flights, connections, and file effects |
| `terminal.rs` | PTY ownership, Bash integration, ANSI state, and terminal snapshots |
| `theme.rs` | Omarchy palette, background, and live theme tracking |
| `topology.rs` | Background indexing, bounded city selection, and stable slots |

## Current limits

- Bash is the only shell with two-way directory synchronisation.
- The terminal does not support text selection or the clipboard.
- The directory index stops after 250,000 directories.
- The visualiser shows a maximum of 128 towers at one time.
- Tower sizes use direct-child data instead of recursive disk usage.
- Tower labels use screen-facing text instead of textured 3D geometry.
- The renderer does not contain a bloom post-process.
- The repository does not contain the Omarchy package, desktop entry, icon, or installer.

## Bundled assets

The tower interface uses Michroma by the Michroma Project Authors.

The font uses the SIL Open Font License 1.1. See `assets/fonts/OFL.txt`.

The development tree contains Mixkit Free sound effects. The release binary embeds compressed Ogg versions.
