//! Decoded animation data and a monotonic, seekable frame timeline.
use crate::image::ImageData;
use std::{num::NonZeroU32, sync::Arc, time::Duration};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopCount {
    Infinite,
    Finite(NonZeroU32),
}
impl LoopCount {
    pub const ONCE: Self = Self::Finite(NonZeroU32::MIN);
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub image: Arc<ImageData>,
    pub duration: Duration,
}
#[derive(Debug)]
pub struct Animation {
    frames: Arc<[Frame]>,
    ends: Vec<Duration>,
    duration: Duration,
    loops: LoopCount,
}
impl Animation {
    pub fn new(frames: Vec<Frame>, loops: LoopCount) -> Result<Self, &'static str> {
        if frames.is_empty() || frames.len() > 4096 {
            return Err("animation requires 1..=4096 frames");
        }
        let size = (frames[0].image.width(), frames[0].image.height());
        let mut bytes = 0usize;
        let mut duration = Duration::ZERO;
        let mut ends = Vec::with_capacity(frames.len());
        for frame in &frames {
            if (frame.image.width(), frame.image.height()) != size {
                return Err("animation frame sizes must match");
            }
            if frame.duration < Duration::from_millis(10)
                || frame.duration > Duration::from_secs(86400)
            {
                return Err("frame duration must be between 10 ms and one day");
            }
            bytes = bytes
                .checked_add(frame.image.pixels().len())
                .ok_or("animation size overflow")?;
            if bytes > 64 * 1024 * 1024 {
                return Err("decoded animation exceeds 64 MiB");
            }
            duration = duration
                .checked_add(frame.duration)
                .ok_or("animation duration overflow")?;
            ends.push(duration);
        }
        Ok(Self {
            frames: frames.into(),
            ends,
            duration,
            loops,
        })
    }
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }
    pub fn loop_count(&self) -> LoopCount {
        self.loops
    }
    pub fn cycle_duration(&self) -> Duration {
        self.duration
    }
    /// Frame and time until its next boundary. None means finite playback ended
    /// (last frame retained) or the image has only one frame.
    pub fn at(&self, elapsed: Duration) -> (usize, Option<Duration>) {
        let nanos = self.duration.as_nanos();
        if let LoopCount::Finite(count) = self.loops
            && elapsed.as_nanos() >= nanos * u128::from(count.get())
        {
            return (self.frames.len() - 1, None);
        }
        if self.frames.len() == 1 {
            return (0, None);
        }
        let local = elapsed.as_nanos() % nanos;
        let index = self.ends.partition_point(|end| end.as_nanos() <= local);
        let remaining = self.ends[index].as_nanos() - local;
        (
            index,
            Some(Duration::new(
                (remaining / 1_000_000_000) as u64,
                (remaining % 1_000_000_000) as u32,
            )),
        )
    }
}
