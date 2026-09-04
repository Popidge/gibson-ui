# Changelog

This project uses [Semantic Versioning](https://semver.org/).

## Unreleased

## 0.3.1 - 2026-09-05

- Reduced terminal reshaping, navigator snapshot work, and repeated text preparation.
- Reduced film-mode scene-copy costs on supported graphics devices.
- Paused inaudible music to avoid unnecessary decoding and mixing.
- Added optional GPU pass timings to performance logging.
- Fixed terminal snapshot consistency during concurrent output and first-row text shaping.
- Fixed Unicode color parsing and restored classic file-change effects.
- Removed dead rendering code and simplified hot paths.
- Added a checksum-verified installer for the latest GitHub release.

## 0.3.0 - 2026-08-29

- Fixed cockpit mode on Hyprland 0.55 and later.
- Added cached, depth-aware tower labels to the 3D scene pipeline.
- Fixed tower-label anchoring during orbit changes.
- Reduced per-frame text preparation and file-path comparison costs.
- Improved camera framing for sparse and tall directories.
- Mapped the complete visualiser palette to the active Omarchy theme.
- Added live theme and wallpaper updates to a running session.
- Extended the Omarchy wallpaper across the complete GIBSON window.
- Aligned the wallpaper with its monitor position during window moves and layout changes.

## 0.2.0 - 2026-08-29

- Added the 1995 film visual style with glass towers and a circuit-board floor.
- Added a GPU blur behind translucent tower surfaces.
- Moved the tower-face navigator into the 3D scene pipeline.
- Added depth-aware GPU text for tower labels.
- Added tower lightning that responds to CPU and scheduler load.
- Improved navigator-to-terminal directory changes with prompt acknowledgement and queued requests.
- Fixed Omarchy backgrounds that use extensionless symbolic links.
- Changed the visualiser HUD to focus on frame rate and controls.

## 0.1.0 - 2026-08-27

- Added the native `wgpu` file-system city visualiser.
- Added the Bash terminal and two-way directory synchronisation.
- Added the tower-face file navigator and metadata panel.
- Added stable city routes, cinematic flights, and tower orbit controls.
- Added visual effects for common file-system changes.
- Added the ambient soundscape, flight cues, and focus fades.
- Added responsive cockpit layouts and persistent settings.
- Added live Omarchy palette, background, and Foot integration.
- Added graphics-quality, frame-rate, and performance controls.
- Added the desktop launcher and Arch binary package recipe.
