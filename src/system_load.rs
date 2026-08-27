use std::fs;
use std::time::{Duration, Instant};

const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CpuTimes {
    idle: u64,
    total: u64,
}

pub struct SystemLoad {
    level: f32,
    initialized: bool,
    next_sample: Instant,
    previous_cpu: Option<CpuTimes>,
    processors: f32,
}

impl SystemLoad {
    pub fn new(now: Instant) -> Self {
        Self {
            level: 0.0,
            initialized: false,
            next_sample: now,
            previous_cpu: None,
            processors: std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
                as f32,
        }
    }

    pub fn sample(&mut self, now: Instant) -> f32 {
        if now < self.next_sample {
            return self.level;
        }
        self.next_sample = now + SAMPLE_INTERVAL;

        let current_cpu = fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|text| parse_cpu_times(&text));
        let cpu_busy = self
            .previous_cpu
            .zip(current_cpu)
            .and_then(|(previous, current)| busy_fraction(previous, current));
        self.previous_cpu = current_cpu;

        let scheduler_load = fs::read_to_string("/proc/loadavg")
            .ok()
            .and_then(|text| parse_load_average(&text))
            .map_or(0.0, |load| load / self.processors);
        let target = cpu_busy
            .unwrap_or(scheduler_load)
            .max(scheduler_load)
            .clamp(0.0, 1.0);

        if self.initialized {
            let blend = if target > self.level { 0.55 } else { 0.18 };
            self.level += (target - self.level) * blend;
        } else {
            self.level = target;
            self.initialized = true;
        }
        self.level
    }
}

fn parse_cpu_times(text: &str) -> Option<CpuTimes> {
    let mut fields = text.lines().next()?.split_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let mut values = [0_u64; 8];
    let mut count = 0;
    for (index, field) in fields.take(values.len()).enumerate() {
        values[index] = field.parse().ok()?;
        count += 1;
    }
    if count < 4 {
        return None;
    }
    Some(CpuTimes {
        idle: values[3] + if count > 4 { values[4] } else { 0 },
        total: values[..count].iter().sum(),
    })
}

fn parse_load_average(text: &str) -> Option<f32> {
    text.split_whitespace().next()?.parse().ok()
}

fn busy_fraction(previous: CpuTimes, current: CpuTimes) -> Option<f32> {
    let total = current.total.checked_sub(previous.total)?;
    if total == 0 {
        return None;
    }
    let idle = current.idle.saturating_sub(previous.idle).min(total);
    Some(1.0 - idle as f32 / total as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aggregate_linux_cpu_times() {
        assert_eq!(
            parse_cpu_times("cpu  10 2 3 85 5 1 4 0 0 0\ncpu0 1 2 3 4\n"),
            Some(CpuTimes {
                idle: 90,
                total: 110,
            })
        );
    }

    #[test]
    fn calculates_cpu_pressure_from_counter_deltas() {
        let previous = CpuTimes {
            idle: 80,
            total: 100,
        };
        let current = CpuTimes {
            idle: 90,
            total: 140,
        };
        assert_eq!(busy_fraction(previous, current), Some(0.75));
    }

    #[test]
    fn parses_the_one_minute_load_average() {
        assert_eq!(parse_load_average("3.25 2.00 1.00 2/100 42"), Some(3.25));
        assert_eq!(parse_load_average("unavailable"), None);
    }
}
