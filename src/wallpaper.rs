use crossbeam_channel::{Receiver, unbounded};
use serde::Deserialize;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

const CLIENT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MONITOR_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Maps window-local UVs into the compositor's monitor-local wallpaper UVs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WallpaperPlacement {
    pub monitor_uv: [f32; 4],
    pub monitor_aspect: f32,
}

impl Default for WallpaperPlacement {
    fn default() -> Self {
        Self {
            monitor_uv: [0.0, 0.0, 1.0, 1.0],
            monitor_aspect: 0.0,
        }
    }
}

pub struct WallpaperPlacementWatcher {
    updates: Receiver<WallpaperPlacement>,
    enabled: Arc<AtomicBool>,
}

impl WallpaperPlacementWatcher {
    pub fn spawn(enabled: bool) -> Self {
        let (sender, updates) = unbounded();
        let enabled = Arc::new(AtomicBool::new(enabled));
        let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");
        if let Some(signature) = signature {
            let socket = hyprland_socket_path(signature.to_string_lossy().as_ref());
            let pid = std::process::id();
            let thread_enabled = enabled.clone();
            let _ = thread::Builder::new()
                .name("gibson-wallpaper-placement".into())
                .spawn(move || {
                    let mut monitors = Vec::new();
                    let mut next_monitor_poll = Instant::now();
                    let mut last = None;
                    loop {
                        if !thread_enabled.load(Ordering::Relaxed) {
                            thread::sleep(CLIENT_POLL_INTERVAL);
                            continue;
                        }
                        let now = Instant::now();
                        if monitors.is_empty() || now >= next_monitor_poll {
                            if let Some(next) = query::<Vec<HyprMonitor>>(&socket, "j/monitors") {
                                monitors = next;
                            }
                            next_monitor_poll = now + MONITOR_POLL_INTERVAL;
                        }
                        if let Some(clients) = query::<Vec<HyprClient>>(&socket, "j/clients")
                            && let Some(client) =
                                clients.into_iter().find(|client| client.pid == pid)
                            && let Some(monitor) =
                                monitors.iter().find(|monitor| monitor.id == client.monitor)
                            && let Some(placement) = placement(client, monitor)
                            && last != Some(placement)
                        {
                            if sender.send(placement).is_err() {
                                break;
                            }
                            last = Some(placement);
                        }
                        thread::sleep(CLIENT_POLL_INTERVAL);
                    }
                });
        }
        Self { updates, enabled }
    }

    pub fn poll(&self) -> Option<WallpaperPlacement> {
        self.updates.try_iter().last()
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }
}

#[derive(Deserialize)]
struct HyprClient {
    at: [f32; 2],
    size: [f32; 2],
    monitor: i32,
    pid: u32,
}

#[derive(Deserialize)]
struct HyprMonitor {
    id: i32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    scale: f32,
    transform: u8,
}

fn placement(client: HyprClient, monitor: &HyprMonitor) -> Option<WallpaperPlacement> {
    let scale = monitor.scale.max(f32::EPSILON);
    let rotated = matches!(monitor.transform, 1 | 3 | 5 | 7);
    let (physical_width, physical_height) = if rotated {
        (monitor.height, monitor.width)
    } else {
        (monitor.width, monitor.height)
    };
    let monitor_width = physical_width / scale;
    let monitor_height = physical_height / scale;
    if monitor_width <= 0.0
        || monitor_height <= 0.0
        || client.size[0] <= 0.0
        || client.size[1] <= 0.0
    {
        return None;
    }
    Some(WallpaperPlacement {
        monitor_uv: [
            (client.at[0] - monitor.x) / monitor_width,
            (client.at[1] - monitor.y) / monitor_height,
            client.size[0] / monitor_width,
            client.size[1] / monitor_height,
        ],
        monitor_aspect: monitor_width / monitor_height,
    })
}

fn query<T: for<'de> Deserialize<'de>>(socket: &PathBuf, request: &str) -> Option<T> {
    let mut stream = UnixStream::connect(socket).ok()?;
    stream.write_all(request.as_bytes()).ok()?;
    stream.shutdown(std::net::Shutdown::Write).ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    serde_json::from_str(&response).ok()
}

fn hyprland_socket_path(signature: &str) -> PathBuf {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        let runtime_socket = PathBuf::from(runtime)
            .join("hypr")
            .join(signature)
            .join(".socket.sock");
        if runtime_socket.exists() {
            return runtime_socket;
        }
    }
    PathBuf::from("/tmp/hypr")
        .join(signature)
        .join(".socket.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_includes_reserved_monitor_space() {
        let client = HyprClient {
            at: [0.0, 26.0],
            size: [1536.0, 838.0],
            monitor: 0,
            pid: 1,
        };
        let monitor = HyprMonitor {
            id: 0,
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
            scale: 1.25,
            transform: 0,
        };

        let placement = placement(client, &monitor).unwrap();
        assert_eq!(placement.monitor_uv[0], 0.0);
        assert!((placement.monitor_uv[1] - 26.0 / 864.0).abs() < f32::EPSILON);
        assert_eq!(placement.monitor_uv[2], 1.0);
        assert!((placement.monitor_uv[3] - 838.0 / 864.0).abs() < f32::EPSILON);
        assert!((placement.monitor_aspect - 16.0 / 9.0).abs() < f32::EPSILON);
    }
}
