//! Owned, display-paced scalar animation. Values on the same frame clock and
//! reactive runtime share one task; only running values are sampled and all
//! writes are batched before effects run. No work is requested while idle.
use crate::{
    compose::{Context, TaskRunner, TaskToken},
    frame::{Frame, FrameClock, NextFrame},
    reactive::{Runtime, Signal},
};
use std::{
    cell::{Cell, RefCell},
    future::Future,
    pin::Pin,
    rc::{Rc, Weak},
    task::{Context as PollContext, Poll, Waker},
    time::{Duration, Instant},
};

#[path = "motion_math.rs"]
mod math;
use math::Oscillator;
pub use math::{AnimationKind, Bezier, Easing, Spring, Transition};
#[path = "motion_inspector.rs"]
mod inspector;
#[path = "motion_tracks.rs"]
mod tracks;
pub use inspector::{
    MotionId, MotionInspector, MotionSnapshot, RenderDiagnostics, RenderSnapshot, TrackSnapshot,
    TrackState,
};
#[path = "motion_bindings.rs"]
mod bindings;
pub(crate) use bindings::project_layout;
pub use bindings::{DragMotion, MotionAxis, MotionStates, ScrollProgress, SharedLayoutScope};
pub use tracks::{
    AnimationGroup, Interpolate, Keyframe, Keyframes, MotionColor, MotionError, MotionPoint,
    Playback, Repeat, Timeline, Vec2,
};

/// Optional subtree policy, supplied with `compose::provide`. Pausing freezes
/// time and velocity; reduced motion immediately finishes at the target.
#[derive(Clone)]
pub struct MotionPolicy {
    pub active: Signal<bool>,
    pub reduced: Signal<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completion {
    Finished,
    Cancelled,
}
struct Run {
    from: f32,
    target: f32,
    transition: Transition,
    oscillator: Option<Oscillator>,
    elapsed: Duration,
    last: Option<Instant>,
    keyframes: Option<Keyframes<f32>>,
    playback: Playback,
}
impl Run {
    fn sample(&self) -> (f32, f32, bool) {
        if self.elapsed < self.transition.delay {
            return (self.from, 0., false);
        }
        let elapsed = self.elapsed.saturating_sub(self.transition.delay);
        match self.transition.kind {
            AnimationKind::Tween { duration, easing } => {
                let (t, finished) = self.playback.phase(elapsed, duration);
                let value = self.keyframes.as_ref().map_or_else(
                    || self.from.interpolate(self.target, easing.sample(t)),
                    |frames| frames.sample(t),
                );
                (value, 0., finished)
            }
            AnimationKind::Spring(spring) => {
                let (displacement, velocity) =
                    self.oscillator.unwrap().sample(elapsed.as_secs_f64());
                (
                    finite_scalar(self.target as f64 + displacement),
                    finite_scalar(velocity),
                    displacement.abs() <= spring.rest_delta as f64
                        && velocity.abs() <= spring.rest_speed as f64,
                )
            }
        }
    }
}
struct Track {
    value: Signal<f32>,
    velocity: Cell<f32>,
    completion: RefCell<Signal<Option<Completion>>>,
    run: RefCell<Option<Run>>,
    policy: Option<MotionPolicy>,
    alive: Cell<bool>,
    queued: Cell<bool>,
    paused: Cell<bool>,
    rate: Cell<f64>,
    id: u64,
    label: RefCell<String>,
}
impl Track {
    fn active(&self) -> bool {
        !self.paused.get()
            && self
                .policy
                .as_ref()
                .is_none_or(|p| p.active.with_untracked(|v| *v))
    }
    fn reduced(&self) -> bool {
        self.policy
            .as_ref()
            .is_some_and(|p| p.reduced.with_untracked(|v| *v))
    }
    fn finish(&self, result: Completion) {
        let run = self.run.borrow_mut().take();
        if run.is_none() {
            return;
        }
        self.velocity.set(0.0);
        if result == Completion::Finished
            && let Some(run) = run
        {
            self.value.set(run.target);
        }
        let completion = self.completion.borrow().clone();
        completion.set(Some(result));
    }
    fn accrue_active_time(&self) {
        if let Some(run) = self.run.borrow_mut().as_mut()
            && let Some(last) = run.last.take()
            && !self.paused.get()
        {
            run.elapsed = run.elapsed.saturating_add(scale_duration(
                Instant::now().saturating_duration_since(last),
                self.rate.get(),
            ));
        }
    }
    fn step(&self, frame: Frame) {
        let (value, velocity, finished) = {
            let mut run = self.run.borrow_mut();
            let Some(run) = run.as_mut() else {
                return;
            };
            let elapsed = run.last.map_or(frame.interval, |last| {
                frame.time.saturating_duration_since(last)
            });
            run.elapsed = run
                .elapsed
                .saturating_add(scale_duration(elapsed, self.rate.get()));
            run.last = Some(frame.time);
            if run.elapsed < run.transition.delay {
                return;
            }
            run.sample()
        };
        if finished {
            self.finish(Completion::Finished);
        } else {
            self.velocity.set(velocity);
            self.value.set(value);
        }
    }
}

// Intermediate differences and spring velocities can exceed f32 even when
// both endpoints are finite. Keep the solver wide, then saturate UI scalars.
fn finite_scalar(value: f64) -> f32 {
    value.clamp(-(f32::MAX as f64), f32::MAX as f64) as f32
}
fn scale_duration(duration: Duration, rate: f64) -> Duration {
    Duration::try_from_secs_f64(duration.as_secs_f64() * rate).unwrap_or(Duration::MAX)
}

fn scheduler_for(cx: &Context) -> Rc<Scheduler> {
    let runtime = cx.runtime();
    let frames = cx.try_service::<FrameClock>();
    // Headless trees without BOTH services settle synchronously.
    let runner = frames
        .as_ref()
        .and_then(|_| cx.try_service::<TaskRunner>())
        .map(|r| (*r).clone());
    match (frames, runner) {
        (Some(frames), Some(runner)) => frames.motion_scheduler(runtime.clone(), Some(runner)),
        _ => Scheduler::new(runtime.clone(), FrameClock::new(), None),
    }
}

static NEXT_SCHEDULER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(crate) struct Scheduler {
    id: u64,
    runtime: Runtime,
    frames: FrameClock,
    runner: Option<TaskRunner>,
    tracks: RefCell<Vec<Rc<Track>>>,
    wake: RefCell<Option<Waker>>,
    task: RefCell<Option<TaskToken>>,
    registered: RefCell<Vec<Weak<Track>>>,
    next_id: Cell<u64>,
    profiling: Cell<usize>,
    live_tracks: Cell<usize>,
    sample_cost: Cell<Duration>,
    sampled: Cell<usize>,
}
impl Scheduler {
    pub(crate) fn matches(&self, runtime: &Runtime) -> bool {
        self.runtime.same(runtime)
    }
    pub(crate) fn new(
        runtime: Runtime,
        frames: FrameClock,
        runner: Option<TaskRunner>,
    ) -> Rc<Self> {
        let this = Rc::new(Self {
            id: NEXT_SCHEDULER_ID
                .fetch_update(
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                    |id| id.checked_add(1),
                )
                .expect("motion scheduler id overflow"),
            runtime,
            frames,
            runner,
            tracks: RefCell::default(),
            wake: RefCell::default(),
            task: RefCell::default(),
            registered: RefCell::default(),
            next_id: Cell::new(1),
            profiling: Cell::new(0),
            live_tracks: Cell::new(0),
            sample_cost: Cell::new(Duration::ZERO),
            sampled: Cell::new(0),
        });
        this
    }
    fn notify(&self) {
        let wake = self.wake.borrow_mut().take();
        if let Some(wake) = wake {
            wake.wake();
        }
    }
    fn changed(&self) {
        self.notify();
    }
    fn enqueue(self: &Rc<Self>, track: &Rc<Track>) {
        if !track.queued.replace(true) {
            self.tracks.borrow_mut().push(track.clone());
        }
        self.changed();
        if self.task.borrow().is_none()
            && let Some(runner) = &self.runner
        {
            let task = runner.spawn_owned(Driver {
                scheduler: Rc::downgrade(self),
                frame: None,
                sleep: None,
            });
            *self.task.borrow_mut() = Some(task);
        }
    }
    fn prune(&self) {
        self.tracks.borrow_mut().retain(|track| {
            let keep = track.alive.get() && track.run.borrow().is_some();
            if !keep {
                track.queued.set(false);
            }
            keep
        });
    }
    fn step(&self, frame: Frame) {
        let started = (self.profiling.get() > 0).then(Instant::now);
        let mut sampled = 0;
        self.runtime.batch(|| {
            // No user effects run until every track is sampled and the vector
            // borrow is released. A reentrant retarget joins the next frame.
            for track in self.tracks.borrow().iter() {
                if track.alive.get() && track.active() {
                    sampled += 1;
                    track.step(frame);
                }
            }
            self.prune();
        });
        if let Some(started) = started {
            self.sample_cost.set(started.elapsed());
            self.sampled.set(sampled);
        }
    }
}

struct Driver {
    scheduler: Weak<Scheduler>,
    frame: Option<NextFrame>,
    sleep: Option<(Instant, crate::timer::Sleep)>,
}
impl Future for Driver {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut PollContext<'_>) -> Poll<()> {
        let Some(scheduler) = self.scheduler.upgrade() else {
            return Poll::Ready(());
        };
        *scheduler.wake.borrow_mut() = Some(cx.waker().clone());
        scheduler.prune();
        let now = Instant::now();
        let mut wants_frame = false;
        let mut earliest = None::<Instant>;
        for track in scheduler
            .tracks
            .borrow()
            .iter()
            .filter(|track| track.active())
        {
            let mut run = track.run.borrow_mut();
            let Some(run) = run.as_mut() else {
                continue;
            };
            // Resuming starts a new presentation interval, excluding pause time.
            let remaining = scale_duration(
                run.transition.delay.saturating_sub(run.elapsed),
                1. / track.rate.get(),
            );
            let last = if remaining.is_zero() {
                run.last.unwrap_or(now)
            } else {
                *run.last.get_or_insert(now)
            };
            // An unrepresentable deadline cannot be reached by this clock.
            let Some(deadline) = last.checked_add(remaining) else {
                continue;
            };
            if remaining.is_zero() || deadline <= now {
                wants_frame = true;
            } else {
                earliest = Some(earliest.map_or(deadline, |old| old.min(deadline)));
            }
        }
        if !wants_frame {
            self.frame = None;
            if let Some(deadline) = earliest {
                if self.sleep.as_ref().is_none_or(|(old, _)| *old != deadline) {
                    self.sleep = Some((deadline, crate::timer::sleep_until(deadline)));
                }
                if Pin::new(&mut self.sleep.as_mut().unwrap().1)
                    .poll(cx)
                    .is_ready()
                {
                    cx.waker().wake_by_ref();
                }
            } else {
                self.sleep = None;
            }
            return Poll::Pending;
        }
        self.sleep = None;
        if self.frame.is_none() {
            self.frame = Some(scheduler.frames.next());
        }
        if let Poll::Ready(frame) = Pin::new(self.frame.as_mut().unwrap()).poll(cx) {
            self.frame = None;
            scheduler.step(frame);
            // Register the next refresh now. The clock only wakes this driver
            // on delivery; there is no extra executor turn per animation frame.
            return self.poll(cx);
        }
        Poll::Pending
    }
}

struct Owner {
    track: Rc<Track>,
    scheduler: Rc<Scheduler>,
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.track.alive.set(false);
        self.scheduler
            .live_tracks
            .set(self.scheduler.live_tracks.get() - 1);
        self.scheduler
            .runtime
            .batch(|| self.track.finish(Completion::Cancelled));
        self.scheduler.changed();
    }
}

/// A component-owned scalar. Clones cannot keep a disposed component animating.
/// The signal is exposed for retained styles; use `set` to interrupt animation.
#[derive(Clone)]
pub struct MotionValue {
    track: Rc<Track>,
    scheduler: Rc<Scheduler>,
}
impl MotionValue {
    pub fn new(cx: &mut Context, initial: f32) -> Self {
        Self::with_policy(
            cx,
            initial,
            cx.try_service::<MotionPolicy>().map(|p| (*p).clone()),
        )
    }
    pub fn with_policy(cx: &mut Context, initial: f32, policy: Option<MotionPolicy>) -> Self {
        assert!(initial.is_finite(), "non-finite motion value");
        let scheduler = scheduler_for(cx);
        let runtime = scheduler.runtime.clone();
        let track = Rc::new(Track {
            value: runtime.signal(initial),
            velocity: Cell::new(0.0),
            completion: RefCell::new(runtime.signal(Some(Completion::Finished))),
            run: RefCell::default(),
            policy,
            alive: Cell::new(true),
            queued: Cell::new(false),
            paused: Cell::new(false),
            rate: Cell::new(1.),
            id: scheduler.next_id.get(),
            label: RefCell::default(),
        });
        scheduler.live_tracks.set(scheduler.live_tracks.get() + 1);
        scheduler.next_id.set(
            scheduler
                .next_id
                .get()
                .checked_add(1)
                .expect("motion track ID exhausted"),
        );
        let mut registered = scheduler.registered.borrow_mut();
        if registered.len() >= 4096 {
            registered.retain(|track| track.upgrade().is_some_and(|t| t.alive.get()));
        }
        if registered.len() < 4096 {
            registered.push(Rc::downgrade(&track));
        }
        drop(registered);
        if let Some(policy) = track.policy.clone() {
            let weak_track = Rc::downgrade(&track);
            let weak_scheduler = Rc::downgrade(&scheduler);
            // Policy is immutable per track. Observe it once, without rescanning
            // the entire active set on every start, interruption or disposal.
            cx.retain(runtime.effect(move || {
                let active = policy.active.get();
                let reduced = policy.reduced.get();
                if let (Some(track), Some(scheduler)) =
                    (weak_track.upgrade(), weak_scheduler.upgrade())
                {
                    scheduler.runtime.batch(|| {
                        if !active {
                            track.accrue_active_time();
                        }
                        if reduced && track.run.borrow().is_some() {
                            track.finish(Completion::Finished);
                        }
                    });
                    scheduler.notify();
                }
            }));
        }
        cx.retain(Owner {
            track: track.clone(),
            scheduler: scheduler.clone(),
        });
        Self { track, scheduler }
    }
    pub fn signal(&self) -> Signal<f32> {
        self.track.value.clone()
    }
    pub fn get(&self) -> f32 {
        self.track.value.get()
    }
    pub fn velocity(&self) -> f32 {
        self.track.velocity.get()
    }
    pub fn animate_to(&self, target: f32, transition: Transition) -> Animation {
        assert!(target.is_finite(), "non-finite animation target");
        transition.validate();
        if !self.track.alive.get() {
            return Animation {
                result: self.scheduler.runtime.signal(Some(Completion::Cancelled)),
                runtime: self.scheduler.runtime.clone(),
            };
        }
        let result = self.scheduler.runtime.signal(None);
        self.scheduler.runtime.batch(|| {
            // Each handle retains its own result: a completed historical run
            // stays Finished after a later retarget, while pending runs cancel.
            let previous = self.track.completion.replace(result.clone());
            if previous.with_untracked(|v| v.is_none()) { previous.set(Some(Completion::Cancelled)); }
            let from = self.track.value.with_untracked(|v| *v);
            let velocity = self.track.velocity.get();
            let oscillator = match transition.kind {
                AnimationKind::Spring(spring) => Some(Oscillator::new(spring, from as f64 - target as f64, velocity as f64)),
                _ => { self.track.velocity.set(0.0); None }
            };
            let now = Instant::now();
            let start = self.scheduler.frames.last().map_or(now, |f| f.time.max(now));
            *self.track.run.borrow_mut() = Some(Run { from, target, transition, oscillator, elapsed: Duration::ZERO, last: self.track.active().then_some(start), keyframes: None, playback: Playback::default() });
            let instant = matches!(transition.kind, AnimationKind::Tween { duration, .. } if duration.is_zero()) && transition.delay.is_zero();
            if self.scheduler.runner.is_none() || self.track.reduced() || instant || (from == target && velocity == 0.0) {
                self.track.finish(Completion::Finished);
                self.scheduler.changed();
            } else { self.scheduler.enqueue(&self.track); }
        });
        Animation {
            result,
            runtime: self.scheduler.runtime.clone(),
        }
    }
    pub fn animation(&self) -> Animation {
        Animation {
            result: self.track.completion.borrow().clone(),
            runtime: self.scheduler.runtime.clone(),
        }
    }
    pub fn animate_keyframes(
        &self,
        frames: Keyframes<f32>,
        duration: Duration,
        playback: Playback,
    ) -> Animation {
        assert!(
            !duration.is_zero() || playback.repeat != Repeat::Forever,
            "a repeating curve needs positive duration"
        );
        let result = self.scheduler.runtime.signal(None);
        if !self.track.alive.get() {
            result.set(Some(Completion::Cancelled));
            return Animation {
                result,
                runtime: self.scheduler.runtime.clone(),
            };
        }
        self.scheduler.runtime.batch(|| {
            let previous = self.track.completion.replace(result.clone());
            if previous.with_untracked(|s| s.is_none()) {
                previous.set(Some(Completion::Cancelled));
            }
            let from = frames.sample(0.);
            let target = frames.sample(playback.end());
            *self.track.run.borrow_mut() = Some(Run {
                from,
                target,
                transition: Transition::tween(duration, Easing::Linear).delay(playback.delay),
                oscillator: None,
                elapsed: Duration::ZERO,
                last: self.track.active().then(|| {
                    self.scheduler
                        .frames
                        .last()
                        .map_or_else(Instant::now, |f| f.time.max(Instant::now()))
                }),
                keyframes: Some(frames),
                playback,
            });
            self.track.velocity.set(0.);
            self.track.value.set(from);
            if self.scheduler.runner.is_none()
                || self.track.reduced()
                || (duration.is_zero() && playback.delay.is_zero())
            {
                self.track.finish(Completion::Finished);
                self.scheduler.changed();
            } else {
                self.scheduler.enqueue(&self.track);
            }
        });
        Animation {
            result,
            runtime: self.scheduler.runtime.clone(),
        }
    }
    pub fn is_running(&self) -> bool {
        self.track.alive.get() && self.track.run.borrow().is_some()
    }
    pub fn is_paused(&self) -> bool {
        !self.track.active()
    }
    pub fn pause(&self) {
        if !self.track.paused.get() {
            if self.track.active() {
                self.track.accrue_active_time();
            }
            self.track.paused.set(true);
            self.scheduler.changed();
        }
    }
    pub fn resume(&self) {
        if self.track.paused.replace(false) {
            if let Some(run) = self.track.run.borrow_mut().as_mut() {
                run.last = None;
            }
            self.scheduler.changed();
        }
    }
    /// Sample immediately without changing pause state or completing the run.
    pub fn seek(&self, position: Duration) {
        if !self.track.alive.get() {
            return;
        }
        let sampled = {
            let mut current = self.track.run.borrow_mut();
            current.as_mut().map(|run| {
                run.elapsed = run.transition.delay.saturating_add(position);
                run.last = None;
                let (value, velocity, _) = run.sample();
                (value, velocity)
            })
        };
        if let Some((value, velocity)) = sampled {
            self.scheduler.runtime.batch(|| {
                self.track.velocity.set(velocity);
                self.track.value.set(value);
                self.scheduler.changed();
            });
        }
    }
    pub fn set_rate(&self, rate: f64) {
        assert!(
            rate.is_finite() && (0.01..=100.).contains(&rate),
            "playback rate must be in 0.01..=100"
        );
        if self.track.active() {
            self.track.accrue_active_time();
        }
        self.track.rate.set(rate);
        self.scheduler.changed();
    }
    pub fn label(&self, label: &str) {
        *self.track.label.borrow_mut() = label.chars().take(80).collect();
    }
    /// Direct manipulation can preserve measured velocity for spring release.
    pub fn set_with_velocity(&self, value: f32, velocity: f32) {
        assert!(velocity.is_finite(), "non-finite motion velocity");
        self.scheduler.runtime.batch(|| {
            self.set(value);
            if self.track.alive.get() {
                self.track.velocity.set(velocity);
            }
        });
    }
    pub fn set(&self, value: f32) {
        assert!(value.is_finite(), "non-finite motion value");
        if !self.track.alive.get() {
            return;
        }
        self.scheduler.runtime.batch(|| {
            self.track.finish(Completion::Cancelled);
            self.track.value.set(value);
            self.scheduler.changed();
        });
    }
    pub fn stop(&self) {
        self.scheduler
            .runtime
            .batch(|| self.track.finish(Completion::Cancelled));
        self.scheduler.changed();
    }
}
impl Context {
    pub fn motion_value(&mut self, initial: f32) -> MotionValue {
        MotionValue::new(self, initial)
    }
}

/// A particular run, including its delay. Superseded runs resolve Cancelled.
/// Dropping this handle does not stop the animation.
#[derive(Clone)]
pub struct Animation {
    result: Signal<Option<Completion>>,
    runtime: Runtime,
}
impl Animation {
    pub async fn finished(self) -> Completion {
        let wake = Rc::new(RefCell::new(None::<Waker>));
        let notify = wake.clone();
        let status = self.result.clone();
        let _effect = self.runtime.effect(move || {
            status.get();
            let wake = notify.borrow_mut().take();
            if let Some(wake) = wake {
                wake.wake();
            }
        });
        std::future::poll_fn(|cx| {
            if let Some(result) = self.result.with_untracked(|s| *s) {
                return Poll::Ready(result);
            }
            *wake.borrow_mut() = Some(cx.waker().clone());
            Poll::Pending
        })
        .await
    }
}

/// Retain a conditional view until its exit finishes. Reopening invalidates an
/// earlier exit and retains the current progress. Bind `mounted()` with switch.
#[derive(Clone)]
pub struct Presence {
    pub progress: MotionValue,
    mounted: Signal<bool>,
    present: Rc<Cell<bool>>,
    revision: Rc<Cell<u64>>,
    tasks: Option<crate::compose::Tasks>,
}
impl Presence {
    pub fn new(cx: &mut Context, present: bool) -> Self {
        Self {
            progress: cx.motion_value(if present { 1.0 } else { 0.0 }),
            mounted: cx.state(present),
            present: Rc::new(Cell::new(present)),
            revision: Rc::new(Cell::new(0)),
            tasks: cx.try_service::<TaskRunner>().map(|_| cx.tasks()),
        }
    }
    pub fn mounted(&self) -> Signal<bool> {
        self.mounted.clone()
    }
    pub fn set_present(&self, present: bool, transition: Transition) {
        self.set_present_with(present, transition, AnimationGroup::default());
    }
    /// Keep the exit mounted until both presence progress and additional
    /// property/timeline runs finish. A cancelled exit never removes the view.
    pub fn set_present_with(
        &self,
        present: bool,
        transition: Transition,
        mut exits: AnimationGroup,
    ) {
        assert!(
            exits.0.len() < tracks::MAX_TIMELINE_TRACKS,
            "presence exit exceeds 128 tracks"
        );
        assert!(
            exits
                .0
                .iter()
                .all(|run| run.runtime.same(&self.progress.scheduler.runtime)),
            "presence exit spans reactive runtimes"
        );
        if !self.progress.track.alive.get() || self.present.replace(present) == present {
            return;
        }
        let revision = self.revision.get().wrapping_add(1);
        self.revision.set(revision);
        if present {
            self.mounted.set(true);
        }
        let animation = self
            .progress
            .animate_to(if present { 1.0 } else { 0.0 }, transition);
        exits.0.push(animation);
        assert!(
            exits.0.len() <= tracks::MAX_TIMELINE_TRACKS,
            "presence exit exceeds 128 tracks"
        );
        if !present {
            let this = self.clone();
            if let Some(tasks) = &self.tasks {
                tasks.spawn(async move {
                    if exits.finished().await == Completion::Finished
                        && !this.present.get()
                        && this.revision.get() == revision
                    {
                        this.mounted.set(false);
                    }
                });
            } else if exits.completion() == Some(Completion::Finished) {
                self.mounted.set(false);
            }
        }
    }
}

#[cfg(test)]
#[path = "motion_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "motion_extended_tests.rs"]
mod extended_tests;
