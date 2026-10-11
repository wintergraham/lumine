//! Small retained input/state adapters; all release motion uses the shared driver.
use super::*;
use crate::{
    compose::View,
    input::{EventPhase, InputEvent, PointerButton},
};
use std::ops::RangeInclusive;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionAxis {
    X,
    Y,
}
#[derive(Clone, Copy)]
struct Gesture {
    origin: f32,
    value: f32,
    last_pointer: f32,
    last_time: Instant,
}
#[derive(Clone)]
pub struct DragMotion {
    value: MotionValue,
    output: Signal<f32>,
    axis: MotionAxis,
    bounds: RangeInclusive<f32>,
    snaps: Rc<[f32]>,
    spring: Spring,
    gesture: Rc<Cell<Option<Gesture>>>,
}
impl DragMotion {
    pub fn new(
        cx: &mut Context,
        initial: f32,
        axis: MotionAxis,
        bounds: RangeInclusive<f32>,
    ) -> Result<Self, MotionError> {
        if !initial.is_finite()
            || !bounds.start().is_finite()
            || !bounds.end().is_finite()
            || bounds.start() > bounds.end()
            || !bounds.contains(&initial)
        {
            return Err(MotionError(
                "drag requires finite ordered bounds containing its initial value",
            ));
        }
        let value = cx.motion_value(initial);
        let source = value.signal();
        let limits = bounds.clone();
        let output = cx.derive(move || source.get().clamp(*limits.start(), *limits.end()));
        Ok(Self {
            value,
            output,
            axis,
            bounds,
            snaps: Rc::from([]),
            spring: Spring::default(),
            gesture: Rc::default(),
        })
    }
    pub fn signal(&self) -> Signal<f32> {
        self.output.clone()
    }
    pub fn get(&self) -> f32 {
        self.output.get()
    }
    pub fn velocity(&self) -> f32 {
        self.value.velocity()
    }
    pub fn snap_points(
        mut self,
        points: impl IntoIterator<Item = f32>,
    ) -> Result<Self, MotionError> {
        let points: Vec<_> = points.into_iter().take(65).collect();
        if points.len() > 64
            || points
                .iter()
                .any(|p| !p.is_finite() || !self.bounds.contains(p))
        {
            return Err(MotionError(
                "drag snap points must be finite, in bounds, and at most 64",
            ));
        }
        self.snaps = points.into();
        Ok(self)
    }
    pub fn spring(mut self, spring: Spring) -> Self {
        Transition::spring(spring);
        self.spring = spring;
        self
    }
    pub fn begin(&self, pointer: f32, now: Instant) -> bool {
        if !self.value.track.alive.get() || !pointer.is_finite() {
            return false;
        }
        self.value.resume();
        self.value.stop();
        let value = self
            .value
            .track
            .value
            .with_untracked(|v| *v)
            .clamp(*self.bounds.start(), *self.bounds.end());
        self.value.set_with_velocity(value, 0.);
        self.gesture.set(Some(Gesture {
            origin: pointer,
            value,
            last_pointer: pointer,
            last_time: now,
        }));
        true
    }
    pub fn update(&self, pointer: f32, now: Instant) {
        let Some(mut gesture) = self.gesture.get() else {
            return;
        };
        if !pointer.is_finite() || !self.value.track.alive.get() {
            return;
        }
        let next = finite_scalar(gesture.value as f64 + pointer as f64 - gesture.origin as f64)
            .clamp(*self.bounds.start(), *self.bounds.end());
        let previous = self.value.track.value.with_untracked(|v| *v);
        let seconds = now
            .saturating_duration_since(gesture.last_time)
            .as_secs_f64()
            .max(0.001);
        let velocity = finite_scalar((next as f64 - previous as f64) / seconds);
        self.value.set_with_velocity(next, velocity);
        gesture.last_pointer = pointer;
        gesture.last_time = now;
        self.gesture.set(Some(gesture));
    }
    pub fn release(&self, pointer: f32, now: Instant) -> Option<Animation> {
        let gesture = self.gesture.get()?;
        if pointer != gesture.last_pointer {
            self.update(pointer, now);
        }
        let gesture = self.gesture.take()?;
        if !self.value.track.alive.get() {
            return None;
        }
        if now.saturating_duration_since(gesture.last_time) > Duration::from_millis(100) {
            let value = self.value.track.value.with_untracked(|v| *v);
            self.value.set_with_velocity(value, 0.);
        }
        let value = self.value.track.value.with_untracked(|v| *v);
        let projected = (value as f64 + self.value.velocity() as f64 * 0.15)
            .clamp(*self.bounds.start() as f64, *self.bounds.end() as f64);
        let target = self
            .snaps
            .iter()
            .copied()
            .min_by(|a, b| {
                ((*a as f64 - projected).abs()).total_cmp(&(*b as f64 - projected).abs())
            })
            .unwrap_or(projected as f32);
        Some(
            self.value
                .animate_to(target, Transition::spring(self.spring)),
        )
    }
    pub fn cancel(&self) {
        self.gesture.set(None);
        self.value.stop();
    }
    pub fn dragging(&self) -> bool {
        self.gesture.get().is_some() && self.value.track.alive.get()
    }
    pub fn bind(&self, view: View) -> View {
        let drag = self.clone();
        view.on_event(move |event| {
            if event.phase == EventPhase::Capture {
                return;
            }
            let coordinate = |x, y| {
                event
                    .pointer_in_parent(x, y)
                    .map(|(x, y)| if drag.axis == MotionAxis::X { x } else { y })
            };
            match event.event {
                InputEvent::PointerDown {
                    x,
                    y,
                    button: PointerButton::Primary,
                } if !event.default_prevented() && !event.capture_requested() => {
                    if coordinate(x, y).is_some_and(|pointer| drag.begin(pointer, Instant::now())) {
                        event.capture_pointer();
                        event.prevent_default();
                    }
                }
                InputEvent::PointerMove { x, y } if drag.dragging() => {
                    if let Some(pointer) = coordinate(x, y) {
                        drag.update(pointer, Instant::now());
                    } else {
                        drag.cancel();
                        event.release_pointer();
                    }
                    event.prevent_default();
                }
                InputEvent::PointerUp {
                    x,
                    y,
                    button: PointerButton::Primary,
                } if drag.dragging() => {
                    if let Some(pointer) = coordinate(x, y) {
                        drag.release(pointer, Instant::now());
                    } else {
                        drag.cancel();
                    }
                    event.release_pointer();
                    event.prevent_default();
                }
                InputEvent::PointerCancel | InputEvent::Blur | InputEvent::FocusScopeClosed
                    if drag.dragging() =>
                {
                    drag.cancel();
                    event.release_pointer();
                }
                _ => {}
            }
        })
    }
}

#[derive(Clone)]
pub struct ScrollProgress(Signal<f32>);
impl ScrollProgress {
    pub fn new(cx: &Context) -> Self {
        Self(cx.state(0.))
    }
    pub fn signal(&self) -> Signal<f32> {
        self.0.clone()
    }
    pub fn bind(&self, view: View) -> View {
        view.scroll_progress(self.0.clone())
    }
}

/// A bounded named state table over a scalar. Derive the component's properties
/// from its value; ordinary spring interruption/velocity semantics are preserved.
#[derive(Clone)]
pub struct MotionStates<K> {
    pub value: MotionValue,
    current: Signal<K>,
    states: Rc<[(K, f32, Transition)]>,
}
impl<K: Clone + Eq + 'static> MotionStates<K> {
    pub fn new(
        cx: &mut Context,
        initial: K,
        states: impl IntoIterator<Item = (K, f32, Transition)>,
    ) -> Result<Self, MotionError> {
        let states: Vec<_> = states.into_iter().take(129).collect();
        if states.is_empty()
            || states.len() > 128
            || states.iter().any(|(_, v, _)| !v.is_finite())
            || states
                .iter()
                .enumerate()
                .any(|(i, (key, _, _))| states[..i].iter().any(|(other, _, _)| key == other))
        {
            return Err(MotionError(
                "state table needs 1..=128 unique keys and finite values",
            ));
        }
        for (_, _, transition) in &states {
            transition.validate();
        }
        let value = states
            .iter()
            .find(|(k, _, _)| *k == initial)
            .ok_or(MotionError("initial motion state is missing"))?
            .1;
        Ok(Self {
            value: cx.motion_value(value),
            current: cx.state(initial),
            states: states.into(),
        })
    }
    pub fn state(&self) -> Signal<K> {
        self.current.clone()
    }
    pub fn signal(&self) -> Signal<f32> {
        self.value.signal()
    }
    pub fn set(&self, key: K) -> Result<Animation, MotionError> {
        let (_, value, transition) = self
            .states
            .iter()
            .find(|(k, _, _)| *k == key)
            .ok_or(MotionError("unknown motion state"))?;
        if !self.value.track.alive.get() {
            return Ok(self.value.animation());
        }
        if self.current.with_untracked(|k| *k == key) {
            return Ok(self.value.animation());
        }
        Ok(self.value.scheduler.runtime.batch(|| {
            let run = self.value.animate_to(*value, *transition);
            self.current.set(key);
            run
        }))
    }
}

/// Explicit component-owned namespace for up to 128 shared layout IDs.
/// A replacement continues from the previous mount's projected layout rectangle;
/// user scale/rotation remain independent paint styles, not shared-layout tracks.
#[derive(Clone)]
pub struct SharedLayoutScope(Rc<SharedLayouts>);
struct SharedLayouts {
    runtime: Runtime,
    alive: Cell<bool>,
    next: Cell<u64>,
    entries: RefCell<std::collections::HashMap<String, SharedLayoutEntry>>,
}
struct SharedLayoutEntry {
    generation: u64,
    mounted: bool,
    bounds: Option<crate::scene::Rect>,
}
struct SharedLayoutOwner(Rc<SharedLayouts>);
impl Drop for SharedLayoutOwner {
    fn drop(&mut self) {
        self.0.alive.set(false);
        self.0.entries.borrow_mut().clear();
    }
}
impl SharedLayoutScope {
    pub fn new(cx: &mut Context) -> Self {
        let inner = Rc::new(SharedLayouts {
            runtime: cx.runtime(),
            alive: Cell::new(true),
            next: Cell::new(0),
            entries: RefCell::default(),
        });
        cx.retain(SharedLayoutOwner(inner.clone()));
        Self(inner)
    }
    /// Release an unmounted ID's retained snapshot. Active IDs cannot be forgotten.
    pub fn forget(&self, id: &str) -> bool {
        let mut entries = self.0.entries.borrow_mut();
        if entries.get(id).is_some_and(|entry| !entry.mounted) {
            entries.remove(id);
            true
        } else {
            false
        }
    }
    fn register(
        &self,
        cx: &mut Context,
        id: String,
    ) -> (SharedLayoutLease, Option<crate::scene::Rect>) {
        assert!(self.0.alive.get(), "shared layout scope is disposed");
        assert!(
            self.0.runtime.same(&cx.runtime()),
            "shared layout scope spans reactive runtimes"
        );
        let generation = self
            .0
            .next
            .get()
            .checked_add(1)
            .expect("shared layout generation exhausted");
        self.0.next.set(generation);
        let previous = {
            let mut entries = self.0.entries.borrow_mut();
            assert!(
                entries.contains_key(&id) || entries.len() < 128,
                "shared layout scope exceeds 128 IDs; forget unmounted IDs"
            );
            let previous = entries.get(&id).and_then(|entry| entry.bounds);
            entries.insert(
                id.clone(),
                SharedLayoutEntry {
                    generation,
                    mounted: true,
                    bounds: previous,
                },
            );
            previous
        };
        let lease = SharedLayoutLease {
            scope: Rc::downgrade(&self.0),
            id,
            generation,
        };
        cx.retain(SharedLayoutLease {
            scope: lease.scope.clone(),
            id: lease.id.clone(),
            generation,
        });
        (lease, previous)
    }
}
struct SharedLayoutLease {
    scope: Weak<SharedLayouts>,
    id: String,
    generation: u64,
}
impl SharedLayoutLease {
    fn record(&self, bounds: crate::scene::Rect) {
        if let Some(scope) = self.scope.upgrade().filter(|s| s.alive.get()) {
            let mut entries = scope.entries.borrow_mut();
            if let Some(entry) = entries
                .get_mut(&self.id)
                .filter(|e| e.generation == self.generation)
            {
                entry.bounds = Some(bounds);
            }
        }
    }
}
impl Drop for SharedLayoutLease {
    fn drop(&mut self) {
        if let Some(scope) = self.scope.upgrade() {
            let mut entries = scope.entries.borrow_mut();
            if let Some(entry) = entries
                .get_mut(&self.id)
                .filter(|e| e.generation == self.generation)
            {
                entry.mounted = false;
            }
        }
    }
}

pub(crate) fn project_layout(
    ui: &mut crate::widgets::Ui,
    node: crate::scene::NodeId,
    cx: &mut Context,
    transition: Transition,
    id: Option<String>,
) -> Signal<crate::affine::Affine> {
    use crate::{affine::Affine, scene::Rect};
    let position = MotionPoint::new(cx, Vec2::default());
    let size = MotionPoint::new(cx, Vec2::default());
    let (lease, mut seed) = if let Some(id) = id {
        let scope = cx.service::<SharedLayoutScope>();
        let (lease, seed) = scope.register(cx, id);
        (Some(lease), seed)
    } else {
        (None, None)
    };
    let bounds = ui.observe_layout_bounds(node);
    let output = ui.signal(Affine::IDENTITY);
    let (p, s) = (position.clone(), size.clone());
    let layout = bounds.clone();
    let runtime = ui.runtime.clone();
    let mut previous = None;
    ui.bind(node, move || {
        let Some(bounds) = layout.get() else {
            previous = None;
            runtime.batch(|| {
                p.x.stop();
                p.y.stop();
                s.x.stop();
                s.y.stop();
            });
            return;
        };
        if previous == Some(bounds) {
            return;
        }
        let initialized = previous.replace(bounds).is_some();
        runtime.batch(|| {
            if !initialized {
                let start = seed.take().unwrap_or(bounds);
                p.set(Vec2::new(start.x, start.y));
                s.set(Vec2::new(start.width, start.height));
            }
            p.animate_to(Vec2::new(bounds.x, bounds.y), transition);
            s.animate_to(Vec2::new(bounds.width, bounds.height), transition);
        });
    });
    let weak = ui.downgrade();
    let result = output.clone();
    let (p, s) = (position.signal(), size.signal());
    ui.bind(node, move || {
        if weak.upgrade().is_none() {
            return;
        }
        let Some(bounds) = bounds.get() else {
            result.set(Affine::IDENTITY);
            return;
        };
        let (p, s) = (p.get(), s.get());
        let visual = Rect::new(p.x, p.y, s.x.max(0.), s.y.max(0.));
        if let Some(lease) = &lease {
            lease.record(visual);
        }
        let ratio = |current: f32, final_size: f32| {
            if final_size > 0. {
                finite_scalar(current as f64 / final_size as f64)
            } else {
                1.
            }
        };
        let desired = Affine::translation(-bounds.x, -bounds.y)
            .then(Affine::scale(
                ratio(visual.width, bounds.width),
                ratio(visual.height, bounds.height),
            ))
            .then(Affine::translation(visual.x, visual.y));
        let local = Affine::translation(bounds.x, bounds.y)
            .then(desired)
            .then(Affine::translation(-bounds.x, -bounds.y));
        result.set(if local.is_finite() {
            local
        } else {
            Affine::IDENTITY
        });
    });
    output
}
