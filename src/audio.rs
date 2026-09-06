use anyhow::Context;
use kira::sound::FromFileError;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::track::MainTrackBuilder;
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Easing, Tween};
use std::io::Cursor;
use std::time::{Duration, Instant};
use tracing::warn;

#[cfg(has_mixkit_audio)]
macro_rules! sound_asset {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/sounds/encoded/",
            $name
        ))
    };
}
#[cfg(not(has_mixkit_audio))]
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
    arrival: StaticSoundData,
    flight: FlightSounds,
    ui_handle: Option<StaticSoundHandle>,
    flight_handle: Option<StaticSoundHandle>,
    flying: bool,
    flight_volume: f32,
    arrival_handle: Option<StaticSoundHandle>,
    focused: bool,
    enabled: bool,
    music_paused: bool,
    master_volume: f32,
    variation: u32,
    last_health_check: Instant,
}

impl Soundscape {
    pub fn new() -> anyhow::Result<Self> {
        let ui_sweep = decode_static(UI_SWEEP).context("decode UI sweep")?;
        let arrival = prepare_arrival(&ui_sweep);
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
            arrival,
            flight,
            ui_handle: None,
            flight_handle: None,
            flying: false,
            flight_volume: -10.0,
            arrival_handle: None,
            focused: true,
            enabled: true,
            music_paused: false,
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
        if duration.is_zero() {
            return;
        }
        if let Some(mut handle) = self.arrival_handle.take() {
            handle.stop(tween(Duration::from_millis(80)));
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
        self.flight_volume = volume;
        let panning = 0.0;
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

    pub fn update(&mut self, flying: bool, roll: f32, intensity: f32) {
        if self.flying && !flying && self.enabled && self.focused {
            // Reuse the established sonic palette for a quiet landing accent.
            let cue = self.arrival.clone();
            match self.manager.play(cue) {
                Ok(handle) => self.arrival_handle = Some(handle),
                Err(error) => warn!(%error, "cannot play arrival sound"),
            }
        }
        self.set_flying(flying);
        if flying && let Some(handle) = &mut self.flight_handle {
            handle.set_panning(
                (roll * 2.0).clamp(-0.22, 0.22),
                tween(Duration::from_millis(70)),
            );
            handle.set_volume(
                self.flight_volume - 3.0 * (1.0 - intensity.clamp(0.0, 1.0)),
                tween(Duration::from_millis(70)),
            );
        }
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
            if let Some(mut handle) = self.arrival_handle.take() {
                handle.stop(tween(Duration::from_millis(80)));
            }
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
        let paused = volume == Decibels::SILENCE;
        if paused != self.music_paused {
            // Muting the output alone leaves the streaming decoder and mixer running.
            if paused {
                self.music.pause(tween(duration));
            } else {
                self.music.resume(tween(duration));
            }
            self.music_paused = paused;
        }
        self.manager
            .main_track()
            .set_volume(volume, tween(duration));
    }
}

// Bake both fades into the short clip: an immediately scheduled stop would
// replace Kira's playback fade-in while its gain is still silence.
fn prepare_arrival(source: &StaticSoundData) -> StaticSoundData {
    let mut cue = source.slice(0.0..0.36);
    let frames: Vec<_> = (0..cue.num_frames())
        .map(|index| {
            let mut frame = cue.frame_at_index(index).unwrap();
            let seconds = index as f32 / cue.sample_rate as f32 / 0.82;
            let fade_in = (seconds / 0.02).clamp(0.0, 1.0);
            let fade_out = ((0.4 - seconds) / 0.15).clamp(0.0, 1.0);
            let gain = fade_in * fade_out;
            frame.left *= gain;
            frame.right *= gain;
            frame
        })
        .collect();
    cue.frames = frames.into();
    cue.slice = None;
    cue.volume(-14.0).playback_rate(0.82).panning(0.0)
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
    fn arrival_clip_has_audible_body_and_silent_boundaries() {
        for asset in [
            UI_SWEEP,
            include_bytes!("../assets/sounds/fallback/ui-sweep.ogg").as_slice(),
        ] {
            let cue = prepare_arrival(&decode_static(asset).unwrap());
            assert!(cue.settings.fade_in_tween.is_none());
            let energy = |frame: &kira::Frame| frame.left.abs() + frame.right.abs();
            assert_eq!(energy(cue.frames.first().unwrap()), 0.0);
            assert_eq!(energy(cue.frames.last().unwrap()), 0.0);
            assert!(cue.frames.iter().map(energy).fold(0.0_f32, f32::max) > 0.01);
        }
    }

    #[test]
    #[ignore = "requires a live audio output device"]
    fn muted_music_stops_advancing_and_resumes() {
        use kira::sound::PlaybackState;

        let mut soundscape = Soundscape::new().unwrap();
        for reason in 0..3 {
            match reason {
                0 => soundscape.apply_settings(false, 0.01),
                1 => soundscape.set_focused(false),
                _ => soundscape.apply_settings(true, 0.0),
            }
            std::thread::sleep(Duration::from_millis(500));
            assert_eq!(soundscape.music.state(), PlaybackState::Paused);
            let paused_position = soundscape.music.position();
            std::thread::sleep(Duration::from_millis(100));
            assert_eq!(soundscape.music.position(), paused_position);

            soundscape.apply_settings(true, 0.01);
            soundscape.set_focused(true);
            std::thread::sleep(Duration::from_millis(500));
            assert_eq!(soundscape.music.state(), PlaybackState::Playing);
            assert!(soundscape.music.position() > paused_position);
        }
    }

    #[test]
    fn embedded_sound_assets_decode() {
        let ui = decode_static(UI_SWEEP).unwrap();
        assert!(ui.duration() < Duration::from_millis(850));
        for bytes in [FLIGHT_SHORT, FLIGHT_MEDIUM_A, FLIGHT_MEDIUM_B, FLIGHT_LONG] {
            assert!(!decode_static(bytes).unwrap().frames.is_empty());
        }
        let music = StreamingSoundData::from_cursor(Cursor::new(MUSIC)).unwrap();
        #[cfg(has_mixkit_audio)]
        assert!(music.duration() > Duration::from_secs(210));
        #[cfg(not(has_mixkit_audio))]
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
