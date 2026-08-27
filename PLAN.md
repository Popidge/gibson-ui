# GIBSON 0.1 Plan

## Product definition

GIBSON turns directory changes into journeys through a cinematic file-system city.

The terminal and file navigator control the working directory. The 3D scene does not act as a spatial file-system controller.

Each directory is a tower. The direct children of the working directory are the storeys of the active tower.

The active tower contains the file navigator on its visible face. A floating panel shows metadata for the selected item.

## Product loop

The version 0.1 loop contains these actions:

1. The user starts `gibson .` from an Omarchy terminal.
2. GIBSON opens Bash with the normal user environment.
3. The user changes the directory in Bash or the file navigator.
4. The camera travels through the stable city grid.
5. The scene settles at the destination tower.
6. The user creates, copies, moves, modifies, or removes an item.
7. The scene shows the change and keeps the file-system snapshot authoritative.

## Implemented pre-release scope

The application contains these completed systems:

- A native Wayland window with a Rust and `wgpu` renderer.
- A Bash PTY with OSC 7 directory reports.
- A terminal cell grid with ANSI backgrounds and Powerline support.
- A file navigator with metadata and directory previews.
- A bounded directory index and stable city-grid positions.
- Direct and hierarchical camera journeys.
- A tower-face file menu and an anchored metadata panel.
- Manual tower orbit and an optional idle sweep.
- Semantic effects for create, copy, remove, rename, move, and modify changes.
- An ambient music loop, flight sounds, navigator sounds, and focus fades.
- Wide and compact cockpit layouts with stored splitter positions.
- Persistent settings for graphics, motion, audio, theme, and frame rate.
- Live Omarchy palette and background tracking.
- Capped and uncapped frame scheduling with performance logs.
- Unit and integration tests for the application model.

## Architecture

The main thread owns the window, renderer, scene, navigator, settings, and audio controls.

Dedicated threads own the Bash PTY, file watch, topology index, and metadata previews.

The file watcher supplies complete snapshots and semantic differences. The snapshot always defines the final scene state.

The topology index shows one ancestor tier and two descendant tiers. Hard limits prevent large dependency trees from filling the GPU buffers.

The renderer uses instanced cube geometry, GPU text, terminal background rectangles, a depth buffer, and theme-aware shaders.

## Settings model

GIBSON stores settings in `~/.config/gibson/gibson.toml`.

The `F10` screen changes and saves these values:

- Omarchy palette tracking.
- The theme tint or wallpaper.
- The graphics-quality level.
- The frame-rate limit.
- Floor pulses and panel scanlines.
- The animation scale.
- Performance logs.
- Orbit and idle-sweep behaviour.
- Audio state and volume.

## Performance target

The primary target is 60 frames per second on the six-year-old Radeon Vega reference laptop.

The default `high` level limits the scene to 800 render objects and 26 labels.

The `cinematic` level permits 960 render objects and 32 labels.

The `balanced` and `performance` levels provide more GPU headroom.

The `unlimited` frame-rate mode removes the application timer. It also requests a non-vsync present mode when the driver supports one.

## Safety and privacy

The file navigator opens files and changes directories. It does not create, move, copy, or remove file-system items.

File names do not enter generated shell source text. The Bash bridge reads exact paths from a user-only control file.

GIBSON does not send paths, commands, terminal output, or file names over the network.

The Omarchy integration reads current state. It does not change Omarchy, Hyprland, or terminal settings.

## Work that remains before release

Packaging and release work remains outside the implemented application scope:

- Select the final project name and application identifier.
- Select the launch surface and optional cockpit key binding.
- Add the desktop entry and application icon.
- Build the Omarchy and Arch installation flow.
- Define the binary update and removal flow.
- Record the audio asset notice and the source-asset policy.
- Measure release performance on the reference laptop.
- Run installation and removal tests on a clean Omarchy user profile.

## Release acceptance

Version 0.1 is ready for packaging when these statements are true:

- `cargo test` passes.
- Strict Clippy analysis passes without warnings.
- The release build completes with the locked dependencies.
- The Powerline test prompt matches the Foot cell layout.
- Directory journeys end at the correct city tower.
- Each semantic file change produces the correct effect.
- Audio fades when the application loses focus.
- Theme changes update the running application.
- The default level meets the frame-rate target on the reference laptop.

Packaging acceptance will add installation, startup, update, and removal tests.
