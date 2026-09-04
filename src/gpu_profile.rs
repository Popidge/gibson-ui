use crossbeam_channel::{Receiver, bounded};
use std::time::{Duration, Instant};

const PASSES: [&str; 8] = [
    "navigator",
    "labels",
    "opaque",
    "blur_x",
    "blur_y",
    "copy",
    "glass",
    "ui",
];
const BUFFER_SIZE: u64 = (PASSES.len() * 2 * size_of::<u64>()) as u64;

/// Samples GPU passes without waiting for the GPU on the render thread.
pub struct GpuProfile {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    pending: Option<Receiver<Result<(), wgpu::BufferAsyncError>>>,
    recording: bool,
    encoder_timestamps: bool,
    used: [bool; PASSES.len()],
    next_sample: Instant,
    last_log: Instant,
    totals: [f64; PASSES.len()],
    samples: u32,
}

impl GpuProfile {
    pub fn new(device: &wgpu::Device) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        Some(Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("performance timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: (PASSES.len() * 2) as u32,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("performance resolve"),
                size: BUFFER_SIZE,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("performance readback"),
                size: BUFFER_SIZE,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            pending: None,
            recording: false,
            encoder_timestamps: device
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS),
            used: [false; PASSES.len()],
            next_sample: Instant::now(),
            last_log: Instant::now(),
            totals: [0.0; PASSES.len()],
            samples: 0,
        })
    }

    pub fn begin_frame(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enabled: bool) {
        if self.pending.is_some() {
            let _ = device.poll(wgpu::PollType::Poll);
        }
        if let Some(result) = self.pending.as_ref().and_then(|rx| rx.try_recv().ok()) {
            if result.is_ok() {
                if let Ok(data) = self.readback.slice(..).get_mapped_range() {
                    let times: &[u64] = bytemuck::cast_slice(&data);
                    for (index, used) in self.used.iter().enumerate() {
                        if *used {
                            self.totals[index] +=
                                times[index * 2 + 1].saturating_sub(times[index * 2]) as f64
                                    * f64::from(queue.get_timestamp_period())
                                    / 1_000_000.0;
                        }
                    }
                    self.samples += 1;
                }
                self.readback.unmap();
            }
            self.pending = None;
        }
        if self.last_log.elapsed() >= Duration::from_secs(2) && self.samples > 0 {
            if enabled {
                let timings = PASSES
                    .iter()
                    .zip(self.totals)
                    .map(|(name, total)| {
                        format!("{name}_ms={:.3}", total / f64::from(self.samples))
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                eprintln!("GIBSON GPU samples={} {timings}", self.samples);
            }
            self.totals.fill(0.0);
            self.samples = 0;
            self.last_log = Instant::now();
        }
        self.recording = enabled && self.pending.is_none() && Instant::now() >= self.next_sample;
        if self.recording {
            self.used.fill(false);
            self.next_sample = Instant::now() + Duration::from_millis(100);
        }
    }

    pub fn writes(&mut self, pass: usize) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        if !self.recording {
            return None;
        }
        self.used[pass] = true;
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(pass as u32 * 2),
            end_of_pass_write_index: Some(pass as u32 * 2 + 1),
        })
    }

    pub fn copy_timestamp(&mut self, encoder: &mut wgpu::CommandEncoder, end: bool) {
        if self.recording && self.encoder_timestamps {
            self.used[5] = true;
            encoder.write_timestamp(&self.queries, 10 + u32::from(end));
        }
    }

    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.recording {
            encoder.resolve_query_set(
                &self.queries,
                0..(PASSES.len() * 2) as u32,
                &self.resolve,
                0,
            );
            encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, BUFFER_SIZE);
        }
    }

    pub fn submitted(&mut self) {
        if self.recording {
            let (tx, rx) = bounded(1);
            self.readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = tx.send(result);
                });
            self.pending = Some(rx);
            self.recording = false;
        }
    }
}
