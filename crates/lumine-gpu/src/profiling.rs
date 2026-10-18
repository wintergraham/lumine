//! Opt-in, bounded GPU timestamp collection. Normal rendering never waits.
//! Read results with GpuRenderer::take_gpu_profiles after polling the device.
use std::{collections::VecDeque, sync::mpsc};
const QUERIES: u32 = 128;
const MAX_PENDING: usize = 8;
#[derive(Clone, Debug)]
pub struct GpuSpan {
    pub label: &'static str,
    pub offset_ms: f64,
    pub duration_ms: f64,
}
#[derive(Clone, Debug)]
pub struct GpuFrameProfile {
    pub frame: u64,
    pub duration_ms: f64,
    pub spans: Vec<GpuSpan>,
}
fn profile(frame: u64, labels: &[&'static str], ticks: &[u64], period: f64) -> Option<GpuFrameProfile> {
    let ticks = ticks.get(..labels.len() * 2)?;
    if labels.is_empty() || ticks.chunks_exact(2).any(|pair| pair[0] == 0 || pair[1] < pair[0]) {
        return None;
    }
    let base = *ticks.iter().min()?;
    let last = *ticks.iter().max()?;
    Some(GpuFrameProfile {
        frame,
        duration_ms: (last - base) as f64 * period,
        spans: labels.iter().enumerate().map(|(i, label)| GpuSpan {
            label,
            offset_ms: (ticks[2 * i] - base) as f64 * period,
            duration_ms: (ticks[2 * i + 1] - ticks[2 * i]) as f64 * period,
        }).collect(),
    })
}
pub(crate) struct PassStamp {
    set: wgpu::QuerySet,
    start: u32,
    end: u32,
}
impl PassStamp {
    pub fn compute_writes(&self) -> wgpu::ComputePassTimestampWrites<'_> {
        wgpu::ComputePassTimestampWrites {
            query_set: &self.set,
            beginning_of_pass_write_index: Some(self.start),
            end_of_pass_write_index: Some(self.end),
        }
    }
    pub fn writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.set,
            beginning_of_pass_write_index: Some(self.start),
            end_of_pass_write_index: Some(self.end),
        }
    }
}
struct Slot {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    labels: Vec<&'static str>,
    frame: u64,
    ready: Option<mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    overflowed: bool,
}
impl Slot {
    fn new(device: &wgpu::Device) -> Self {
        Self {
            set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("lumine GPU timings"),
                ty: wgpu::QueryType::Timestamp,
                count: QUERIES,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lumine timestamp resolve"),
                size: QUERIES as u64 * 8,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            read: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lumine timestamp readback"),
                size: QUERIES as u64 * 8,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            labels: Vec::new(),
            frame: 0,
            ready: None,
            overflowed: false,
        }
    }
}
pub(crate) struct Profiler {
    active: Option<Slot>,
    pending: VecDeque<Slot>,
    free: Vec<Slot>,
    completed: Vec<GpuFrameProfile>,
    period: f64,
    encoders: bool,
    frame: u64,
    pub dropped: u64,
}
impl Profiler {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then(|| Self {
                active: None,
                pending: VecDeque::new(),
                free: Vec::new(),
                completed: Vec::new(),
                period: queue.get_timestamp_period() as f64 / 1e6,
                // Metal's out-of-pass timestamps use blit encoders. Empty blits
                // can return zero timestamps, invalidating an otherwise valid
                // frame. Measure real render/compute pass boundaries instead.
                encoders: !cfg!(target_os = "macos") && device
                    .features()
                    .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS),
                frame: 0,
                dropped: 0,
            })
    }
    pub(crate) fn frame_id(&self) -> u64 { self.frame }
    pub fn begin(&mut self, device: &wgpu::Device) {
        self.poll();
        // No command buffer for an idle frame.
        if let Some(slot) = self.active.take() {
            self.free.push(slot);
        }
        self.frame += 1;
        if self.pending.len() >= MAX_PENDING {
            self.dropped += 1;
            return;
        }
        let mut slot = self.free.pop().unwrap_or_else(|| Slot::new(device));
        slot.labels.clear();
        slot.overflowed = false;
        slot.frame = self.frame;
        self.active = Some(slot);
    }
    pub fn pass(&mut self, label: &'static str) -> Option<PassStamp> {
        let slot = self.active.as_mut()?;
        let start = slot.labels.len() as u32 * 2;
        if start + 2 > QUERIES {
            if !slot.overflowed {
                self.dropped += 1;
                slot.overflowed = true;
            }
            return None;
        }
        slot.labels.push(label);
        Some(PassStamp {
            set: slot.set.clone(),
            start,
            end: start + 1,
        })
    }
    pub fn start(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        label: &'static str,
    ) -> Option<PassStamp> {
        if !self.encoders {
            return None;
        }
        let stamp = self.pass(label)?;
        encoder.write_timestamp(&stamp.set, stamp.start);
        Some(stamp)
    }
    pub fn end(encoder: &mut wgpu::CommandEncoder, stamp: Option<PassStamp>) {
        if let Some(stamp) = stamp {
            encoder.write_timestamp(&stamp.set, stamp.end);
        }
    }
    pub fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Some(slot) = self.active.take() else {
            return;
        };
        let count = slot.labels.len() as u32 * 2;
        // A prefix of a frame is not a valid total or per-stage sample.
        // Recycle the bounded slot without resolving its incomplete queries.
        if count == 0 || slot.overflowed {
            self.free.push(slot);
            return;
        }
        encoder.resolve_query_set(&slot.set, 0..count, &slot.resolve, 0);
        encoder.copy_buffer_to_buffer(&slot.resolve, 0, &slot.read, 0, count as u64 * 8);
        self.pending.push_back(slot);
    }
    pub fn submitted(&mut self) {
        for slot in &mut self.pending {
            if slot.ready.is_some() {
                continue;
            }
            let (tx, rx) = mpsc::channel();
            slot.read
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = tx.send(result);
                });
            slot.ready = Some(rx);
        }
    }
    fn poll(&mut self) {
        while let Some(slot) = self.pending.front() {
            let Some(result) = slot.ready.as_ref().and_then(|rx| rx.try_recv().ok()) else {
                break;
            };
            let mut slot = self.pending.pop_front().unwrap();
            if result.is_ok() {
                let data = slot.read.slice(..).get_mapped_range();
                let ticks: Vec<u64> = data
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|b| u64::from_ne_bytes(*b))
                    .collect();
                // A stalled consumer must not grow profiling memory indefinitely.
                if self.completed.len() < MAX_PENDING * 8
                    && let Some(profile) = profile(slot.frame, &slot.labels, &ticks, self.period)
                {
                    self.completed.push(profile);
                } else {
                    self.dropped += 1;
                }
                drop(data);
                slot.read.unmap();
            } else {
                self.dropped += 1;
            }
            slot.ready = None;
            self.free.push(slot);
        }
    }
    pub fn take(&mut self) -> Vec<GpuFrameProfile> {
        self.poll();
        std::mem::take(&mut self.completed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_or_reversed_gpu_queries_are_not_frame_durations() {
        let labels = ["uploads", "repaint"];
        assert!(profile(1, &labels, &[0, 0, 8_449_491_000, 8_449_492_000], 0.001).is_none());
        assert!(profile(1, &labels, &[100, 110, 130, 120], 0.001).is_none());
        assert!(profile(1, &labels, &[100, 110], 0.001).is_none());
        let valid = profile(1, &labels, &[100, 110, 120, 150], 0.001).unwrap();
        assert_eq!(valid.duration_ms, 0.05);
        assert_eq!(valid.spans[1].duration_ms, 0.03);
        assert_eq!(valid.spans[1].offset_ms, 0.02);
    }
    #[test]
    fn query_overflow_discards_partial_frame_and_recovers_next_frame() {
        let context = crate::GpuContext::new().unwrap();
        let device = &context.inner.device;
        let Some(mut profiler) = Profiler::new(device, &context.inner.queue) else {
            return; // This adapter does not expose timestamp queries.
        };
        profiler.begin(device);
        for _ in 0..QUERIES / 2 {
            assert!(profiler.pass("test").is_some());
        }
        assert!(profiler.pass("overflow").is_none());
        assert!(profiler.pass("overflow again").is_none());
        assert_eq!(profiler.dropped, 1);
        let mut encoder = device.create_command_encoder(&Default::default());
        profiler.resolve(&mut encoder);
        assert!(profiler.pending.is_empty());
        assert!(profiler.take().is_empty());
        profiler.begin(device);
        assert!(profiler.pass("next frame").is_some());
        assert_eq!(profiler.dropped, 1);
    }
}
