mod audio;
mod config;
mod filesystem;
mod layout;
mod navigation;
mod navigator;
mod renderer;
mod scene;
mod system_load;
mod terminal;
mod theme;
mod topology;

use anyhow::{Context, bail};
use audio::Soundscape;
use clap::Parser;
use config::{FrameRate, Settings, SettingsMenu};
use filesystem::{FileEntry, FileMutation, FsEvent, FsWatcher};
use layout::PaneTarget;
use navigation::VisualState;
use navigator::{Navigator, NavigatorAction};
use renderer::{RenderStatus, Renderer};
use scene::Scene;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use terminal::{Terminal, TerminalEvent};
use theme::ThemeWatcher;
use topology::{
    DirectoryChild, DirectoryIndex, DirectoryScan, TopologyEvent, TopologyIndexer,
    affects_visible_city, hierarchy_route,
};
use tracing::{error, info, warn};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::window::{Fullscreen, Window};

#[derive(Parser, Debug)]
#[command(version, about = "Travel through the file system as a cinematic city")]
struct Args {
    /// Directory to show and use as the shell working directory.
    path: Option<PathBuf>,

    /// Open the top level of the machine.
    #[arg(long)]
    root: bool,

    /// Start in borderless fullscreen mode.
    #[arg(long)]
    fullscreen: bool,

    /// Start fullscreen and use an empty Hyprland workspace when available.
    #[arg(long)]
    cockpit: bool,

    /// Set a 30, 60, or 120 FPS application limit.
    #[arg(long, value_parser = clap::value_parser!(u16).range(30..=120), conflicts_with = "uncapped")]
    fps: Option<u16>,

    /// Render continuously and use a non-vsync present mode when available.
    #[arg(long)]
    uncapped: bool,

    /// Log renderer timings and object counts every two seconds.
    #[arg(long)]
    perf: bool,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut settings = Settings::load();
    let theme_watcher = ThemeWatcher::new();
    let omarchy_available = theme_watcher.is_available();
    settings.adapt_to_environment(omarchy_available);
    settings.graphics.frame_rate = if args.uncapped {
        FrameRate::Unlimited
    } else {
        match args.fps {
            Some(30) => FrameRate::Fps30,
            Some(60) => FrameRate::Fps60,
            Some(120) => FrameRate::Fps120,
            Some(value) => bail!("--fps accepts 30, 60, or 120; received {value}"),
            None => settings.graphics.frame_rate,
        }
    };
    settings.graphics.performance_log |= args.perf;
    let mut log_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "gibson_ui=info,wgpu_core=warn,wgpu_hal=warn".into());
    if settings.graphics.performance_log {
        log_filter = log_filter.add_directive(
            "gibson_ui=info"
                .parse()
                .expect("the built-in performance log directive must be valid"),
        );
    }
    tracing_subscriber::fmt()
        .with_env_filter(log_filter)
        .with_target(false)
        .compact()
        .init();

    let requested = if args.root {
        PathBuf::from("/")
    } else {
        args.path
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."))
    };
    let root = requested
        .canonicalize()
        .with_context(|| format!("cannot open {}", requested.display()))?;
    if !root.is_dir() {
        bail!("{} is not a directory", root.display());
    }
    if args.cockpit {
        enter_cockpit_workspace();
    }

    let terminal = Terminal::spawn(&root).context("start the built-in terminal")?;
    let watcher = FsWatcher::spawn(root.clone()).context("start the file-system watcher")?;
    let topology_indexer =
        TopologyIndexer::spawn(root.clone()).context("start the topology indexer")?;
    let topology = DirectoryIndex::new(&root);
    let mut scene = Scene::new(root.clone());
    scene.set_motion_scale(settings.graphics.motion_scale);
    let event_loop = EventLoop::new().context("create Wayland event loop")?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let soundscape = match Soundscape::new() {
        Ok(mut soundscape) => {
            soundscape.apply_settings(settings.audio.enabled, settings.audio.master_volume);
            info!("soundscape ready");
            Some(soundscape)
        }
        Err(error) => {
            warn!(%error, "audio unavailable; continuing without sound");
            None
        }
    };
    let mut app = App {
        initial_root: root.clone(),
        start_fullscreen: args.fullscreen || args.cockpit,
        window: None,
        renderer: None,
        terminal,
        watcher,
        topology_indexer,
        topology,
        navigator: Navigator::new(root).context("start the file preview worker")?,
        journey: VecDeque::new(),
        pending_snapshot: None,
        next_frame: Instant::now(),
        scene,
        soundscape,
        settings_menu: SettingsMenu::new(settings, omarchy_available),
        theme_watcher,
        input_target: InputTarget::Terminal,
        requested_navigator_path: None,
        pending_navigation: None,
        shell_at_prompt: false,
        modifiers: ModifiersState::default(),
        cursor_position: PhysicalPosition::new(0.0, 0.0),
        shell_exited: false,
    };
    event_loop.run_app(&mut app).context("run GIBSON")?;
    Ok(())
}

struct App {
    initial_root: PathBuf,
    start_fullscreen: bool,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    terminal: Terminal,
    watcher: FsWatcher,
    topology_indexer: TopologyIndexer,
    topology: DirectoryIndex,
    navigator: Navigator,
    journey: VecDeque<PathBuf>,
    pending_snapshot: Option<PendingSnapshot>,
    next_frame: Instant,
    scene: Scene,
    soundscape: Option<Soundscape>,
    settings_menu: SettingsMenu,
    theme_watcher: ThemeWatcher,
    input_target: InputTarget,
    requested_navigator_path: Option<PathBuf>,
    pending_navigation: Option<(PathBuf, NavigationSource)>,
    shell_at_prompt: bool,
    modifiers: ModifiersState,
    cursor_position: PhysicalPosition<f64>,
    shell_exited: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputTarget {
    Terminal,
    Navigator,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NavigationSource {
    Navigator,
    Terminal,
}

impl App {
    fn drain_events(&mut self) {
        let mut topology_changed = false;
        for event in self.topology_indexer.event_rx.try_iter().take(128) {
            match event {
                TopologyEvent::Scan(scan) => {
                    let relevant = affects_visible_city(self.scene.root(), &scan.path);
                    topology_changed |= self.topology.ingest(scan) && relevant;
                }
                TopologyEvent::Complete { indexed, capped } => {
                    info!(indexed, capped, "file-system topology index ready");
                }
            }
        }
        if topology_changed {
            let layout = self.topology.visible_layout(self.scene.root());
            self.scene.apply_city_layout(&layout);
        }

        while let Ok(event) = self.watcher.event_rx.try_recv() {
            match event {
                FsEvent::Snapshot {
                    root,
                    entries,
                    mutations,
                } => self.accept_snapshot(root, entries, mutations),
                FsEvent::Error(message) => {
                    error!("{message}");
                }
            }
        }

        while let Ok(event) = self.terminal.event_rx.try_recv() {
            match event {
                TerminalEvent::Cwd(path) => {
                    if path.is_dir() && path != self.scene.root() {
                        let source = if self.requested_navigator_path.as_ref() == Some(&path) {
                            self.requested_navigator_path = None;
                            NavigationSource::Navigator
                        } else {
                            self.requested_navigator_path = None;
                            NavigationSource::Terminal
                        };
                        self.pending_navigation = Some((path.clone(), source));
                        info!(path = %path.display(), "shell changed directory");
                        self.watcher.set_root(path.clone());
                        self.topology_indexer.prioritise(path.clone());
                    } else if self.requested_navigator_path.as_ref() == Some(&path) {
                        self.requested_navigator_path = None;
                    }
                }
                TerminalEvent::Prompt => {
                    self.shell_at_prompt = true;
                    self.scene.finish_command();
                }
                TerminalEvent::Exited => {
                    self.shell_exited = true;
                }
                TerminalEvent::Error(message) => {
                    error!("{message}");
                }
            }
        }

        self.navigator.poll();
        self.sync_navigator_selection();
        self.advance_journey();
        if let Some(theme) = self.theme_watcher.poll(Instant::now()).cloned()
            && let Some(renderer) = &mut self.renderer
        {
            renderer.apply_omarchy_theme(&theme);
            info!(theme = %theme.name, "Omarchy theme applied");
        }
        let flying = self.visual_state() == VisualState::Transit;
        if let Some(soundscape) = &mut self.soundscape {
            soundscape.update(flying);
        }
    }

    fn accept_snapshot(
        &mut self,
        root: PathBuf,
        entries: Vec<FileEntry>,
        mutations: Vec<FileMutation>,
    ) {
        self.topology.ingest(directory_scan(&root, &entries));
        self.navigator.update(root.clone(), entries.clone());
        let root_changed = root != self.scene.root();
        let source = self
            .pending_navigation
            .take_if(|(target, _)| target == &root)
            .map_or(NavigationSource::Terminal, |(_, source)| source);
        let route = if root_changed && source == NavigationSource::Terminal {
            hierarchy_route(self.scene.root(), &root)
        } else {
            Vec::new()
        };
        if route.len() > 1 {
            self.journey = route.into();
            self.pending_snapshot = Some(PendingSnapshot {
                root,
                entries,
                mutations,
            });
            self.advance_journey();
            return;
        }
        self.journey.clear();
        self.pending_snapshot = None;
        if root_changed {
            self.scene.set_visual_state(VisualState::Transit);
        }
        self.scene.update(root.clone(), entries, &mutations);
        if root_changed {
            self.begin_flight_audio();
        }
        let layout = self.topology.visible_layout(&root);
        self.scene.apply_city_layout(&layout);
        self.sync_navigator_selection();
    }

    fn advance_journey(&mut self) {
        if !self.scene.flight_complete() {
            return;
        }
        if let Some(waypoint) = self.journey.pop_front() {
            let layout = self.topology.visible_layout(&waypoint);
            self.scene.set_visual_state(VisualState::Transit);
            self.scene.begin_waypoint(waypoint.clone(), &layout);
            self.begin_flight_audio();
            return;
        }
        let Some(snapshot) = self.pending_snapshot.take() else {
            return;
        };
        self.scene
            .update(snapshot.root.clone(), snapshot.entries, &snapshot.mutations);
        let layout = self.topology.visible_layout(&snapshot.root);
        self.scene.apply_city_layout(&layout);
        self.sync_navigator_selection();
    }

    fn resize_terminal(&self) {
        if let Some(renderer) = &self.renderer {
            let (rows, cols, width, height) = renderer.terminal_dimensions();
            self.terminal.resize(rows, cols, width, height);
        }
    }

    fn visual_state(&self) -> VisualState {
        if self.scene.flight_complete()
            && self.journey.is_empty()
            && self.pending_snapshot.is_none()
        {
            VisualState::Settled
        } else {
            VisualState::Transit
        }
    }

    fn sync_navigator_selection(&mut self) {
        self.scene.select_entry(self.navigator.selected_entry_id());
    }

    fn begin_flight_audio(&mut self) {
        let duration = self.scene.flight_duration();
        if let Some(soundscape) = &mut self.soundscape {
            soundscape.begin_flight(duration);
        }
    }

    fn navigator_feedback(&mut self, changed: bool, direction: isize) {
        if changed && let Some(soundscape) = &mut self.soundscape {
            soundscape.navigate(direction);
        }
    }

    fn navigate_from_navigator(&mut self, path: PathBuf) {
        if !self.shell_at_prompt {
            return;
        }
        self.requested_navigator_path = Some(path.clone());
        self.scene.set_visual_state(VisualState::Transit);
        self.terminal.navigate(path);
    }

    fn activate_navigator(&mut self) {
        let Some(action) = self.navigator.activate() else {
            return;
        };
        match action {
            NavigatorAction::EnterDirectory(path) => self.navigate_from_navigator(path),
            NavigatorAction::OpenFile(path) => {
                if let Err(error) = open_with("xdg-open", &path) {
                    error!(path = %path.display(), %error, "default file open failed");
                }
            }
        }
    }

    fn set_input_target(&mut self, target: InputTarget) {
        self.input_target = target;
    }

    fn apply_settings(&mut self) {
        let settings = self.settings_menu.settings();
        if let Err(error) = settings.save() {
            warn!(%error, "cannot save GIBSON settings");
        }
        self.scene.set_motion_scale(settings.graphics.motion_scale);
        if let Some(renderer) = &mut self.renderer {
            renderer.apply_settings(settings, self.theme_watcher.current());
        }
        if let Some(soundscape) = &mut self.soundscape {
            soundscape.apply_settings(settings.audio.enabled, settings.audio.master_volume);
        }
        self.next_frame = Instant::now();
    }

    fn handle_app_shortcut(&mut self, key: &Key) -> bool {
        match key {
            Key::Named(NamedKey::F2) => {
                let next = match self.input_target {
                    InputTarget::Terminal => InputTarget::Navigator,
                    InputTarget::Navigator => InputTarget::Terminal,
                };
                if let Some(renderer) = &mut self.renderer {
                    match next {
                        InputTarget::Terminal if !renderer.terminal_visible() => {
                            renderer.toggle_terminal();
                        }
                        InputTarget::Navigator if !renderer.navigator_visible() => {
                            renderer.toggle_navigator();
                        }
                        _ => {}
                    }
                }
                self.set_input_target(next);
                self.resize_terminal();
            }
            Key::Named(NamedKey::F3) => {
                if let Some(renderer) = &mut self.renderer {
                    let visible = renderer.toggle_terminal();
                    if !visible && self.input_target == InputTarget::Terminal {
                        self.input_target = InputTarget::Navigator;
                    }
                }
                self.resize_terminal();
            }
            Key::Named(NamedKey::F4) => {
                if let Some(renderer) = &mut self.renderer {
                    let visible = renderer.toggle_navigator();
                    if !visible && self.input_target == InputTarget::Navigator {
                        self.input_target = InputTarget::Terminal;
                    }
                }
                self.resize_terminal();
            }
            Key::Named(NamedKey::F11) => {
                if let Some(window) = &self.window {
                    let fullscreen = window
                        .fullscreen()
                        .is_none()
                        .then_some(Fullscreen::Borderless(None));
                    window.set_fullscreen(fullscreen);
                }
            }
            Key::Named(NamedKey::F10) => {
                self.settings_menu.toggle();
            }
            _ => return false,
        }
        true
    }

    fn handle_key(&mut self, event: KeyEvent) {
        if event.state != ElementState::Pressed || self.handle_app_shortcut(&event.logical_key) {
            return;
        }

        if self.settings_menu.is_open() {
            let changed = match event.logical_key {
                Key::Named(NamedKey::ArrowUp) => {
                    self.settings_menu.move_selection(-1);
                    false
                }
                Key::Named(NamedKey::ArrowDown) => {
                    self.settings_menu.move_selection(1);
                    false
                }
                Key::Named(NamedKey::ArrowLeft) => self.settings_menu.adjust(-1),
                Key::Named(NamedKey::ArrowRight) | Key::Named(NamedKey::Enter) => {
                    self.settings_menu.adjust(1)
                }
                Key::Named(NamedKey::Escape) => {
                    self.settings_menu.close();
                    false
                }
                _ => false,
            };
            if changed {
                self.apply_settings();
            }
            return;
        }

        if self.input_target == InputTarget::Terminal {
            if let Some(bytes) = terminal_input(&event, self.modifiers) {
                if bytes == b"\r" {
                    self.shell_at_prompt = false;
                    self.scene.begin_command();
                }
                self.terminal.send_input(bytes);
            }
            return;
        }

        match event.logical_key {
            Key::Named(NamedKey::ArrowUp) => {
                let changed = self.navigator.move_selection(-1);
                self.navigator_feedback(changed, -1);
            }
            Key::Named(NamedKey::ArrowDown) => {
                let changed = self.navigator.move_selection(1);
                self.navigator_feedback(changed, 1);
            }
            Key::Named(NamedKey::ArrowLeft) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.rotate_orbit(-1);
                }
            }
            Key::Named(NamedKey::ArrowRight) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.rotate_orbit(1);
                }
            }
            Key::Named(NamedKey::PageUp) => {
                let changed = self.navigator.move_selection(-8);
                self.navigator_feedback(changed, -1);
            }
            Key::Named(NamedKey::PageDown) => {
                let changed = self.navigator.move_selection(8);
                self.navigator_feedback(changed, 1);
            }
            Key::Named(NamedKey::Home) => {
                let changed = self.navigator.select_first();
                self.navigator_feedback(changed, -1);
            }
            Key::Named(NamedKey::End) => {
                let changed = self.navigator.select_last();
                self.navigator_feedback(changed, 1);
            }
            Key::Named(NamedKey::Enter) => self.activate_navigator(),
            Key::Named(NamedKey::Backspace) => {
                if let Some(path) = self.navigator.parent_path() {
                    self.navigate_from_navigator(path);
                }
            }
            Key::Named(NamedKey::Escape) => self.set_input_target(InputTarget::Terminal),
            Key::Character(ref value) if value.eq_ignore_ascii_case("r") => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.reset_orbit();
                }
            }
            _ => {}
        }
        self.sync_navigator_selection();
    }

    fn cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        self.cursor_position = position;
        if self.settings_menu.is_open() {
            return;
        }
        let x = position.x as f32;
        let y = position.y as f32;
        if self
            .renderer
            .as_mut()
            .is_some_and(|renderer| renderer.drag_splitter(x, y))
        {
            self.resize_terminal();
        } else if let Some(row) = self
            .renderer
            .as_ref()
            .and_then(|renderer| renderer.navigator_row_at(x, y))
        {
            let changed = self.navigator.select_visible_row(row);
            self.navigator_feedback(changed, 0);
            self.sync_navigator_selection();
        }
    }

    fn press_left_mouse(&mut self) {
        if self.settings_menu.is_open() {
            return;
        }
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        let x = self.cursor_position.x as f32;
        let y = self.cursor_position.y as f32;
        if renderer.begin_splitter_drag(x, y) {
            return;
        }
        match renderer.pane_at(x, y) {
            PaneTarget::Terminal => self.set_input_target(InputTarget::Terminal),
            PaneTarget::Navigator | PaneTarget::Visualiser => {
                self.set_input_target(InputTarget::Navigator);
            }
            PaneTarget::Splitter | PaneTarget::Empty => {}
        }
    }

    fn scroll_navigator(&mut self, delta: MouseScrollDelta) {
        if self.settings_menu.is_open() {
            return;
        }
        let over_navigator = self.renderer.as_ref().is_some_and(|renderer| {
            renderer.pane_at(self.cursor_position.x as f32, self.cursor_position.y as f32)
                == PaneTarget::Navigator
        });
        if !over_navigator {
            return;
        }
        let amount = match delta {
            MouseScrollDelta::LineDelta(_, vertical) if vertical > 0.0 => -1,
            MouseScrollDelta::LineDelta(_, vertical) if vertical < 0.0 => 1,
            MouseScrollDelta::PixelDelta(position) if position.y > 0.0 => -1,
            MouseScrollDelta::PixelDelta(position) if position.y < 0.0 => 1,
            _ => 0,
        };
        let changed = self.navigator.move_selection(amount);
        self.navigator_feedback(changed, amount);
        self.sync_navigator_selection();
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        self.drain_events();
        if self.shell_exited {
            event_loop.exit();
            return;
        }
        if self
            .renderer
            .as_mut()
            .is_some_and(Renderer::sync_window_size)
        {
            self.resize_terminal();
        }
        let terminal = self.terminal.snapshot();
        let (rows, columns) = self
            .renderer
            .as_ref()
            .map_or((12, 40), Renderer::navigator_dimensions);
        let navigator =
            self.navigator
                .snapshot(rows, columns, self.input_target == InputTarget::Navigator);
        let settings_snapshot = self
            .settings_menu
            .is_open()
            .then(|| self.settings_menu.snapshot());
        let status = RenderStatus {
            terminal_focused: self.input_target == InputTarget::Terminal
                && !self.settings_menu.is_open(),
            visual_state: self.visual_state(),
            settings: settings_snapshot.as_ref(),
        };
        if let Some(renderer) = &mut self.renderer
            && let Err(error) = renderer.render(&self.scene, &terminal, &navigator, status)
        {
            error!("render failed: {error:#}");
        }
    }
}

struct PendingSnapshot {
    root: PathBuf,
    entries: Vec<FileEntry>,
    mutations: Vec<FileMutation>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        let mut attributes = Window::default_attributes()
            .with_title(format!("GIBSON // {}", self.initial_root.display()))
            .with_name("gibson-ui", "gibson-ui")
            .with_inner_size(LogicalSize::new(1440.0, 900.0))
            .with_min_inner_size(LogicalSize::new(900.0, 600.0));
        if self.start_fullscreen {
            attributes = attributes.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                error!("window creation failed: {error}");
                event_loop.exit();
                return;
            }
        };
        match pollster::block_on(Renderer::new(
            window.clone(),
            event_loop,
            self.settings_menu.settings().clone(),
            self.theme_watcher.current().clone(),
        )) {
            Ok(renderer) => {
                info!(gpu = renderer.adapter_name(), "renderer ready");
                self.window = Some(window);
                self.renderer = Some(renderer);
                self.resize_terminal();
            }
            Err(error) => {
                error!("renderer creation failed: {error:#}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                self.resize_terminal();
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::Focused(focused) => {
                if let Some(soundscape) = &mut self.soundscape {
                    soundscape.set_focused(focused);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => self.handle_key(event),
            WindowEvent::CursorMoved { position, .. } => self.cursor_moved(position),
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => self.press_left_mouse(),
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                if self
                    .renderer
                    .as_mut()
                    .is_some_and(Renderer::end_splitter_drag)
                {
                    self.resize_terminal();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => self.scroll_navigator(delta),
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.drain_events();
        if self.shell_exited {
            event_loop.exit();
            return;
        }
        let now = Instant::now();
        if let Some(interval) = self.settings_menu.settings().graphics.frame_rate.interval() {
            if now >= self.next_frame
                && let Some(window) = &self.window
            {
                window.request_redraw();
                self.next_frame = now + interval;
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
        } else {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            event_loop.set_control_flow(ControlFlow::Poll);
        }
    }
}

fn directory_scan(root: &std::path::Path, entries: &[FileEntry]) -> DirectoryScan {
    DirectoryScan {
        path: root.to_path_buf(),
        entry_count: entries.len(),
        children: entries
            .iter()
            .filter(|entry| entry.is_directory())
            .map(|entry| DirectoryChild {
                path: entry.path.clone(),
                name: entry.display_name.clone(),
                readable: entry.readable,
                traversable: true,
            })
            .collect(),
    }
}

fn enter_cockpit_workspace() {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        return;
    }
    let result = Command::new("hyprctl")
        .args(["dispatch", "workspace", "empty"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if result.is_err() || result.is_ok_and(|status| !status.success()) {
        error!("cannot select an empty Hyprland workspace for cockpit mode");
    }
}

fn open_with(program: &str, path: &std::path::Path) -> anyhow::Result<()> {
    let mut child = Command::new(program)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("cannot start the default opener for {}", path.display()))?;
    thread::Builder::new()
        .name("gibson-open".into())
        .spawn(move || {
            let _ = child.wait();
        })?;
    Ok(())
}

fn terminal_input(event: &KeyEvent, modifiers: ModifiersState) -> Option<Vec<u8>> {
    let named = match event.logical_key {
        Key::Named(NamedKey::Enter) => Some(b"\r".as_slice()),
        Key::Named(NamedKey::Backspace) => Some(b"\x7f".as_slice()),
        Key::Named(NamedKey::Tab) => Some(b"\t".as_slice()),
        Key::Named(NamedKey::Escape) => Some(b"\x1b".as_slice()),
        Key::Named(NamedKey::ArrowUp) => Some(b"\x1b[A".as_slice()),
        Key::Named(NamedKey::ArrowDown) => Some(b"\x1b[B".as_slice()),
        Key::Named(NamedKey::ArrowRight) => Some(b"\x1b[C".as_slice()),
        Key::Named(NamedKey::ArrowLeft) => Some(b"\x1b[D".as_slice()),
        Key::Named(NamedKey::Home) => Some(b"\x1b[H".as_slice()),
        Key::Named(NamedKey::End) => Some(b"\x1b[F".as_slice()),
        Key::Named(NamedKey::Delete) => Some(b"\x1b[3~".as_slice()),
        Key::Named(NamedKey::PageUp) => Some(b"\x1b[5~".as_slice()),
        Key::Named(NamedKey::PageDown) => Some(b"\x1b[6~".as_slice()),
        _ => None,
    };
    if let Some(bytes) = named {
        return Some(bytes.to_vec());
    }

    if modifiers.control_key()
        && let Key::Character(text) = &event.logical_key
        && let Some(character) = text.chars().next()
        && character.is_ascii()
    {
        let byte = character.to_ascii_lowercase() as u8 & 0x1f;
        return Some(vec![byte]);
    }

    let text = event.text.as_ref()?;
    if text.is_empty() {
        return None;
    }
    let mut bytes = Vec::with_capacity(text.len() + 1);
    if modifiers.alt_key() {
        bytes.push(0x1b);
    }
    bytes.extend_from_slice(text.as_bytes());
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn desktop_opener_receives_a_literal_file_path() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file with spaces.txt");
        fs::write(&path, b"data").unwrap();
        open_with("/bin/true", &path).unwrap();
    }
}
