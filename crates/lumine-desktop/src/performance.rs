//! Bounded, opt-in frame telemetry and focus-aware developer pacing.
use lumine::profiling::Profiler;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
    time::Instant,
};

pub const FRAME_HISTORY: usize = 600;
pub const CPU_STAGES: [&str; 6] = [
    "input",
    "tasks",
    "update",
    "layout/damage",
    "encode/submit",
    "present",
];
#[derive(Clone, Debug)]
pub struct FrameSample {
    pub number: u64,
    pub at: Instant,
    pub interval_ms: Option<f64>,
    pub cpu_ms: [f64; 6],
    pub budget_ms: f64,
}
#[derive(Clone, Debug, Default)]
pub struct FrameSummary {
    pub samples: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub missed: usize,
}
pub fn summarize(samples: &[FrameSample]) -> FrameSummary {
    let mut times: Vec<_> = samples.iter().filter_map(|s| s.interval_ms).collect();
    times.sort_by(f64::total_cmp);
    if times.is_empty() {
        return FrameSummary::default();
    }
    let percentile = |p: f64| times[((times.len() as f64 * p).ceil() as usize).max(1) - 1];
    FrameSummary {
        samples: times.len(),
        p50_ms: percentile(0.5),
        p95_ms: percentile(0.95),
        p99_ms: percentile(0.99),
        max_ms: *times.last().unwrap(),
        missed: samples
            .iter()
            .filter(|s| s.interval_ms.is_some_and(|ms| ms > s.budget_ms * 1.1))
            .count(),
    }
}
#[derive(Clone, Default)]
pub struct Performance {
    pub profiler: Profiler,
    enabled: Rc<Cell<bool>>,
    paused: Rc<Cell<bool>>,
    uncapped: Rc<Cell<bool>>,
    focused: Rc<Cell<bool>>,
    drawable: Rc<Cell<bool>>,
    compositor_paced: Rc<Cell<bool>>,
    effective_uncapped: Rc<Cell<bool>>,
    gpu_supported: Rc<Cell<bool>>,
    pub(crate) gpu: Rc<RefCell<VecDeque<lumine_gpu::profiling::GpuFrameProfile>>>,
    gpu_dropped: Rc<Cell<u64>>,
    stats: Rc<Cell<lumine_gpu::GpuStats>>,
    frames: Rc<RefCell<VecDeque<FrameSample>>>,
    previous: Rc<Cell<Option<Instant>>>,
    work: Rc<Cell<[f64; 6]>>,
    number: Rc<Cell<u64>>,
    budget: Rc<Cell<f64>>,
    fps_since: Rc<Cell<Option<Instant>>>,
    fps_count: Rc<Cell<u64>>,
    fps_rate: Rc<Cell<f64>>,
    last_present: Rc<Cell<Option<Instant>>>,
    overlay: Rc<Cell<Option<lumine::scene::Rect>>>,
    continuous: Rc<Cell<bool>>,
    skip_next: Rc<Cell<bool>>,
}
impl Performance {
    pub fn new(profiler: Profiler) -> Self {
        Self {
            profiler,
            ..Default::default()
        }
    }
    pub fn enabled(&self) -> bool {
        self.enabled.get()
    }
    pub fn set_enabled(&self, enabled: bool) {
        if self.enabled.replace(enabled) != enabled {
            self.reset();
        }
        self.profiler.set_enabled(enabled && !self.paused());
    }
    pub fn paused(&self) -> bool {
        self.paused.get()
    }
    pub fn set_paused(&self, paused: bool) {
        self.paused.set(paused);
        self.profiler.set_enabled(self.enabled() && !paused);
        self.break_sequence();
    }
    pub fn reset(&self) {
        self.stats.set(lumine_gpu::GpuStats::default());
        self.frames.borrow_mut().clear();
        self.gpu.borrow_mut().clear();
        self.number.set(0);
        self.profiler.reset();
        self.fps_since.set(None);
        self.fps_count.set(0);
        self.fps_rate.set(0.);
        self.last_present.set(None);
        self.break_sequence();
    }
    pub fn continuous(&self) -> bool {
        self.continuous.get()
    }
    pub fn set_continuous(&self, enabled: bool) {
        self.continuous.set(enabled);
    }
    pub fn set_overlay_bounds(&self, bounds: Option<lumine::scene::Rect>) {
        self.overlay.set(bounds);
    }
    pub fn skip_next_presentation(&self) {
        self.skip_next.set(true);
        self.break_sequence();
    }
    pub(crate) fn content_damage(
        &self,
        damage: &[lumine::scene::Rect],
        other_overlay: Option<lumine::scene::Rect>,
    ) -> bool {
        let inside = |r: &lumine::scene::Rect, o: lumine::scene::Rect| {
            r.x >= o.x
                && r.y >= o.y
                && r.x + r.width <= o.x + o.width
                && r.y + r.height <= o.y + o.height
        };
        self.enabled()
            && damage.iter().any(|r| {
                !self.overlay.get().is_some_and(|o| inside(r, o))
                    && !other_overlay.is_some_and(|o| inside(r, o))
            })
    }
    pub fn uncapped(&self) -> bool {
        self.uncapped.get()
    }
    pub fn set_uncapped(&self, uncapped: bool) {
        self.uncapped.set(uncapped);
    }
    pub fn drawable(&self) -> bool {
        self.drawable.get()
    }
    pub(crate) fn set_compositor_paced(&self, paced: bool) {
        self.compositor_paced.set(paced);
    }
    pub fn focused(&self) -> bool {
        self.focused.get()
    }
    pub fn effective_uncapped(&self) -> bool {
        self.effective_uncapped.get()
    }
    pub fn gpu_supported(&self) -> bool {
        self.gpu_supported.get()
    }
    pub fn render_stats(&self) -> lumine_gpu::GpuStats {
        self.stats.get()
    }
    pub(crate) fn record_stats(&self, stats: lumine_gpu::GpuStats) {
        if self.enabled() && !self.paused() {
            self.stats.set(stats);
        }
    }
    pub fn gpu_dropped(&self) -> u64 {
        self.gpu_dropped.get()
    }
    pub fn live_fps(&self) -> f64 {
        if self
            .last_present
            .get()
            .is_none_or(|last| last.elapsed().as_secs_f64() > 1.)
        {
            return 0.;
        }
        self.fps_rate.get()
    }
    pub fn frames(&self) -> Vec<FrameSample> {
        self.frames.borrow().iter().cloned().collect()
    }
    pub fn gpu_frames(&self) -> Vec<lumine_gpu::profiling::GpuFrameProfile> {
        self.gpu.borrow().iter().cloned().collect()
    }
    pub fn budget_ms(&self) -> f64 {
        self.budget.get()
    }
    pub(crate) fn sync(&self, focused: bool, drawable: bool, budget_ms: f64) -> bool {
        self.focused.set(focused);
        self.drawable.set(drawable);
        self.budget.set(budget_ms);
        let uncapped = self.uncapped() && focused && drawable && !self.compositor_paced.get();
        if self.effective_uncapped.replace(uncapped) != uncapped {
            self.break_sequence();
        }
        uncapped
    }
    pub(crate) fn set_gpu_status(&self, supported: bool, dropped: u64) {
        self.gpu_supported.set(supported);
        self.gpu_dropped.set(dropped);
    }
    pub(crate) fn work(&self, stage: usize) -> Option<WorkGuard> {
        (self.enabled() && !self.paused()).then(|| WorkGuard {
            monitor: self.clone(),
            stage,
            start: Instant::now(),
            excluded_before: self.profiler.excluded_cost_ms(),
        })
    }
    pub(crate) fn break_sequence(&self) {
        self.previous.set(None);
        self.work.set([0.; 6]);
    }
    pub(crate) fn presented(&self, now: Instant, content: bool, continuous: bool) {
        if !self.enabled() || self.paused() {
            return;
        }
        if self.skip_next.replace(false) {
            self.break_sequence();
            return;
        }
        if !content {
            self.work.set([0.; 6]);
            return;
        }
        let interval_ms = self
            .previous
            .replace(continuous.then_some(now))
            .filter(|_| continuous)
            .map(|prev| now.duration_since(prev).as_secs_f64() * 1000.);
        if self
            .last_present
            .get()
            .is_none_or(|last| now.saturating_duration_since(last).as_secs_f64() > 1.)
        {
            self.fps_since.set(Some(now));
            self.fps_count.set(0);
            self.fps_rate.set(0.);
        }
        self.last_present.set(Some(now));
        self.fps_count.set(self.fps_count.get() + 1);
        let since = self.fps_since.get().unwrap_or(now);
        let elapsed = now.saturating_duration_since(since).as_secs_f64();
        if elapsed >= 1. {
            self.fps_rate.set(self.fps_count.get() as f64 / elapsed);
            self.fps_count.set(0);
            self.fps_since.set(Some(now));
        }
        self.number.set(self.number.get().saturating_add(1));
        let sample = FrameSample {
            number: self.number.get(),
            at: now,
            interval_ms,
            cpu_ms: self.work.replace([0.; 6]),
            budget_ms: self.budget.get(),
        };
        let mut frames = self.frames.borrow_mut();
        if frames.len() == FRAME_HISTORY {
            frames.pop_front();
        }
        frames.push_back(sample);
    }
}
pub(crate) struct WorkGuard {
    monitor: Performance,
    stage: usize,
    start: Instant,
    excluded_before: f64,
}
impl Drop for WorkGuard {
    fn drop(&mut self) {
        let mut work = self.monitor.work.get();
        let excluded = (self.monitor.profiler.excluded_cost_ms() - self.excluded_before).max(0.);
        work[self.stage] += (self.start.elapsed().as_secs_f64() * 1000. - excluded).max(0.);
        self.monitor.work.set(work);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn focus_and_visibility_gate_uncapping_and_idle_gaps_are_excluded() {
        let monitor = Performance::default();
        monitor.set_enabled(true);
        monitor.set_uncapped(true);
        assert!(monitor.sync(true, true, 10.));
        assert!(!monitor.sync(false, true, 10.));
        assert!(!monitor.sync(true, false, 10.));
        assert!(monitor.sync(true, true, 10.));
        let now = Instant::now();
        monitor.presented(now, true, true);
        monitor.presented(now + Duration::from_millis(10), true, true);
        monitor.presented(now + Duration::from_millis(60), true, true);
        monitor.break_sequence();
        monitor.presented(now + Duration::from_secs(30), true, true);
        let stats = summarize(&monitor.frames());
        assert_eq!(stats.samples, 2);
        assert_eq!(stats.p99_ms, 50.);
        assert_eq!(stats.missed, 1);
        monitor.set_paused(true);
        monitor.presented(now + Duration::from_secs(31), true, true);
        assert_eq!(monitor.frames().len(), 4);
        monitor.set_paused(false);
        monitor.reset();
        monitor.skip_next_presentation();
        monitor.presented(now, false, true);
        monitor.presented(now + Duration::from_millis(10), true, true);
        assert_eq!(
            monitor.frames().len(),
            1,
            "tool-only skip leaked into the next content frame"
        );
    }
}
