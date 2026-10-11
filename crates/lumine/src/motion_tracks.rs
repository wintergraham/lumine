//! Compiled property curves and one owned transport for coordinated playback.
use super::*;
use crate::scene::Color;
use std::{fmt, num::NonZeroU32};

pub const MAX_KEYFRAMES: usize = 1024;
pub const MAX_TIMELINE_TRACKS: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MotionError(pub &'static str);
impl fmt::Display for MotionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for MotionError {}

/// Copyable property values avoid allocating during sampling.
pub trait Interpolate: Copy + PartialEq + 'static {
    fn interpolate(self, other: Self, progress: f32) -> Self;
    fn finite(self) -> bool;
}
impl Interpolate for f32 {
    fn interpolate(self, other: Self, progress: f32) -> Self {
        finite_scalar(self as f64 + (other as f64 - self as f64) * progress as f64)
    }
    fn finite(self) -> bool {
        self.is_finite()
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}
impl Vec2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}
impl Interpolate for Vec2 {
    fn interpolate(self, other: Self, t: f32) -> Self {
        Self::new(
            self.x.interpolate(other.x, t),
            self.y.interpolate(other.y, t),
        )
    }
    fn finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// Linear, premultiplied RGBA. Decode sRGB once when constructing endpoints.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MotionColor([f32; 4]);
impl MotionColor {
    pub fn from_color(Color(r, g, b, a): Color) -> Self {
        let alpha = a as f32 / 255.;
        let linear = |channel: u8| {
            let c = channel as f32 / 255.;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        Self([
            linear(r) * alpha,
            linear(g) * alpha,
            linear(b) * alpha,
            alpha,
        ])
    }
    pub fn premultiplied(self) -> [f32; 4] {
        self.0
    }
    pub fn color(self) -> Color {
        let a = self.0[3].clamp(0., 1.);
        let srgb = |c: f32| {
            let c = if a > 0. { (c / a).clamp(0., 1.) } else { 0. };
            let c = if c <= 0.0031308 {
                c * 12.92
            } else {
                1.055 * c.powf(1. / 2.4) - 0.055
            };
            (c * 255.).round() as u8
        };
        Color(
            srgb(self.0[0]),
            srgb(self.0[1]),
            srgb(self.0[2]),
            (a * 255.).round() as u8,
        )
    }
}
impl Interpolate for MotionColor {
    fn interpolate(self, other: Self, t: f32) -> Self {
        Self(std::array::from_fn(|i| {
            self.0[i].interpolate(other.0[i], t)
        }))
    }
    fn finite(self) -> bool {
        self.0.iter().all(|v| v.is_finite())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframe<T> {
    pub offset: f32,
    pub value: T,
    pub easing: Easing,
}
impl<T> Keyframe<T> {
    pub fn new(offset: f32, value: T) -> Self {
        Self {
            offset,
            value,
            easing: Easing::Linear,
        }
    }
    pub fn easing(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }
}
#[derive(Clone)]
struct CompiledKeyframe<T> {
    frame: Keyframe<T>,
    inverse_span: f64,
}
#[derive(Clone)]
pub struct Keyframes<T>(Rc<[CompiledKeyframe<T>]>);
impl<T: Interpolate> Keyframes<T> {
    pub fn new(frames: impl IntoIterator<Item = Keyframe<T>>) -> Result<Self, MotionError> {
        let frames: Vec<_> = frames.into_iter().take(MAX_KEYFRAMES + 1).collect();
        if !(2..=MAX_KEYFRAMES).contains(&frames.len()) {
            return Err(MotionError("keyframes require 2..=1024 entries"));
        }
        if frames.first().unwrap().offset != 0.
            || frames.last().unwrap().offset != 1.
            || frames
                .iter()
                .any(|f| !f.offset.is_finite() || !f.value.finite())
            || frames.windows(2).any(|f| f[0].offset >= f[1].offset)
        {
            return Err(MotionError(
                "keyframes need finite values and strictly increasing offsets from 0 to 1",
            ));
        }
        let compiled = frames
            .iter()
            .enumerate()
            .map(|(i, frame)| CompiledKeyframe {
                frame: *frame,
                inverse_span: frames
                    .get(i + 1)
                    .map_or(0., |next| 1. / (next.offset as f64 - frame.offset as f64)),
            })
            .collect::<Vec<_>>();
        Ok(Self(compiled.into()))
    }
    pub fn between(from: T, to: T, easing: Easing) -> Self {
        Self::new([
            Keyframe::new(0., from).easing(easing),
            Keyframe::new(1., to),
        ])
        .expect("finite keyframe endpoints")
    }
    pub fn sample(&self, progress: f32) -> T {
        assert!(progress.is_finite(), "non-finite keyframe time");
        if progress <= 0. {
            return self.0[0].frame.value;
        }
        if progress >= 1. {
            return self.0.last().unwrap().frame.value;
        }
        let end = self
            .0
            .partition_point(|frame| frame.frame.offset <= progress);
        let segment = &self.0[end - 1];
        let a = segment.frame;
        let b = self.0[end].frame;
        let local = ((progress as f64 - a.offset as f64) * segment.inverse_span) as f32;
        a.value.interpolate(b.value, a.easing.sample(local))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Count(NonZeroU32),
    Forever,
}
impl Default for Repeat {
    fn default() -> Self {
        Self::Count(NonZeroU32::new(1).unwrap())
    }
}
impl Repeat {
    pub fn count(cycles: u32) -> Result<Self, MotionError> {
        NonZeroU32::new(cycles)
            .map(Self::Count)
            .ok_or(MotionError("repeat count must be positive"))
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Playback {
    pub repeat: Repeat,
    pub alternate: bool,
    pub delay: Duration,
}
impl Playback {
    pub(crate) fn end(self) -> f32 {
        if self.alternate && matches!(self.repeat, Repeat::Count(n) if n.get().is_multiple_of(2)) {
            0.
        } else {
            1.
        }
    }
    pub(crate) fn phase(self, elapsed: Duration, duration: Duration) -> (f32, bool) {
        if duration.is_zero() {
            return (self.end(), true);
        }
        let cycles = elapsed.as_secs_f64() / duration.as_secs_f64();
        if matches!(self.repeat, Repeat::Count(n) if cycles >= n.get() as f64) {
            return (self.end(), true);
        }
        let mut phase = cycles.fract() as f32;
        if self.alternate && (cycles.floor() as u64) % 2 == 1 {
            phase = 1. - phase;
        }
        (phase, false)
    }
}

struct Clip {
    sample: Box<dyn Fn(f64)>,
}
struct TimelineInner {
    transport: MotionValue,
    duration: Cell<Duration>,
    clips: RefCell<Vec<Clip>>,
    revision: Signal<u64>,
    started: Cell<bool>,
    playback: Cell<Playback>,
}
/// Bounded parallel/sequenced property curves sampled by one motion track.
/// Register clips before playback. Outputs are owned read signals, not competing
/// animations of existing MotionValues. Equal start times create parallel clips.
#[derive(Clone)]
pub struct Timeline(Rc<TimelineInner>);
impl Timeline {
    pub fn new(cx: &mut Context) -> Self {
        let inner = Rc::new(TimelineInner {
            transport: cx.motion_value(0.),
            duration: Cell::new(Duration::ZERO),
            clips: RefCell::default(),
            revision: cx.state(0),
            started: Cell::new(false),
            playback: Cell::new(Playback::default()),
        });
        inner.transport.label("timeline");
        let weak = Rc::downgrade(&inner);
        let runtime = cx.runtime();
        let batch = runtime.clone();
        let effect = runtime.effect(move || {
            let Some(inner) = weak.upgrade() else { return };
            inner.revision.get();
            let elapsed = inner.transport.get() as f64 * inner.duration.get().as_secs_f64();
            batch.batch(|| {
                for clip in inner.clips.borrow().iter() {
                    (clip.sample)(elapsed);
                }
            });
        });
        cx.retain(effect);
        // Outputs may outlive the local builder handle. Keep their sampler with
        // the component, just like the transport's motion ownership lease.
        cx.retain(inner.clone());
        Self(inner)
    }
    pub fn track<T: Interpolate>(
        &self,
        at: Duration,
        duration: Duration,
        frames: Keyframes<T>,
    ) -> Result<Signal<T>, MotionError> {
        if self.0.started.get() {
            return Err(MotionError("register timeline tracks before playback"));
        }
        if self.0.clips.borrow().len() >= MAX_TIMELINE_TRACKS {
            return Err(MotionError("timeline exceeds 128 tracks"));
        }
        let end = at
            .checked_add(duration)
            .ok_or(MotionError("timeline duration overflow"))?;
        let output = self.0.transport.scheduler.runtime.signal(frames.sample(0.));
        let target = output.clone();
        let at = at.as_secs_f64();
        let seconds = duration.as_secs_f64();
        let inverse_duration = if seconds == 0. { 0. } else { 1. / seconds };
        self.0.clips.borrow_mut().push(Clip {
            sample: Box::new(move |elapsed| {
                let t = if seconds == 0. {
                    if elapsed >= at { 1. } else { 0. }
                } else {
                    ((elapsed - at) * inverse_duration).clamp(0., 1.) as f32
                };
                target.set(frames.sample(t));
            }),
        });
        self.0.duration.set(self.0.duration.get().max(end));
        self.0.revision.update(|r| *r = r.wrapping_add(1));
        Ok(output)
    }
    pub fn then<T: Interpolate>(
        &self,
        duration: Duration,
        frames: Keyframes<T>,
    ) -> Result<Signal<T>, MotionError> {
        self.track(self.duration(), duration, frames)
    }
    pub fn duration(&self) -> Duration {
        self.0.duration.get()
    }
    pub fn position(&self) -> Duration {
        scale_duration(self.duration(), self.0.transport.get().clamp(0., 1.) as f64)
    }
    pub fn progress(&self) -> Signal<f32> {
        self.0.transport.signal()
    }
    pub fn playback(&self, playback: Playback) -> Result<(), MotionError> {
        if self.0.transport.is_running() {
            return Err(MotionError("stop timeline before changing playback"));
        }
        if self.duration().is_zero() && playback.repeat == Repeat::Forever {
            return Err(MotionError("a repeating timeline needs positive duration"));
        }
        self.0.playback.set(playback);
        Ok(())
    }
    fn begin(&self) -> Animation {
        self.0.started.set(true);
        self.0.transport.animate_keyframes(
            Keyframes::between(0., 1., Easing::Linear),
            self.duration(),
            self.0.playback.get(),
        )
    }
    pub fn play(&self) -> Animation {
        self.0.transport.resume();
        if self.0.transport.is_running() {
            self.0.transport.animation()
        } else {
            self.begin()
        }
    }
    pub fn restart(&self) -> Animation {
        self.stop();
        self.0.transport.resume();
        self.begin()
    }
    pub fn pause(&self) {
        self.0.transport.pause();
    }
    pub fn stop(&self) {
        self.0.transport.stop();
    }
    /// Seek within one cycle. A stopped timeline starts a paused run; seeking a
    /// playing timeline preserves playback. Endpoint completion occurs on play.
    pub fn seek(&self, position: Duration) {
        self.0.transport.scheduler.runtime.batch(|| {
            if !self.0.transport.is_running() {
                self.0.transport.pause();
                self.begin();
            }
            if self.0.transport.is_running() {
                self.0.transport.seek(position.min(self.duration()));
            } else {
                self.0.transport.set(if self.duration().is_zero() {
                    1.
                } else {
                    (position.min(self.duration()).as_secs_f64() / self.duration().as_secs_f64())
                        as f32
                });
            }
        });
    }
    pub fn set_rate(&self, rate: f64) {
        self.0.transport.set_rate(rate);
    }
    pub fn is_running(&self) -> bool {
        self.0.transport.is_running() && !self.0.transport.is_paused()
    }
    /// A finite directional transition preserves the current pose on reversal.
    /// Repeat settings apply to `play`, not to this explicit target transition.
    pub fn animate_to(&self, progress: f32) -> Animation {
        assert!(
            progress.is_finite() && (0.0..=1.0).contains(&progress),
            "timeline target must be in 0..=1"
        );
        self.0.started.set(true);
        self.0.transport.resume();
        let from = self
            .0
            .transport
            .track
            .value
            .with_untracked(|v| *v)
            .clamp(0., 1.);
        self.0.transport.animate_to(
            progress,
            Transition::tween(
                scale_duration(self.duration(), (progress as f64 - from as f64).abs()),
                Easing::Linear,
            ),
        )
    }
}

impl Context {
    /// A component-owned pure derived value, updated in reactive batches.
    pub fn derive<T: PartialEq + 'static>(
        &mut self,
        mut compute: impl FnMut() -> T + 'static,
    ) -> Signal<T> {
        let runtime = self.runtime();
        let output = runtime.signal(runtime.untracked(&mut compute));
        let target = output.clone();
        let effect = runtime.effect(move || {
            target.set(compute());
        });
        self.retain(effect);
        output
    }
}

/// Wait for all tracks; any cancellation cancels the group immediately.
#[derive(Clone, Default)]
pub struct AnimationGroup(pub(crate) Vec<Animation>);
impl AnimationGroup {
    pub fn new(animations: impl IntoIterator<Item = Animation>) -> Self {
        let animations: Vec<_> = animations
            .into_iter()
            .take(MAX_TIMELINE_TRACKS + 1)
            .collect();
        assert!(
            animations.len() <= MAX_TIMELINE_TRACKS,
            "animation group exceeds 128 tracks"
        );
        if let Some(first) = animations.first() {
            assert!(
                animations.iter().all(|a| a.runtime.same(&first.runtime)),
                "animation group spans reactive runtimes"
            );
        }
        Self(animations)
    }
    pub fn completion(&self) -> Option<Completion> {
        let mut all = true;
        let mut cancelled = false;
        for animation in &self.0 {
            let state = animation.result.with_untracked(|s| *s);
            all &= state == Some(Completion::Finished);
            cancelled |= state == Some(Completion::Cancelled);
        }
        if cancelled {
            Some(Completion::Cancelled)
        } else if all {
            Some(Completion::Finished)
        } else {
            None
        }
    }
    pub async fn finished(self) -> Completion {
        let Some(first) = self.0.first() else {
            return Completion::Finished;
        };
        let runtime = first.runtime.clone();
        let wake = Rc::new(RefCell::new(None::<Waker>));
        let notify = wake.clone();
        let statuses: Vec<_> = self.0.iter().map(|a| a.result.clone()).collect();
        let output = runtime.signal(None);
        let result = output.clone();
        let _effect = runtime.effect(move || {
            let mut all = true;
            let mut cancelled = false;
            for status in &statuses {
                let state = status.get();
                all &= state == Some(Completion::Finished);
                cancelled |= state == Some(Completion::Cancelled);
            }
            let done = if cancelled {
                Some(Completion::Cancelled)
            } else if all {
                Some(Completion::Finished)
            } else {
                None
            };
            result.set(done);
            if done.is_some() {
                let wake = notify.borrow_mut().take();
                if let Some(wake) = wake {
                    wake.wake();
                }
            }
        });
        std::future::poll_fn(|cx| {
            if let Some(result) = output.with_untracked(|s| *s) {
                Poll::Ready(result)
            } else {
                *wake.borrow_mut() = Some(cx.waker().clone());
                Poll::Pending
            }
        })
        .await
    }
}

#[derive(Clone)]
pub struct MotionPoint {
    pub x: MotionValue,
    pub y: MotionValue,
    value: Signal<Vec2>,
    runtime: Runtime,
}
impl MotionPoint {
    pub fn new(cx: &mut Context, initial: Vec2) -> Self {
        assert!(initial.finite(), "non-finite motion point");
        let x = cx.motion_value(initial.x);
        let y = cx.motion_value(initial.y);
        let (a, b) = (x.signal(), y.signal());
        let value = cx.derive(move || Vec2::new(a.get(), b.get()));
        Self {
            x,
            y,
            value,
            runtime: cx.runtime(),
        }
    }
    pub fn signal(&self) -> Signal<Vec2> {
        self.value.clone()
    }
    pub fn get(&self) -> Vec2 {
        self.value.get()
    }
    pub fn set(&self, value: Vec2) {
        assert!(value.finite(), "non-finite motion point");
        self.runtime.batch(|| {
            self.x.set(value.x);
            self.y.set(value.y);
        });
    }
    pub fn animate_to(&self, value: Vec2, transition: Transition) -> AnimationGroup {
        assert!(value.finite(), "non-finite motion point");
        transition.validate();
        self.runtime.batch(|| {
            AnimationGroup::new([
                self.x.animate_to(value.x, transition),
                self.y.animate_to(value.y, transition),
            ])
        })
    }
}
