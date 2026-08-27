use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(750);

#[derive(Clone, Debug, PartialEq)]
pub struct OmarchyTheme {
    pub name: String,
    pub background: [u8; 3],
    pub dark_background: [u8; 3],
    pub foreground: [u8; 3],
    pub accent: [u8; 3],
    pub cyan: [u8; 3],
    pub blue: [u8; 3],
    pub magenta: [u8; 3],
    pub red: [u8; 3],
    pub wallpaper: Option<PathBuf>,
    fingerprint: u64,
}

impl OmarchyTheme {
    pub fn available() -> bool {
        omarchy_state_path().is_some_and(|path| path.join("theme/colors.toml").is_file())
    }

    pub fn load() -> Self {
        let state = omarchy_state_path();
        let colors_path = state.as_ref().map(|path| path.join("theme/colors.toml"));
        let colors_text = colors_path
            .as_ref()
            .and_then(|path| fs::read_to_string(path).ok())
            .unwrap_or_default();
        let values = colors_text
            .parse::<toml::Table>()
            .unwrap_or_else(|_| toml::Table::new());
        let name = state
            .as_ref()
            .and_then(|path| fs::read_to_string(path.join("theme.name")).ok())
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "GIBSON".into());
        let wallpaper = state
            .as_ref()
            .map(|path| path.join("background"))
            .filter(|path| path.is_file());
        let mut hasher = DefaultHasher::new();
        name.hash(&mut hasher);
        colors_text.hash(&mut hasher);
        wallpaper
            .as_ref()
            .and_then(|path| fs::canonicalize(path).ok())
            .hash(&mut hasher);
        wallpaper
            .as_ref()
            .and_then(|path| fs::metadata(path).ok())
            .and_then(|metadata| metadata.modified().ok())
            .hash(&mut hasher);
        Self {
            name,
            background: color(&values, "background", [0x28, 0x25, 0x21]),
            dark_background: color(&values, "darker_background", [0x00, 0x03, 0x0b]),
            foreground: color(&values, "foreground", [0xd6, 0xc9, 0xad]),
            accent: color(&values, "accent", [0xd7, 0xa9, 0x3d]),
            cyan: color(&values, "cyan", [0x00, 0xeb, 0xf5]),
            blue: color(&values, "blue", [0x1f, 0xa7, 0xe0]),
            magenta: color(&values, "magenta", [0xff, 0x26, 0xd2]),
            red: color(&values, "red", [0xe8, 0x30, 0x62]),
            wallpaper,
            fingerprint: hasher.finish(),
        }
    }

    pub fn classic() -> Self {
        Self {
            name: "GIBSON 1995".into(),
            background: [0x00, 0x08, 0x18],
            dark_background: [0x00, 0x01, 0x08],
            foreground: [0x9d, 0xff, 0xf9],
            accent: [0x2a, 0xe9, 0xff],
            cyan: [0x00, 0xeb, 0xf5],
            blue: [0x1f, 0xa7, 0xe0],
            magenta: [0xff, 0x26, 0xd2],
            red: [0xe8, 0x30, 0x62],
            wallpaper: None,
            fingerprint: 0,
        }
    }

    pub fn rgba(color: [u8; 3], alpha: f32) -> [f32; 4] {
        [
            srgb_to_linear(color[0]),
            srgb_to_linear(color[1]),
            srgb_to_linear(color[2]),
            alpha,
        ]
    }
}

fn srgb_to_linear(value: u8) -> f32 {
    let value = f32::from(value) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

pub struct ThemeWatcher {
    current: OmarchyTheme,
    available: bool,
    next_poll: Instant,
}

impl ThemeWatcher {
    pub fn new() -> Self {
        let available = OmarchyTheme::available();
        Self {
            current: if available {
                OmarchyTheme::load()
            } else {
                OmarchyTheme::classic()
            },
            available,
            next_poll: Instant::now() + POLL_INTERVAL,
        }
    }

    pub fn is_available(&self) -> bool {
        self.available
    }

    pub fn current(&self) -> &OmarchyTheme {
        &self.current
    }

    pub fn poll(&mut self, now: Instant) -> Option<&OmarchyTheme> {
        if !self.available || now < self.next_poll {
            return None;
        }
        self.next_poll = now + POLL_INTERVAL;
        let next = OmarchyTheme::load();
        if next.fingerprint == self.current.fingerprint {
            return None;
        }
        self.current = next;
        Some(&self.current)
    }
}

fn omarchy_state_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_STATE_HOME") {
        return Some(PathBuf::from(path).join("omarchy/current"));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state/omarchy/current"))
}

fn color(values: &toml::Table, key: &str, fallback: [u8; 3]) -> [u8; 3] {
    values
        .get(key)
        .and_then(toml::Value::as_str)
        .and_then(parse_hex_color)
        .unwrap_or(fallback)
}

fn parse_hex_color(value: &str) -> Option<[u8; 3]> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 6 {
        return None;
    }
    Some([
        u8::from_str_radix(&value[0..2], 16).ok()?,
        u8::from_str_radix(&value[2..4], 16).ok()?,
        u8::from_str_radix(&value[4..6], 16).ok()?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_six_digit_theme_colors() {
        assert_eq!(parse_hex_color("#d7a93d"), Some([0xd7, 0xa9, 0x3d]));
        assert_eq!(parse_hex_color("invalid"), None);
    }

    #[test]
    fn classic_palette_is_independent_of_omarchy_state() {
        let palette = OmarchyTheme::classic();
        assert_eq!(palette.name, "GIBSON 1995");
        assert!(palette.wallpaper.is_none());
    }
}
