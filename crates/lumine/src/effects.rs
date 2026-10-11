//! Shared, visibility-aware clocks for continuous visual effects. Finite
//! transitions use [`crate::motion`]; these clocks advance only while selected.
use crate::{
    compose::{Context, TaskRunner, TaskToken, View},
    frame::{Frame, FrameClock, NextFrame},
    motion::MotionPolicy,
    reactive::{Effect, Runtime, Signal},
};
use std::{
    cell::{Cell, RefCell},
    future::Future,
    pin::Pin,
    rc::{Rc, Weak},
    task::{Context as PollContext, Poll, Waker},
    time::{Duration, Instant},
};

/// Lower priorities are selected first; equal priorities keep registration
/// order. Rates round down to whole display refreshes and the window frame cap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectOptions {
    pub hz: f64,
    pub priority: u8,
}
impl Default for EffectOptions {
    fn default() -> Self {
        Self {
            hz: 30.0,
            priority: 0,
        }
    }
}
impl EffectOptions {
    fn validate(self) {
        assert!(
            self.hz.is_finite() && self.hz > 0.0,
            "effect rate must be finite and positive"
        );
    }
}

/// Active time excludes hidden, disabled, reduced-motion and budgeted-out time.
/// `tick` counts samples; elapsed time includes display frames missed by the UI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EffectFrame {
    pub tick: u64,
    pub elapsed: Duration,
    pub delta: Duration,
}

struct Track {
    options: Signal<EffectOptions>,
    enabled: Signal<bool>,
    visible: Signal<bool>,
    selected: Signal<bool>,
    frame: Signal<EffectFrame>,
    policy: Option<MotionPolicy>,
    elapsed: Cell<Duration>,
    last: Cell<Option<Instant>>,
    last_refresh: Cell<Option<u64>>,
    alive: Cell<bool>,
    wanted: Cell<bool>,
}
impl Track {
    fn pause(&self) {
        if let Some(last) = self.last.take() {
            self.elapsed
                .set(self.elapsed.get() + Instant::now().saturating_duration_since(last));
        }
        self.last_refresh.set(None);
    }

    fn step(&self, frame: Frame, divisor: u64) {
        if self
            .last_refresh
            .get()
            .is_some_and(|last| frame.index >= last && frame.index < last.saturating_add(divisor))
        {
            return;
        }
        let period = frame
            .interval
            .saturating_mul(divisor.min(u32::MAX as u64) as u32);
        let delta = self
            .last
            .get()
            .map_or(period, |last| frame.time.saturating_duration_since(last));
        let elapsed = self.elapsed.get() + delta;
        self.elapsed.set(elapsed);
        self.last.set(Some(frame.time));
        self.last_refresh.set(Some(frame.index));
        let tick = self
            .frame
            .with_untracked(|value| value.tick)
            .wrapping_add(1);
        self.frame.set(EffectFrame {
            tick,
            elapsed,
            delta,
        });
    }
}

pub(crate) struct Scheduler {
    runtime: Runtime,
    frames: FrameClock,
    runner: Option<TaskRunner>,
    tracks: RefCell<Vec<Weak<Track>>>,
    selected: RefCell<Vec<Rc<Track>>>,
    budget: Signal<usize>,
    revision: Signal<u64>,
    generation: Cell<u64>,
    task: RefCell<Option<TaskToken>>,
    wake: RefCell<Option<Waker>>,
    observer: RefCell<Option<Effect>>,
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
            budget: runtime.signal(usize::MAX),
            revision: runtime.signal(0),
            runtime,
            frames,
            runner,
            tracks: RefCell::default(),
            selected: RefCell::default(),
            generation: Cell::new(0),
            task: RefCell::default(),
            wake: RefCell::default(),
            observer: RefCell::default(),
        });
        let weak = Rc::downgrade(&this);
        let observer = this.runtime.effect(move || {
            let Some(this) = weak.upgrade() else {
                return;
            };
            this.revision.get();
            let budget = this.budget.get();
            let tracks: Vec<_> = this
                .tracks
                .borrow()
                .iter()
                .filter_map(Weak::upgrade)
                .filter(|track| track.alive.get())
                .collect();
            this.tracks
                .borrow_mut()
                .retain(|track| track.upgrade().is_some_and(|track| track.alive.get()));
            let mut candidates = Vec::new();
            for (index, track) in tracks.iter().enumerate() {
                track.wanted.set(false);
                let options = track.options.get();
                let enabled = track.enabled.get();
                let visible = track.visible.get();
                let (active, reduced) = track.policy.as_ref().map_or((true, false), |policy| {
                    (policy.active.get(), policy.reduced.get())
                });
                if enabled && visible && active && !reduced && this.runner.is_some() {
                    candidates.push((options.priority, index, track.clone()));
                }
            }
            candidates.sort_unstable_by_key(|(priority, index, _)| (*priority, *index));
            let chosen: Vec<_> = candidates
                .into_iter()
                .take(budget)
                .map(|(_, _, track)| {
                    track.wanted.set(true);
                    track
                })
                .collect();
            this.runtime.batch(|| {
                for track in tracks {
                    let selected = track.wanted.get();
                    if track.selected.with_untracked(|value| *value) && !selected {
                        track.pause();
                    }
                    track.selected.set(selected);
                }
                let old = this.selected.replace(chosen);
                drop(old);
                this.update_task();
            });
            this.notify();
        });
        *this.observer.borrow_mut() = Some(observer);
        this
    }

    fn notify(&self) {
        let wake = self.wake.borrow_mut().take();
        if let Some(wake) = wake {
            wake.wake();
        }
    }
    fn changed(&self) {
        let revision = self.revision.with_untracked(|value| *value).wrapping_add(1);
        self.revision.set(revision);
        self.notify();
    }
    fn update_task(self: &Rc<Self>) {
        if self.selected.borrow().is_empty() {
            let old = self.task.borrow_mut().take();
            if old.is_some() {
                self.generation.set(self.generation.get().wrapping_add(1));
            }
            drop(old);
        } else if self.task.borrow().is_none()
            && let Some(runner) = &self.runner
        {
            let generation = self.generation.get().wrapping_add(1);
            self.generation.set(generation);
            let token = runner.spawn_owned(Driver {
                scheduler: Rc::downgrade(self),
                generation,
                frame: None,
                rate: None,
                last_index: None,
            });
            *self.task.borrow_mut() = Some(token);
        }
    }
    fn divisor(&self, refresh: f64, hz: f64) -> u64 {
        let base = self
            .frames
            .max_rate()
            .map_or(1, |cap| crate::frame::divisor(refresh, cap));
        crate::frame::divisor(refresh, hz).div_ceil(base) * base
    }
    fn rate(&self, last_index: Option<u64>) -> Option<f64> {
        let selected = self.selected.borrow();
        if selected.is_empty() {
            return None;
        }
        if let Some(frame) = self.frames.last() {
            let refresh = frame.refresh_rate();
            let index = last_index.unwrap_or(frame.index);
            let next = selected
                .iter()
                .map(|track| {
                    let divisor =
                        self.divisor(refresh, track.options.with_untracked(|value| value.hz));
                    track
                        .last_refresh
                        .get()
                        .map_or(index.saturating_add(1), |last| last.saturating_add(divisor))
                })
                .min()
                .unwrap();
            // Wake at the next effect deadline, not at the GCD of all rates:
            // e.g. 12/8 Hz clocks must not turn into a 60 Hz task on 60 Hz panels.
            Some(refresh / next.saturating_sub(index).max(1) as f64)
        } else {
            selected
                .iter()
                .map(|track| track.options.with_untracked(|value| value.hz))
                .reduce(f64::max)
        }
    }
    fn step(&self, frame: Frame) {
        self.runtime.batch(|| {
            for track in self.selected.borrow().iter() {
                if track.alive.get() {
                    let divisor = self.divisor(
                        frame.refresh_rate(),
                        track.options.with_untracked(|value| value.hz),
                    );
                    track.step(frame, divisor);
                }
            }
        });
    }
}

struct Driver {
    scheduler: Weak<Scheduler>,
    generation: u64,
    frame: Option<NextFrame>,
    rate: Option<f64>,
    last_index: Option<u64>,
}
impl Future for Driver {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut PollContext<'_>) -> Poll<()> {
        let Some(scheduler) = self.scheduler.upgrade() else {
            return Poll::Ready(());
        };
        if scheduler.generation.get() != self.generation {
            return Poll::Ready(());
        }
        let Some(rate) = scheduler.rate(self.last_index) else {
            return Poll::Ready(());
        };
        *scheduler.wake.borrow_mut() = Some(cx.waker().clone());
        // Consume a delivered sample before adapting to a changed display rate.
        // Dropping a ready waiter here would lose the first refresh sample.
        if let Some(frame) = self.frame.as_mut()
            && let Poll::Ready(frame) = Pin::new(frame).poll(cx)
        {
            self.frame = None;
            self.last_index = Some(frame.index);
            scheduler.step(frame);
            return self.poll(cx);
        }
        if self.rate != Some(rate) {
            self.frame = None;
            self.rate = Some(rate);
        }
        if self.frame.is_none() {
            let mut frame = scheduler.frames.next().max_rate(rate);
            if let Some(index) = self.last_index {
                frame = frame.after_index(index);
            }
            self.frame = Some(frame);
        }
        if let Poll::Ready(frame) = Pin::new(self.frame.as_mut().unwrap()).poll(cx) {
            self.frame = None;
            self.last_index = Some(frame.index);
            scheduler.step(frame);
            return self.poll(cx);
        }
        Poll::Pending
    }
}

/// The shared window budget for continuous effects, independent of finite
/// motion values. Clones and clocks on the same runtime/frame clock share it.
#[derive(Clone)]
pub struct EffectScheduler(Rc<Scheduler>);
impl EffectScheduler {
    pub fn new(cx: &Context) -> Self {
        let runtime = cx.runtime();
        let scheduler = match (
            cx.try_service::<FrameClock>(),
            cx.try_service::<TaskRunner>(),
        ) {
            (Some(frames), Some(runner)) => frames.effect_scheduler(runtime, (*runner).clone()),
            _ => Scheduler::new(runtime, FrameClock::new(), None),
        };
        Self(scheduler)
    }
    /// Limit concurrently ticking effects. Zero pauses all continuous effects.
    pub fn set_budget(&self, max_active: usize) {
        self.0.budget.set(max_active);
    }
    pub fn budget(&self) -> usize {
        self.0.budget.with_untracked(|value| *value)
    }
    pub fn clock(&self, cx: &mut Context, options: EffectOptions) -> EffectClock {
        self.clock_with_policy(
            cx,
            options,
            cx.try_service::<MotionPolicy>()
                .map(|policy| (*policy).clone()),
        )
    }
    pub fn clock_with_policy(
        &self,
        cx: &mut Context,
        options: EffectOptions,
        policy: Option<MotionPolicy>,
    ) -> EffectClock {
        options.validate();
        assert!(
            self.0.runtime.same(&cx.runtime()),
            "effect scheduler belongs to another reactive runtime"
        );
        let runtime = &self.0.runtime;
        let track = Rc::new(Track {
            options: runtime.signal(options),
            enabled: runtime.signal(true),
            visible: runtime.signal(true),
            selected: runtime.signal(false),
            frame: runtime.signal(EffectFrame::default()),
            policy,
            elapsed: Cell::new(Duration::ZERO),
            last: Cell::new(None),
            last_refresh: Cell::new(None),
            alive: Cell::new(true),
            wanted: Cell::new(false),
        });
        self.0.tracks.borrow_mut().push(Rc::downgrade(&track));
        cx.retain(Owner {
            track: track.clone(),
            scheduler: self.0.clone(),
        });
        self.0.changed();
        EffectClock {
            track,
            scheduler: self.0.clone(),
        }
    }
}

struct Owner {
    track: Rc<Track>,
    scheduler: Rc<Scheduler>,
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.track.alive.set(false);
        self.track.pause();
        self.scheduler.runtime.batch(|| {
            self.track.selected.set(false);
            self.scheduler.changed();
        });
    }
}

/// A component-owned continuous clock. External clones cannot resume it after
/// disposal. Without a frame clock and executor, it stays at its static sample.
#[derive(Clone)]
pub struct EffectClock {
    track: Rc<Track>,
    scheduler: Rc<Scheduler>,
}
impl EffectClock {
    pub fn new(cx: &mut Context, options: EffectOptions) -> Self {
        EffectScheduler::new(cx).clock(cx, options)
    }
    pub fn with_policy(
        cx: &mut Context,
        options: EffectOptions,
        policy: Option<MotionPolicy>,
    ) -> Self {
        EffectScheduler::new(cx).clock_with_policy(cx, options, policy)
    }
    pub fn frame(&self) -> Signal<EffectFrame> {
        self.track.frame.clone()
    }
    pub fn enabled(&self) -> Signal<bool> {
        self.track.enabled.clone()
    }
    pub fn visible(&self) -> Signal<bool> {
        self.track.visible.clone()
    }
    pub fn selected(&self) -> Signal<bool> {
        self.track.selected.clone()
    }
    pub fn set_rate(&self, hz: f64) {
        let mut options = self.track.options.with_untracked(|value| *value);
        options.hz = hz;
        options.validate();
        self.track.options.set(options);
    }
    pub fn set_priority(&self, priority: u8) {
        self.track
            .options
            .update(|options| options.priority = priority);
    }
    /// Restart the sampled phase without changing eligibility or priority.
    pub fn reset(&self) {
        if !self.track.alive.get() {
            return;
        }
        self.scheduler.runtime.batch(|| {
            self.track.elapsed.set(Duration::ZERO);
            self.track.last.set(None);
            self.track.last_refresh.set(None);
            self.track.frame.set(EffectFrame::default());
        });
    }
    /// Pause while this view is clipped/offscreen or its native window cannot
    /// present. Visibility is observed only when retained geometry changes.
    pub fn bind(&self, view: View) -> View {
        self.track.visible.set(false);
        view.observe_visibility(self.track.visible.clone())
    }
}
impl Context {
    pub fn effect_clock(&mut self, options: EffectOptions) -> EffectClock {
        EffectClock::new(self, options)
    }
}

#[cfg(test)]
#[path = "effects_tests.rs"]
mod tests;
