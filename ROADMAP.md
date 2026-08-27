# GIBSON roadmap

GIBSON 0.1 establishes the complete product loop.

The terminal and file navigator control the working directory. The visualiser turns each change into a city journey.

## Version 0.1

The first release includes these systems:

- Native Wayland rendering with Rust and `wgpu`.
- A Bash PTY with OSC 7 directory reports.
- ANSI, Unicode, Nerd Font, and Powerline terminal rendering.
- A file navigator with metadata and directory previews.
- A bounded directory index with stable city positions.
- Direct and hierarchical camera journeys.
- A tower-face file menu and floating metadata panel.
- Manual orbit and an optional idle sweep.
- Visual effects for common file-system changes.
- Music, flight sounds, navigator sounds, and focus fades.
- Responsive cockpit layouts with stored splitter positions.
- Persistent graphics, motion, audio, and theme settings.
- Live Omarchy palette and background tracking.
- Performance modes, frame limits, and live metrics.
- A desktop launcher and an AUR binary package.

## Later releases

The next work can improve these areas:

- Text selection and clipboard support in the terminal.
- More terminal escape-sequence and mouse support.
- A native bloom post-process.
- More file-system event animations.
- Optional application themes beyond Omarchy.
- More GPU and memory profiling on integrated graphics.
- A source-built AUR package if demand justifies its build time.

## Product rules

The visualiser presents file-system movement. It does not make 3D navigation a requirement.

The final directory snapshot always defines the scene. A visual effect cannot replace file-system state.

The city index stays bounded. Large dependency trees must not fill the renderer buffers.

Omarchy remains the primary desktop target. Other Arch Wayland systems receive a safe fallback experience.
