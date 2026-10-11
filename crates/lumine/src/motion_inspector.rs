//! Opt-in, on-demand inspection. No polling or frame requests of its own.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderSnapshot {
    pub frame: u64,
    pub layout_nodes: usize,
    pub shader_dispatches: usize,
    pub blur_passes: usize,
    pub shader_allocations: usize,
}
#[derive(Clone, Default)]
pub struct RenderDiagnostics(Rc<Cell<RenderSnapshot>>);
impl RenderDiagnostics {
    pub fn snapshot(&self) -> RenderSnapshot {
        self.0.get()
    }
    pub fn record(&self, snapshot: RenderSnapshot) {
        self.0.set(snapshot);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MotionId {
    domain: u64,
    track: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackState {
    Idle,
    Running,
    Delayed,
    Paused,
}
#[derive(Clone, Debug)]
pub struct TrackSnapshot {
    pub id: MotionId,
    pub label: String,
    pub value: f32,
    pub velocity: f32,
    pub state: TrackState,
    pub elapsed: Duration,
    pub duration: Option<Duration>,
    pub rate: f64,
}
#[derive(Clone, Debug, Default)]
pub struct MotionSnapshot {
    pub tracks: Vec<TrackSnapshot>,
    pub active_tracks: usize,
    pub total_tracks: usize,
    pub registry_truncated: bool,
    /// Shared display clock demand, including non-motion consumers.
    pub display_frame_requested: bool,
    pub last_sample_cost: Duration,
    pub last_sampled_tracks: usize,
    pub renderer: Option<RenderSnapshot>,
}
struct InspectorOwner {
    scheduler: Rc<Scheduler>,
    alive: Rc<Cell<bool>>,
}
impl Drop for InspectorOwner {
    fn drop(&mut self) {
        self.alive.set(false);
        self.scheduler
            .profiling
            .set(self.scheduler.profiling.get() - 1);
    }
}
#[derive(Clone)]
pub struct MotionInspector {
    scheduler: Weak<Scheduler>,
    alive: Rc<Cell<bool>>,
    renderer: Option<Rc<RenderDiagnostics>>,
}
impl MotionInspector {
    pub fn new(cx: &mut Context) -> Self {
        let scheduler = scheduler_for(cx);
        scheduler.profiling.set(scheduler.profiling.get() + 1);
        let alive = Rc::new(Cell::new(true));
        let this = Self {
            scheduler: Rc::downgrade(&scheduler),
            alive: alive.clone(),
            renderer: cx.try_service(),
        };
        cx.retain(InspectorOwner { scheduler, alive });
        this
    }
    pub fn snapshot(&self) -> MotionSnapshot {
        let Some(scheduler) = self.scheduler.upgrade().filter(|_| self.alive.get()) else {
            return MotionSnapshot::default();
        };
        let domain = scheduler.id;
        let mut registered = scheduler.registered.borrow_mut();
        registered.retain(|track| track.upgrade().is_some_and(|t| t.alive.get()));
        let tracks = registered
            .iter()
            .filter_map(Weak::upgrade)
            .map(|track| {
                let run = track.run.borrow();
                let state = match run.as_ref() {
                    None => TrackState::Idle,
                    Some(_) if !track.active() => TrackState::Paused,
                    Some(run) if run.elapsed < run.transition.delay => TrackState::Delayed,
                    Some(_) => TrackState::Running,
                };
                TrackSnapshot {
                    id: MotionId {
                        domain,
                        track: track.id,
                    },
                    label: track.label.borrow().clone(),
                    value: track.value.with_untracked(|v| *v),
                    velocity: track.velocity.get(),
                    state,
                    elapsed: run.as_ref().map_or(Duration::ZERO, |r| {
                        r.elapsed.saturating_sub(r.transition.delay)
                    }),
                    duration: run.as_ref().and_then(|r| match r.transition.kind {
                        AnimationKind::Tween { duration, .. } => Some(duration),
                        _ => None,
                    }),
                    rate: track.rate.get(),
                }
            })
            .collect();
        MotionSnapshot {
            tracks,
            total_tracks: scheduler.live_tracks.get(),
            registry_truncated: scheduler.live_tracks.get() > registered.len(),
            active_tracks: scheduler
                .tracks
                .borrow()
                .iter()
                .filter(|t| t.alive.get() && t.active() && t.run.borrow().is_some())
                .count(),
            display_frame_requested: scheduler.frames.wants_frame(),
            last_sample_cost: scheduler.sample_cost.get(),
            last_sampled_tracks: scheduler.sampled.get(),
            renderer: self.renderer.as_ref().map(|r| r.snapshot()),
        }
    }
    fn control(&self, id: MotionId, act: impl FnOnce(MotionValue)) -> bool {
        let Some(scheduler) = self.scheduler.upgrade().filter(|_| self.alive.get()) else {
            return false;
        };
        if id.domain != scheduler.id {
            return false;
        }
        let track = scheduler
            .registered
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .find(|t| t.id == id.track && t.alive.get());
        if let Some(track) = track {
            act(MotionValue { track, scheduler });
            true
        } else {
            false
        }
    }
    pub fn pause(&self, id: MotionId) -> bool {
        self.control(id, |v| v.pause())
    }
    pub fn resume(&self, id: MotionId) -> bool {
        self.control(id, |v| v.resume())
    }
    pub fn stop(&self, id: MotionId) -> bool {
        self.control(id, |v| v.stop())
    }
    pub fn seek(&self, id: MotionId, time: Duration) -> bool {
        self.control(id, |v| v.seek(time))
    }
    pub fn set_rate(&self, id: MotionId, rate: f64) -> bool {
        if !rate.is_finite() || !(0.01..=100.).contains(&rate) {
            return false;
        }
        self.control(id, |v| v.set_rate(rate))
    }
}
