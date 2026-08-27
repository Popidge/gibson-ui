use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GraphicsQuality {
    Performance,
    Balanced,
    #[default]
    High,
    Cinematic,
}

impl GraphicsQuality {
    pub fn label(self) -> &'static str {
        match self {
            Self::Performance => "PERFORMANCE",
            Self::Balanced => "BALANCED",
            Self::High => "HIGH",
            Self::Cinematic => "CINEMATIC",
        }
    }

    pub fn max_objects(self) -> usize {
        match self {
            Self::Performance => 320,
            Self::Balanced => 560,
            Self::High => 800,
            Self::Cinematic => 960,
        }
    }

    pub fn max_labels(self) -> usize {
        match self {
            Self::Performance => 10,
            Self::Balanced => 18,
            Self::High => 26,
            Self::Cinematic => 32,
        }
    }

    fn adjust(self, direction: isize) -> Self {
        const VALUES: [GraphicsQuality; 4] = [
            GraphicsQuality::Performance,
            GraphicsQuality::Balanced,
            GraphicsQuality::High,
            GraphicsQuality::Cinematic,
        ];
        cycle(self, direction, &VALUES)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FrameRate {
    Fps30,
    #[default]
    Fps60,
    Fps120,
    Unlimited,
}

impl FrameRate {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fps30 => "30 FPS",
            Self::Fps60 => "60 FPS",
            Self::Fps120 => "120 FPS",
            Self::Unlimited => "UNLIMITED",
        }
    }

    pub fn interval(self) -> Option<Duration> {
        match self {
            Self::Fps30 => Some(Duration::from_nanos(33_333_333)),
            Self::Fps60 => Some(Duration::from_nanos(16_666_667)),
            Self::Fps120 => Some(Duration::from_nanos(8_333_333)),
            Self::Unlimited => None,
        }
    }

    fn adjust(self, direction: isize) -> Self {
        const VALUES: [FrameRate; 4] = [
            FrameRate::Fps30,
            FrameRate::Fps60,
            FrameRate::Fps120,
            FrameRate::Unlimited,
        ];
        cycle(self, direction, &VALUES)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeBackdrop {
    Off,
    #[default]
    Tint,
    Wallpaper,
}

impl ThemeBackdrop {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::Tint => "COLOUR TINT",
            Self::Wallpaper => "OMARCHY WALLPAPER",
        }
    }

    fn adjust(self, direction: isize, omarchy_available: bool) -> Self {
        const VALUES: [ThemeBackdrop; 3] = [
            ThemeBackdrop::Off,
            ThemeBackdrop::Tint,
            ThemeBackdrop::Wallpaper,
        ];
        let values = if omarchy_available {
            &VALUES[..]
        } else {
            &VALUES[..2]
        };
        cycle(self, direction, values)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct GraphicsSettings {
    pub quality: GraphicsQuality,
    pub frame_rate: FrameRate,
    pub floor_pulses: bool,
    pub scanlines: bool,
    pub motion_scale: f32,
    pub performance_log: bool,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            quality: GraphicsQuality::High,
            frame_rate: FrameRate::Fps60,
            floor_pulses: true,
            scanlines: true,
            motion_scale: 1.0,
            performance_log: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct AppearanceSettings {
    pub follow_omarchy: bool,
    pub backdrop: ThemeBackdrop,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            follow_omarchy: true,
            backdrop: ThemeBackdrop::Tint,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct BehaviourSettings {
    pub orbit_enabled: bool,
    pub idle_sweep: bool,
}

impl Default for BehaviourSettings {
    fn default() -> Self {
        Self {
            orbit_enabled: true,
            idle_sweep: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct AudioSettings {
    pub enabled: bool,
    pub master_volume: f32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            master_volume: 0.65,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Settings {
    pub graphics: GraphicsSettings,
    pub appearance: AppearanceSettings,
    pub behaviour: BehaviourSettings,
    pub audio: AudioSettings,
}

impl Settings {
    pub fn load() -> Self {
        let Some(path) = settings_path() else {
            return Self::default();
        };
        let Ok(text) = fs::read_to_string(path) else {
            return Self::default();
        };
        let mut settings = toml::from_str::<Self>(&text).unwrap_or_default();
        settings.clamp();
        settings
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = settings_path() else {
            return Ok(());
        };
        let Some(parent) = path.parent() else {
            return Ok(());
        };
        fs::create_dir_all(parent)?;
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        fs::write(path, text)
    }

    pub fn adapt_to_environment(&mut self, omarchy_available: bool) {
        if omarchy_available {
            return;
        }
        self.appearance.follow_omarchy = false;
        if self.appearance.backdrop == ThemeBackdrop::Wallpaper {
            self.appearance.backdrop = ThemeBackdrop::Tint;
        }
    }

    fn clamp(&mut self) {
        self.graphics.motion_scale = finite_clamp(self.graphics.motion_scale, 1.0, 0.0, 1.5);
        self.audio.master_volume = finite_clamp(self.audio.master_volume, 0.65, 0.0, 1.0);
    }
}

#[derive(Clone, Debug)]
pub struct SettingsSnapshot {
    pub text: String,
    pub fingerprint: u64,
}

pub struct SettingsMenu {
    settings: Settings,
    omarchy_available: bool,
    open: bool,
    selected: usize,
}

#[derive(Clone, Copy)]
enum SettingItem {
    FollowOmarchy,
    Backdrop,
    GraphicsQuality,
    FrameRate,
    FloorPulses,
    Scanlines,
    Motion,
    PerformanceLog,
    OrbitControls,
    IdleCameraSweep,
    Audio,
    AudioVolume,
}

impl SettingsMenu {
    const OMARCHY_ITEMS: [SettingItem; 12] = [
        SettingItem::FollowOmarchy,
        SettingItem::Backdrop,
        SettingItem::GraphicsQuality,
        SettingItem::FrameRate,
        SettingItem::FloorPulses,
        SettingItem::Scanlines,
        SettingItem::Motion,
        SettingItem::PerformanceLog,
        SettingItem::OrbitControls,
        SettingItem::IdleCameraSweep,
        SettingItem::Audio,
        SettingItem::AudioVolume,
    ];
    const PORTABLE_ITEMS: [SettingItem; 11] = [
        SettingItem::Backdrop,
        SettingItem::GraphicsQuality,
        SettingItem::FrameRate,
        SettingItem::FloorPulses,
        SettingItem::Scanlines,
        SettingItem::Motion,
        SettingItem::PerformanceLog,
        SettingItem::OrbitControls,
        SettingItem::IdleCameraSweep,
        SettingItem::Audio,
        SettingItem::AudioVolume,
    ];

    pub fn new(settings: Settings, omarchy_available: bool) -> Self {
        Self {
            settings,
            omarchy_available,
            open: false,
            selected: 0,
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn toggle(&mut self) -> bool {
        self.open = !self.open;
        self.open
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn move_selection(&mut self, direction: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(direction)
            .min(self.items().len() - 1);
    }

    pub fn adjust(&mut self, direction: isize) -> bool {
        if direction == 0 {
            return false;
        }
        let before = self.settings.clone();
        match self.items()[self.selected] {
            SettingItem::FollowOmarchy => self.settings.appearance.follow_omarchy ^= true,
            SettingItem::Backdrop => {
                self.settings.appearance.backdrop = self
                    .settings
                    .appearance
                    .backdrop
                    .adjust(direction, self.omarchy_available)
            }
            SettingItem::GraphicsQuality => {
                self.settings.graphics.quality = self.settings.graphics.quality.adjust(direction)
            }
            SettingItem::FrameRate => {
                self.settings.graphics.frame_rate =
                    self.settings.graphics.frame_rate.adjust(direction)
            }
            SettingItem::FloorPulses => self.settings.graphics.floor_pulses ^= true,
            SettingItem::Scanlines => self.settings.graphics.scanlines ^= true,
            SettingItem::Motion => {
                self.settings.graphics.motion_scale = stepped(
                    self.settings.graphics.motion_scale,
                    direction,
                    0.1,
                    0.0,
                    1.5,
                )
            }
            SettingItem::PerformanceLog => self.settings.graphics.performance_log ^= true,
            SettingItem::OrbitControls => self.settings.behaviour.orbit_enabled ^= true,
            SettingItem::IdleCameraSweep => self.settings.behaviour.idle_sweep ^= true,
            SettingItem::Audio => self.settings.audio.enabled ^= true,
            SettingItem::AudioVolume => {
                self.settings.audio.master_volume =
                    stepped(self.settings.audio.master_volume, direction, 0.05, 0.0, 1.0);
            }
        }
        self.settings != before
    }

    pub fn snapshot(&self) -> SettingsSnapshot {
        let mut text = String::from("GIBSON // SETTINGS\n\n");
        for (index, item) in self.items().iter().copied().enumerate() {
            let marker = if index == self.selected { '▶' } else { ' ' };
            let (label, value) = self.row(item);
            text.push_str(&format!("{marker} {label:<22} {value}\n"));
        }
        text.push_str(
            "\nUP/DOWN SELECT // LEFT/RIGHT CHANGE\nF10 OR ESC CLOSE // SAVED AUTOMATICALLY",
        );
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        SettingsSnapshot {
            text,
            fingerprint: hasher.finish(),
        }
    }

    fn items(&self) -> &[SettingItem] {
        if self.omarchy_available {
            &Self::OMARCHY_ITEMS
        } else {
            &Self::PORTABLE_ITEMS
        }
    }

    fn row(&self, item: SettingItem) -> (&'static str, String) {
        match item {
            SettingItem::FollowOmarchy => (
                "FOLLOW OMARCHY",
                on_off(self.settings.appearance.follow_omarchy).to_owned(),
            ),
            SettingItem::Backdrop => (
                "BACKDROP",
                self.settings.appearance.backdrop.label().to_owned(),
            ),
            SettingItem::GraphicsQuality => (
                "GRAPHICS QUALITY",
                self.settings.graphics.quality.label().to_owned(),
            ),
            SettingItem::FrameRate => (
                "FRAME RATE",
                self.settings.graphics.frame_rate.label().to_owned(),
            ),
            SettingItem::FloorPulses => (
                "FLOOR PULSES",
                on_off(self.settings.graphics.floor_pulses).to_owned(),
            ),
            SettingItem::Scanlines => (
                "SCANLINES",
                on_off(self.settings.graphics.scanlines).to_owned(),
            ),
            SettingItem::Motion => (
                "MOTION",
                format!("{:.0}%", self.settings.graphics.motion_scale * 100.0),
            ),
            SettingItem::PerformanceLog => (
                "PERFORMANCE LOG",
                on_off(self.settings.graphics.performance_log).to_owned(),
            ),
            SettingItem::OrbitControls => (
                "ORBIT CONTROLS",
                on_off(self.settings.behaviour.orbit_enabled).to_owned(),
            ),
            SettingItem::IdleCameraSweep => (
                "IDLE CAMERA SWEEP",
                on_off(self.settings.behaviour.idle_sweep).to_owned(),
            ),
            SettingItem::Audio => ("AUDIO", on_off(self.settings.audio.enabled).to_owned()),
            SettingItem::AudioVolume => (
                "AUDIO VOLUME",
                format!("{:.0}%", self.settings.audio.master_volume * 100.0),
            ),
        }
    }
}

fn cycle<T: Copy + PartialEq>(current: T, direction: isize, values: &[T]) -> T {
    let index = values
        .iter()
        .position(|value| *value == current)
        .unwrap_or(0);
    values[index.saturating_add_signed(direction).min(values.len() - 1)]
}

fn stepped(value: f32, direction: isize, step: f32, min: f32, max: f32) -> f32 {
    (((value + direction.signum() as f32 * step) / step).round() * step).clamp(min, max)
}

fn finite_clamp(value: f32, fallback: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

fn on_off(value: bool) -> &'static str {
    if value { "ON" } else { "OFF" }
}

fn settings_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(path).join("gibson/gibson.toml"));
    }
    std::env::var_os("HOME").map(|path| PathBuf::from(path).join(".config/gibson/gibson.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_numeric_values_return_to_safe_settings() {
        let mut settings = toml::from_str::<Settings>(
            "[graphics]\nmotion_scale = nan\n[audio]\nmaster_volume = 99.0\n",
        )
        .unwrap();
        settings.clamp();
        assert_eq!(settings.graphics.motion_scale, 1.0);
        assert_eq!(settings.audio.master_volume, 1.0);
    }

    #[test]
    fn menu_changes_persisted_values_and_marks_the_selected_row() {
        let mut menu = SettingsMenu::new(Settings::default(), true);
        menu.move_selection(3);
        assert!(menu.adjust(1));
        assert_eq!(menu.settings.graphics.frame_rate, FrameRate::Fps120);
        assert!(menu.snapshot().text.contains("▶ FRAME RATE"));
    }

    #[test]
    fn portable_settings_hide_omarchy_and_skip_its_wallpaper() {
        let mut settings = Settings::default();
        settings.adapt_to_environment(false);
        let mut menu = SettingsMenu::new(settings, false);

        assert!(!menu.snapshot().text.contains("OMARCHY"));
        assert!(menu.adjust(-1));
        assert_eq!(menu.settings.appearance.backdrop, ThemeBackdrop::Off);
        assert!(menu.adjust(1));
        assert_eq!(menu.settings.appearance.backdrop, ThemeBackdrop::Tint);
        assert!(!menu.adjust(1));
    }
}
