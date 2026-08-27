use std::fs;
use std::path::PathBuf;

const WIDE_MIN_WIDTH: u32 = 1_280;
const WIDE_MIN_HEIGHT: u32 = 720;
const SPLITTER_SIZE: f32 = 8.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn right(self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn normalized(self, width: u32, height: u32) -> [f32; 4] {
        let width = width.max(1) as f32;
        let height = height.max(1) as f32;
        [
            self.x / width,
            self.y / height,
            self.width / width,
            self.height / height,
        ]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneTarget {
    Visualiser,
    Terminal,
    Navigator,
    Splitter,
    Empty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Splitter {
    Dock,
    Drawer,
}

#[derive(Clone, Copy, Debug)]
pub struct LayoutPreferences {
    pub dock_ratio: f32,
    pub drawer_ratio: f32,
    pub terminal_visible: bool,
    pub navigator_visible: bool,
}

impl Default for LayoutPreferences {
    fn default() -> Self {
        Self {
            dock_ratio: 1.0 / 3.0,
            drawer_ratio: 0.30,
            terminal_visible: true,
            navigator_visible: true,
        }
    }
}

impl LayoutPreferences {
    pub fn load() -> Self {
        let Some(path) = preference_path() else {
            return Self::default();
        };
        fs::read_to_string(path)
            .ok()
            .map_or_else(Self::default, |text| Self::parse(&text))
    }

    pub fn save(self) -> std::io::Result<()> {
        let Some(path) = preference_path() else {
            return Ok(());
        };
        let Some(parent) = path.parent() else {
            return Ok(());
        };
        fs::create_dir_all(parent)?;
        let text = format!(
            "dock_ratio = {:.4}\ndrawer_ratio = {:.4}\nterminal_visible = {}\nnavigator_visible = {}\n",
            self.dock_ratio, self.drawer_ratio, self.terminal_visible, self.navigator_visible
        );
        fs::write(path, text)
    }

    fn parse(text: &str) -> Self {
        let mut preferences = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "dock_ratio" => {
                    if let Ok(value) = value.parse::<f32>()
                        && value.is_finite()
                    {
                        preferences.dock_ratio = value.clamp(0.22, 0.50);
                    }
                }
                "drawer_ratio" => {
                    if let Ok(value) = value.parse::<f32>()
                        && value.is_finite()
                    {
                        preferences.drawer_ratio = value.clamp(0.18, 0.48);
                    }
                }
                "terminal_visible" => {
                    if let Ok(value) = value.parse::<bool>() {
                        preferences.terminal_visible = value;
                    }
                }
                "navigator_visible" => {
                    if let Ok(value) = value.parse::<bool>() {
                        preferences.navigator_visible = value;
                    }
                }
                _ => {}
            }
        }
        preferences
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CockpitLayout {
    pub visualiser: Rect,
    pub terminal: Option<Rect>,
    pub dock_splitter: Option<Rect>,
    pub drawer_splitter: Option<Rect>,
}

impl CockpitLayout {
    pub fn calculate(width: u32, height: u32, preferences: LayoutPreferences) -> Self {
        if is_wide(width, height) {
            Self::wide(width, height, preferences)
        } else {
            Self::compact(width, height, preferences)
        }
    }

    pub fn hit_test(self, x: f32, y: f32) -> PaneTarget {
        if self
            .dock_splitter
            .is_some_and(|splitter| splitter.contains(x, y))
            || self
                .drawer_splitter
                .is_some_and(|splitter| splitter.contains(x, y))
        {
            return PaneTarget::Splitter;
        }
        if self.terminal.is_some_and(|pane| pane.contains(x, y)) {
            return PaneTarget::Terminal;
        }
        if self.visualiser.contains(x, y) {
            return PaneTarget::Visualiser;
        }
        PaneTarget::Empty
    }

    pub fn splitter_at(self, x: f32, y: f32) -> Option<Splitter> {
        if self
            .dock_splitter
            .is_some_and(|splitter| splitter.contains(x, y))
        {
            return Some(Splitter::Dock);
        }
        if self
            .drawer_splitter
            .is_some_and(|splitter| splitter.contains(x, y))
        {
            return Some(Splitter::Drawer);
        }
        None
    }

    fn wide(width: u32, height: u32, preferences: LayoutPreferences) -> Self {
        let width = width as f32;
        let height = height as f32;
        let dock_width = (width * preferences.dock_ratio)
            .clamp(320.0, width * 0.50)
            .round();
        let dock_x = width - dock_width;
        let canvas_width = dock_x;
        let visualiser = Rect {
            x: 0.0,
            y: 0.0,
            width: canvas_width,
            height,
        };
        let dock_splitter = Some(Rect {
            x: dock_x - SPLITTER_SIZE * 0.5,
            y: 0.0,
            width: SPLITTER_SIZE,
            height,
        });
        let dock = Rect {
            x: dock_x,
            y: 0.0,
            width: dock_width,
            height,
        };
        let terminal = preferences.terminal_visible.then_some(dock);
        Self {
            visualiser,
            terminal,
            dock_splitter,
            drawer_splitter: None,
        }
    }

    fn compact(width: u32, height: u32, preferences: LayoutPreferences) -> Self {
        let width = width as f32;
        let height = height as f32;
        let drawer_visible = preferences.terminal_visible;
        let drawer_height = if drawer_visible {
            (height * preferences.drawer_ratio)
                .clamp(160.0, height * 0.48)
                .round()
        } else {
            0.0
        };
        let visualiser_height = height - drawer_height;
        let drawer = Rect {
            x: 0.0,
            y: visualiser_height,
            width,
            height: drawer_height,
        };
        let terminal = preferences.terminal_visible.then_some(drawer);
        let drawer_splitter = drawer_visible.then_some(Rect {
            x: 0.0,
            y: visualiser_height - SPLITTER_SIZE * 0.5,
            width,
            height: SPLITTER_SIZE,
        });
        Self {
            visualiser: Rect {
                x: 0.0,
                y: 0.0,
                width,
                height: visualiser_height,
            },
            terminal,
            dock_splitter: None,
            drawer_splitter,
        }
    }
}

fn is_wide(width: u32, height: u32) -> bool {
    width >= WIDE_MIN_WIDTH
        && height >= WIDE_MIN_HEIGHT
        && width as f32 / height.max(1) as f32 >= 1.45
}

fn preference_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(path).join("gibson/layout.toml"));
    }
    std::env::var_os("HOME").map(|path| PathBuf::from(path).join(".config/gibson/layout.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flagship_layout_uses_the_full_height_for_its_visualiser() {
        let layout = CockpitLayout::calculate(1_920, 1_080, LayoutPreferences::default());

        assert_eq!(layout.visualiser.width, 1_280.0);
        assert_eq!(layout.visualiser.height, 1_080.0);
        assert_eq!(layout.visualiser.y, 0.0);
        assert_eq!(layout.terminal.unwrap().width, 640.0);
    }

    #[test]
    fn tiled_layout_uses_a_bottom_drawer() {
        let layout = CockpitLayout::calculate(900, 800, LayoutPreferences::default());

        assert_eq!(layout.visualiser.height, 560.0);
        assert_eq!(layout.terminal.unwrap().height, 240.0);
    }

    #[test]
    fn hidden_drawer_gives_the_visualiser_the_complete_window() {
        let preferences = LayoutPreferences {
            terminal_visible: false,
            navigator_visible: false,
            ..LayoutPreferences::default()
        };
        let layout = CockpitLayout::calculate(900, 800, preferences);

        assert_eq!(layout.visualiser.height, 800.0);
        assert!(layout.terminal.is_none());
    }

    #[test]
    fn preferences_parse_and_clamp_saved_values() {
        let preferences = LayoutPreferences::parse(
            "dock_ratio = 0.4\ndrawer_ratio = 9\nterminal_visible = false\nnavigator_visible = true\n",
        );

        assert_eq!(preferences.dock_ratio, 0.4);
        assert_eq!(preferences.drawer_ratio, 0.48);
        assert!(!preferences.terminal_visible);
        assert!(preferences.navigator_visible);
    }

    #[test]
    fn preferences_ignore_non_finite_ratios() {
        let preferences = LayoutPreferences::parse("dock_ratio = NaN\ndrawer_ratio = inf\n");

        assert_eq!(
            preferences.dock_ratio,
            LayoutPreferences::default().dock_ratio
        );
        assert_eq!(
            preferences.drawer_ratio,
            LayoutPreferences::default().drawer_ratio
        );
    }
}
