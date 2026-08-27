use anyhow::Context;
use kira::sound::FromFileError;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::track::MainTrackBuilder;
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Easing, Tween};
use std::io::Cursor;
use std::time::{Duration, Instant};
use tracing::warn;

#[cfg(feature = "mixkit-audio")]
macro_rules! sound_asset {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/sounds/encoded/",
            $name
        ))
    };
}
#[cfg(not(feature = "mixkit-audio"))]
macro_rules! sound_asset {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/sounds/fallback/",
            $name
        ))
    };
}

const MUSIC: &[u8] = sound_asset!("suspense.ogg");
const UI_SWEEP: &[u8] = sound_asset!("ui-sweep.ogg");
const FLIGHT_SHORT: &[u8] = sound_asset!("flight-short.ogg");
const FLIGHT_MEDIUM_A: &[u8] = sound_asset!("flight-medium-a.ogg");
const FLIGHT_MEDIUM_B: &[u8] = sound_asset!("flight-medium-b.ogg");
const FLIGHT_LONG: &[u8] = sound_asset!("flight-long.ogg");

const MUSIC_IDLE_DB: f32 = -27.0;
const MUSIC_FLIGHT_DB: f32 = -20.0;
const FOCUS_FADE_OUT: Duration = Duration::from_millis(180);
const FOCUS_FADE_IN: Duration = Duration::from_millis(320);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FlightCue {
    Short,
    MediumA,
    MediumB,
    Long,
}

struct FlightSounds {
    short: StaticSoundData,
    medium_a: StaticSoundData,
    medium_b: StaticSoundData,
    long: StaticSoundData,
}

pub struct Soundscape {
    manager: AudioManager<DefaultBackend>,
    music: StreamingSoundHandle<FromFileError>,
    ui_sweep: StaticSoundData,
    flight: FlightSounds,
    ui_handle: Option<StaticSoundHandle>,
    flight_handle: Option<StaticSoundHandle>,
    flying: bool,
    focused: bool,
    enabled: bool,
    master_volume: f32,
    variation: u32,
    last_health_check: Instant,
}

impl Soundscape {
    pub fn new() -> anyhow::Result<Self> {
        let ui_sweep = decode_static(UI_SWEEP).context("decode UI sweep")?;
        let flight = FlightSounds {
            short: decode_static(FLIGHT_SHORT).context("decode short flight cue")?,
            medium_a: decode_static(FLIGHT_MEDIUM_A).context("decode flight cue A")?,
            medium_b: decode_static(FLIGHT_MEDIUM_B).context("decode flight cue B")?,
            long: decode_static(FLIGHT_LONG).context("decode long flight cue")?,
        };
        let music = StreamingSoundData::from_cursor(Cursor::new(MUSIC))
            .context("open suspense music")?
            .loop_region(0.0..)
            .volume(MUSIC_IDLE_DB)
            .fade_in_tween(tween(Duration::from_secs(3)));
        let settings = AudioManagerSettings {
            main_track_builder: MainTrackBuilder::new().sound_capacity(16),
            ..AudioManagerSettings::default()
        };
        let mut manager = AudioManager::<DefaultBackend>::new(settings)
            .context("open the default audio output")?;
        let music = manager.play(music).context("start suspense music")?;

        Ok(Self {
            manager,
            music,
            ui_sweep,
            flight,
            ui_handle: None,
            flight_handle: None,
            flying: false,
            focused: true,
            enabled: true,
            master_volume: 1.0,
            variation: 0,
            last_health_check: Instant::now(),
        })
    }

    pub fn navigate(&mut self, direction: isize) {
        if !self.enabled {
            return;
        }
        if let Some(mut handle) = self.ui_handle.take() {
            handle.stop(tween(Duration::from_millis(12)));
        }
        let variation = [0.985, 1.0, 1.015][self.variation as usize % 3];
        self.variation = self.variation.wrapping_add(1);
        let directional_rate = match direction.cmp(&0) {
            std::cmp::Ordering::Less => 1.055,
            std::cmp::Ordering::Equal => 1.0,
            std::cmp::Ordering::Greater => 0.945,
        };
        let panning = direction.signum() as f32 * 0.07;
        let sound = self
            .ui_sweep
            .volume(-5.0)
            .playback_rate(directional_rate * variation)
            .panning(panning);
        match self.manager.play(sound) {
            Ok(handle) => self.ui_handle = Some(handle),
            Err(error) => warn!(%error, "cannot play file navigator sound"),
        }
    }

    pub fn begin_flight(&mut self, duration: Duration) {
        if !self.enabled {
            return;
        }
        self.set_flying(true);
        if let Some(mut handle) = self.flight_handle.take() {
            handle.stop(tween(Duration::from_millis(80)));
        }

        let serial = self.variation;
        self.variation = self.variation.wrapping_add(1);
        let cue_kind = select_flight_cue(duration, serial);
        let (cue, looping, volume) = match cue_kind {
            FlightCue::Short => (&self.flight.short, false, -8.0),
            FlightCue::MediumA => (&self.flight.medium_a, false, -10.0),
            FlightCue::MediumB => (&self.flight.medium_b, false, -10.0),
            FlightCue::Long => (&self.flight.long, true, -16.0),
        };
        let pitch = [0.97, 1.0, 1.03][serial as usize % 3];
        let playback_rate = if looping {
            pitch
        } else {
            (cue.duration().as_secs_f64() / duration.as_secs_f64().max(0.1)).clamp(0.78, 1.30)
                * pitch
        };
        let panning = [-0.12, 0.08, -0.04, 0.14][serial as usize % 4];
        let mut sound = cue
            .volume(volume)
            .playback_rate(playback_rate)
            .panning(panning)
            .fade_in_tween(tween(Duration::from_millis(35)));
        if looping {
            sound = sound.loop_region(0.0..);
        }
        match self.manager.play(sound) {
            Ok(handle) => self.flight_handle = Some(handle),
            Err(error) => warn!(%error, "cannot play flight sound"),
        }
    }

    pub fn update(&mut self, flying: bool) {
        self.set_flying(flying);
        if self.last_health_check.elapsed() >= Duration::from_secs(2) {
            if let Some(error) = self.music.pop_error() {
                warn!(%error, "suspense music decoder failed");
            }
            self.last_health_check = Instant::now();
        }
    }

    pub fn set_focused(&mut self, focused: bool) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        let duration = if focused {
            FOCUS_FADE_IN
        } else {
            FOCUS_FADE_OUT
        };
        self.apply_main_volume(duration);
    }

    pub fn apply_settings(&mut self, enabled: bool, master_volume: f32) {
        self.enabled = enabled;
        self.master_volume = if master_volume.is_finite() {
            master_volume.clamp(0.0, 1.0)
        } else {
            0.65
        };
        if !enabled {
            if let Some(mut handle) = self.ui_handle.take() {
                handle.stop(tween(Duration::from_millis(80)));
            }
            if let Some(mut handle) = self.flight_handle.take() {
                handle.stop(tween(Duration::from_millis(120)));
            }
        }
        self.apply_main_volume(Duration::from_millis(180));
    }

    fn set_flying(&mut self, flying: bool) {
        if self.flying == flying {
            return;
        }
        self.flying = flying;
        let (volume, duration) = if flying {
            (MUSIC_FLIGHT_DB, Duration::from_millis(450))
        } else {
            if let Some(mut handle) = self.flight_handle.take() {
                handle.stop(tween(Duration::from_millis(180)));
            }
            (MUSIC_IDLE_DB, Duration::from_millis(1_400))
        };
        self.music.set_volume(volume, tween(duration));
    }

    fn apply_main_volume(&mut self, duration: Duration) {
        let volume = if !self.enabled || !self.focused || self.master_volume <= 0.0001 {
            Decibels::SILENCE
        } else {
            Decibels(20.0 * self.master_volume.log10())
        };
        self.manager
            .main_track()
            .set_volume(volume, tween(duration));
    }
}

fn decode_static(bytes: &'static [u8]) -> Result<StaticSoundData, FromFileError> {
    StaticSoundData::from_cursor(Cursor::new(bytes))
}

fn select_flight_cue(duration: Duration, serial: u32) -> FlightCue {
    if duration < Duration::from_millis(1_350) {
        FlightCue::Short
    } else if duration < Duration::from_millis(3_100) {
        if serial.is_multiple_of(2) {
            FlightCue::MediumA
        } else {
            FlightCue::MediumB
        }
    } else {
        FlightCue::Long
    }
}

fn tween(duration: Duration) -> Tween {
    Tween {
        duration,
        easing: Easing::InOutPowi(2),
        ..Tween::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_sound_assets_decode() {
        let ui = decode_static(UI_SWEEP).unwrap();
        assert!(ui.duration() < Duration::from_millis(850));
        for bytes in [FLIGHT_SHORT, FLIGHT_MEDIUM_A, FLIGHT_MEDIUM_B, FLIGHT_LONG] {
            assert!(!decode_static(bytes).unwrap().frames.is_empty());
        }
        let music = StreamingSoundData::from_cursor(Cursor::new(MUSIC)).unwrap();
        #[cfg(feature = "mixkit-audio")]
        assert!(music.duration() > Duration::from_secs(210));
        #[cfg(not(feature = "mixkit-audio"))]
        assert!(music.duration() > Duration::from_secs(20));
    }

    #[test]
    fn flight_cues_follow_animation_length() {
        assert_eq!(
            select_flight_cue(Duration::from_millis(900), 0),
            FlightCue::Short
        );
        assert_eq!(
            select_flight_cue(Duration::from_secs(2), 0),
            FlightCue::MediumA
        );
        assert_eq!(
            select_flight_cue(Duration::from_secs(2), 1),
            FlightCue::MediumB
        );
        assert_eq!(
            select_flight_cue(Duration::from_secs(4), 0),
            FlightCue::Long
        );
    }
}
