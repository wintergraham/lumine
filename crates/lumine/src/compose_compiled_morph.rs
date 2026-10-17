//! Reconcile compiled native variants by permanent IDs. No design interpreter or browser runtime.
use super::*;
use crate::motion::{Keyframe, Keyframes, MotionValue, Playback};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    time::Duration,
};

#[derive(Clone)]
pub struct CompiledMorphTiming {
    pub duration: Duration,
    pub crossfade: Duration,
    pub color_duration: Duration,
    pub curve: [f32; 49],
}
impl CompiledMorphTiming {
    fn animate(&self, motion: &MotionValue) {
        motion.set(0.);
        motion.animate_keyframes(
            Keyframes::new(
                self.curve
                    .iter()
                    .enumerate()
                    .map(|(i, v)| Keyframe::new(i as f32 / 48., *v)),
            )
            .expect("finite compiled morph curve"),
            self.duration,
            Playback::default(),
        );
    }
}
/// Optional authored motion for a stable compiled part.
#[derive(Clone)]
pub struct CompiledMorphAnimation {
    pub duration: Duration,
    pub delay: Duration,
    pub curve: [f32; 49],
    pub offset_y: f32,
    pub scale: f32,
    pub blur: f32,
}
#[derive(Clone, Default)]
pub struct CompiledMorphEffect {
    pub enter: Option<CompiledMorphAnimation>,
    pub enter_on_mount: bool,
    pub after_ancestor_reveal: bool,
    pub exit: Option<CompiledMorphAnimation>,
    pub pin_exit: bool,
    pub color_duration: Option<Duration>,
    pub single_row_grow: Option<Duration>,
    pub single_row_fade: Option<Duration>,
    pub grow_progress: Option<Signal<f32>>,
}
fn animate_authored(motion: &MotionValue, animation: &CompiledMorphAnimation) {
    motion.set(0.);
    motion.animate_keyframes(
        Keyframes::new(
            animation
                .curve
                .iter()
                .enumerate()
                .map(|(i, v)| Keyframe::new(i as f32 / 48., *v)),
        )
        .unwrap(),
        animation.duration,
        Playback {
            delay: animation.delay,
            ..Default::default()
        },
    );
}
fn authored(track: &StyleTrack, animation: &CompiledMorphAnimation, enter: bool) {
    let mut hidden = Styles::new()
        .opacity(0.)
        .translate(0., animation.offset_y)
        .scale(animation.scale, animation.scale)
        .blur(animation.blur);
    let shown = Styles::new()
        .opacity(1.)
        .translate(0., 0.)
        .scale(1., 1.)
        .blur(0.);
    if !enter {
        hidden.opacity = Some(0.);
    }
    track.decoration.set(Some(if enter {
        (hidden, shown)
    } else {
        (shown, hidden)
    }));
    animate_authored(&track.decoration_progress, animation);
}
#[derive(Clone, PartialEq)]
struct Footprint {
    from: Styles,
    to: Styles,
    restore: Option<Styles>,
}
#[derive(Clone)]
struct StyleTrack {
    endpoints: Signal<(Styles, Styles)>,
    progress: MotionValue,
    paint_progress: MotionValue,
    presented: Rc<RefCell<Styles>>,
    exiting: Signal<bool>,
    generation: Rc<Cell<u64>>,
    footprint: Signal<Option<Footprint>>,
    reveal_opacity: MotionValue,
    decoration: Signal<Option<(Styles, Styles)>>,
    decoration_progress: MotionValue,
    row_reveal: Signal<bool>,
}
impl StyleTrack {
    fn new(cx: &mut Context, styles: Styles) -> Self {
        Self {
            endpoints: cx.state((styles.clone(), styles.clone())),
            progress: MotionValue::new(cx, 1.),
            paint_progress: MotionValue::new(cx, 1.),
            presented: Rc::new(RefCell::new(styles)),
            exiting: cx.state(false),
            generation: Rc::new(Cell::new(0)),
            footprint: cx.state(None),
            reveal_opacity: MotionValue::new(cx, 1.),
            decoration: cx.state(None),
            decoration_progress: MotionValue::new(cx, 1.),
            row_reveal: cx.state(false),
        }
    }
    fn update(&self, target: Styles, timing: &CompiledMorphTiming, first: bool) {
        // A text/measurement refresh can rebuild the variant during a resize.
        // Keep the current run when its destination has not changed.
        if !first
            && !self.exiting.with_untracked(|v| *v)
            && self
                .endpoints
                .with_untracked(|(_, destination)| *destination == target)
        {
            return;
        }
        self.generation.set(self.generation.get().wrapping_add(1));
        if self.exiting.get() {
            self.decoration.set(None);
            self.decoration_progress.set(1.);
        }
        self.exiting.set(false);
        if first {
            self.endpoints.set((target.clone(), target.clone()));
            self.progress.set(1.);
            *self.presented.borrow_mut() = target;
            return;
        }
        let from = self.presented.borrow().clone();
        self.footprint.set(None);
        if from == target {
            return;
        }
        self.endpoints.set((from, target));
        timing.animate(&self.progress);
        self.paint_progress.set(0.);
        self.paint_progress.animate_to(
            1.,
            crate::motion::Transition::tween(timing.color_duration, crate::motion::Easing::EaseOut),
        );
    }
    fn sample(&self) -> Styles {
        let (from, to) = self.endpoints.get();
        let p = self.progress.get();
        let mut result = if let Some(reveal) = self.footprint.get() {
            if !self.progress.is_running()
                && !self.reveal_opacity.is_running()
                && reveal.restore.is_some()
            {
                reveal.restore.unwrap()
            } else {
                let mut sampled = interpolate(&reveal.from, &reveal.to, p);
                sampled.opacity = Some(self.reveal_opacity.get().clamp(0., 1.));
                if self.row_reveal.get() {
                    sampled.transform = Some(crate::scene::Transform {
                        x: 0.,
                        y: 3. * (1. - self.reveal_opacity.get()),
                    });
                    sampled.scale = Some([1.; 2]);
                }
                sampled
            }
        } else {
            interpolate(&from, &to, p)
        };
        let paint = interpolate(&from, &to, self.paint_progress.get());
        result.background = paint.background;
        result.text_color = paint.text_color;
        result.border_color = paint.border_color;
        if let Some((from, to)) = self.decoration.get() {
            let effect = interpolate(&from, &to, self.decoration_progress.get());
            result.opacity = Some(result.opacity.unwrap_or(1.) * effect.opacity.unwrap_or(1.));
            result.blur = Some(result.blur.unwrap_or(0.) + effect.blur.unwrap_or(0.));
            let scale = effect.scale.unwrap_or([1.; 2]);
            let base = result.scale.unwrap_or([1.; 2]);
            result.scale = Some([base[0] * scale[0], base[1] * scale[1]]);
            let offset = effect.transform.unwrap_or_default();
            let base = result.transform.unwrap_or_default();
            result.transform = Some(crate::scene::Transform {
                x: base.x + offset.x,
                y: base.y + offset.y,
            });
        }
        *self.presented.borrow_mut() = result.clone();
        result
    }
}
fn footprint(
    track: &StyleTrack,
    original: Styles,
    bounds: crate::scene::Rect,
    layout: Layout,
    gap: f32,
    neighbour: bool,
    enter: bool,
    timing: &CompiledMorphTiming,
) {
    if timing.duration.is_zero() {
        return;
    }
    track.row_reveal.set(false);
    let horizontal = layout == Layout::Row;
    let vertical = layout == Layout::Column;
    let mut open = original.clone();
    let mut margin = original.margin.unwrap_or_default();
    open.margin = Some(margin);
    open.margin_sides = 15;
    let leading = if horizontal { margin.left } else { margin.top };
    let size = if horizontal {
        bounds.width
    } else {
        bounds.height
    };
    // Keep the complete content allocation. Only its contribution to flow changes.
    if horizontal {
        open.width = Some(bounds.width);
    } else if vertical {
        open.height = Some(bounds.height);
    }
    if horizontal || vertical {
        open.flex_grow = Some(0.);
        open.flex_shrink = Some(0.);
    }
    open.clip_insets = Some(Some(crate::scene::Insets::all(-20.)));
    open.opacity = Some(1.);
    let mut shut = open.clone();
    if horizontal {
        margin.right = -(size + leading) - if neighbour { gap } else { 0. };
    } else if vertical {
        margin.bottom = -(size + leading) - if neighbour { gap } else { 0. };
    }
    shut.margin = Some(margin);
    shut.margin_sides = 15;
    shut.clip_insets = Some(Some(if horizontal {
        crate::scene::Insets {
            right: bounds.width,
            ..Default::default()
        }
    } else if vertical {
        crate::scene::Insets {
            bottom: bounds.height,
            ..Default::default()
        }
    } else {
        crate::scene::Insets::default()
    }));
    shut.transform = Some(crate::scene::Transform {
        x: 0.,
        y: if horizontal { 0. } else { 6. },
    });
    shut.scale = Some(
        [if horizontal {
            0.88
        } else if vertical {
            0.98
        } else {
            0.97
        }; 2],
    );
    open.transform = Some(crate::scene::Transform::default());
    open.scale = Some([1.; 2]);
    if horizontal {
        open.height = Some(bounds.height);
        shut.height = Some(0.);
        shut.min_height = Some(0.);
        open.layout_options_mut().align_self = Some(crate::scene::Align::Center);
        shut.layout_options_mut().align_self = Some(crate::scene::Align::Center);
    }
    let mut leaving = track.presented.borrow().clone();
    if leaving.clip_insets.flatten().is_none() {
        leaving.clip_insets = open.clip_insets;
    }
    if leaving.margin.is_none() {
        leaving.margin = Some(Default::default());
    }
    if leaving.transform.is_none() {
        leaving.transform = Some(Default::default());
    }
    if leaving.scale.is_none() {
        leaving.scale = Some([1.; 2]);
    }
    track.footprint.set(Some(Footprint {
        from: if enter { shut.clone() } else { leaving },
        to: if enter { open } else { shut },
        restore: enter.then_some(original),
    }));
    timing.animate(&track.progress);
    // CSS applies ease-out to the complete opacity timeline, then samples its stops.
    let curve = crate::motion::Easing::cubic_bezier(0., 0., 0.58, 1.);
    let frames = (0..=48).map(|i| {
        let t = curve.sample(i as f32 / 48.);
        Keyframe::new(
            i as f32 / 48.,
            if enter {
                ((t - 0.15) / 0.85).clamp(0., 1.)
            } else {
                (1. - t / 0.6).clamp(0., 1.)
            },
        )
    });
    track.reveal_opacity.set(if enter {
        0.
    } else {
        track.presented.borrow().opacity.unwrap_or(1.)
    });
    track.reveal_opacity.animate_keyframes(
        Keyframes::new(frames).unwrap(),
        timing.duration,
        Playback::default(),
    );
}
struct TextTrack {
    value: Signal<String>,
    previous: Signal<String>,
    enter: MotionValue,
    leave: MotionValue,
}
enum ContentTrack {
    None,
    Text(TextTrack),
    Image {
        source: Rc<RefCell<Box<dyn FnMut() -> std::sync::Arc<crate::image::ImageData>>>>,
        revision: Signal<u64>,
    },
    Svg(Signal<std::sync::Arc<crate::svg::SvgData>>),
}
struct Mounted {
    node: NodeId,
    child_parent: NodeId,
    parent: Option<String>,
    tag: u8,
    style: StyleTrack,
    content: ContentTrack,
    effect: Option<CompiledMorphEffect>,
}
struct Spec {
    id: String,
    parent: Option<String>,
    view: View,
}
fn flatten(mut view: View, parent: Option<String>, out: &mut Vec<Spec>, ids: &mut HashSet<String>) {
    let id = view
        .id
        .clone()
        .filter(|id| !id.is_empty())
        .expect("compiled morph requires an ID on every native node");
    assert!(
        ids.insert(id.clone()),
        "duplicate compiled morph identity {id}"
    );
    let children = std::mem::take(&mut view.children);
    out.push(Spec {
        id: id.clone(),
        parent,
        view,
    });
    for child in children {
        flatten(child, Some(id.clone()), out, ids);
    }
}
fn tag(kind: &Kind) -> u8 {
    match kind {
        Kind::Container(_) => 0,
        Kind::Text(_) => 1,
        Kind::BoundText(_) => 2,
        Kind::Image { .. } => 3,
        Kind::Svg { .. } => 4,
        Kind::Component(_) => 5,
        Kind::Scroll {
            horizontal: true, ..
        } => 6,
        Kind::Scroll {
            horizontal: false, ..
        } => 7,
        Kind::Button => 8,
        _ => panic!("compiled morph supports compiled nodes and native component bindings"),
    }
}
fn target_style(view: &View) -> Styles {
    let mut style = view.styles.clone();
    if let Kind::Container(layout) = view.kind {
        style.layout = Some(layout);
    }
    style
}
fn content(cx: &mut Context, view: &mut View) -> ContentTrack {
    match &mut view.kind {
        Kind::Text(text) => {
            let value = cx.state(text.clone());
            let previous = cx.state(String::new());
            let enter = MotionValue::new(cx, 1.);
            let leave = MotionValue::new(cx, 1.);
            let read = value.clone();
            let old = previous.clone();
            let opacity = enter.clone();
            let old_opacity = leave.clone();
            view.kind = Kind::Component(Box::new(move |_| {
                overlay()
                    .child(
                        text_signal(move || read.get())
                            .reactive_style(move || Styles::new().opacity(opacity.get())),
                    )
                    .child(
                        text_signal(move || old.get())
                            .absolute()
                            .left(0.)
                            .top(0.)
                            .disabled(true)
                            .reactive_style(move || {
                                Styles::new()
                                    .opacity(1. - old_opacity.get())
                                    .when(old_opacity.get() >= 1., |s| s.hidden())
                            }),
                    )
            }));
            ContentTrack::Text(TextTrack {
                value,
                previous,
                enter,
                leave,
            })
        }
        Kind::Image { source, .. } => {
            let live = Rc::new(RefCell::new(std::mem::replace(
                source,
                Box::new(|| unreachable!()),
            )));
            let revision = cx.state(0u64);
            let read = live.clone();
            let changed = revision.clone();
            *source = Box::new(move || {
                let _ = changed.get();
                (read.borrow_mut())()
            });
            ContentTrack::Image {
                source: live,
                revision,
            }
        }
        Kind::Svg { source, .. } => {
            let value = cx.state(source());
            let read = value.clone();
            *source = Box::new(move || read.get());
            ContentTrack::Svg(value)
        }
        _ => ContentTrack::None,
    }
}
fn update_content(track: &ContentTrack, view: &mut View, timing: &CompiledMorphTiming) {
    match (track, &mut view.kind) {
        (ContentTrack::Text(track), Kind::Text(text)) => {
            if track.value.get() != *text {
                track.previous.set(track.value.get());
                track.value.set(text.clone());
                track.leave.set(0.);
                track.enter.set(0.);
                track.leave.animate_to(
                    1.,
                    crate::motion::Transition::tween(
                        timing.crossfade,
                        crate::motion::Easing::EaseOut,
                    ),
                );
                track.enter.animate_to(
                    1.,
                    crate::motion::Transition::tween(
                        timing.crossfade.mul_f32(1.3),
                        crate::motion::Easing::EaseOut,
                    )
                    .delay(timing.crossfade.mul_f32(0.2)),
                );
            }
        }
        (
            ContentTrack::Image {
                source: live,
                revision,
            },
            Kind::Image { source, .. },
        ) => {
            *live.borrow_mut() = std::mem::replace(source, Box::new(|| unreachable!()));
            revision.update(|n| *n = n.wrapping_add(1));
        }
        (ContentTrack::Svg(value), Kind::Svg { source, .. }) => {
            value.set(source());
        }
        _ => {}
    }
}

/// Variants must have the same permanent ID for a matched part and unique IDs for paint children.
/// Native component bindings keep their mounted controls, tasks, focus and editor handles.
pub fn compiled_morph<K: PartialEq + Clone + 'static>(
    mut selected: impl FnMut() -> K + 'static,
    mut build: impl FnMut(K) -> View + 'static,
    mut timing: impl FnMut() -> CompiledMorphTiming + 'static,
) -> View {
    View::new(Kind::Region(Box::new(move |ui, root, environment| {
        let weak = ui.downgrade();
        let runtime = ui.runtime.clone();
        let mounted = Rc::new(RefCell::new(HashMap::<String, Mounted>::new()));
        let mut previous = None;
        ui.bind(root, move || {
            let key = selected();
            if previous.as_ref() == Some(&key) {
                return;
            }
            let mut specs = Vec::new();
            let mut ids = HashSet::new();
            // Validate the complete candidate graph before touching the mounted variant.
            flatten(build(key.clone()), None, &mut specs, &mut ids);
            for spec in &specs {
                let _ = tag(&spec.view.kind);
            }
            let Some(mut ui) = weak.upgrade() else {
                return;
            };
            if !ui.scene.borrow().contains(root) {
                return;
            }
            let options = timing();
            let first = previous.is_none();
            runtime.untracked(|| {
                let mut live = mounted.borrow_mut();
                let existing: HashSet<_> = live.keys().cloned().collect();
                let mut fresh_children = HashMap::<String, usize>::new();
                for spec in &specs {
                    if !existing.contains(&spec.id) {
                        if let Some(parent) = &spec.parent {
                            *fresh_children.entry(parent.clone()).or_default() += 1;
                        }
                    }
                }
                let mut reveal_delays = HashMap::<String, Duration>::new();
                let mut order = HashMap::<NodeId, Vec<NodeId>>::new();
                for mut spec in specs {
                    let parent = spec
                        .parent
                        .as_ref()
                        .map_or(root, |id| live[id].child_parent);
                    let kind = tag(&spec.view.kind);
                    let target = target_style(&spec.view);
                    let effect = spec.view.morph_effect.clone();
                    let inherited_delay = spec
                        .parent
                        .as_ref()
                        .and_then(|id| reveal_delays.get(id))
                        .copied()
                        .unwrap_or_default();
                    let row_delay = if !first
                        && !options.duration.is_zero()
                        && !existing.contains(&spec.id)
                        && spec.parent.as_ref().is_some_and(|id| {
                            existing.contains(id) && fresh_children.get(id) == Some(&1)
                        }) {
                        effect
                            .as_ref()
                            .and_then(|e| e.single_row_grow)
                            .unwrap_or_default()
                    } else {
                        Duration::ZERO
                    };
                    reveal_delays.insert(spec.id.clone(), inherited_delay + row_delay);

                    if let Some(old) = live.get_mut(&spec.id) {
                        assert_eq!(
                            old.parent, spec.parent,
                            "compiled morph part changed parent without an identity migration"
                        );
                        assert_eq!(
                            old.tag, kind,
                            "compiled morph part changed kind without an identity migration"
                        );
                        update_content(&old.content, &mut spec.view, &options);
                        let mut local = options.clone();
                        if let Some(duration) = effect.as_ref().and_then(|e| e.color_duration) {
                            local.color_duration = duration;
                        }
                        old.style.update(target, &local, first);
                        old.effect = effect;
                        order.entry(parent).or_default().push(old.node);
                        continue;
                    }
                    let mut cx = Context::new(runtime.clone(), environment.clone());
                    let style = StyleTrack::new(&mut cx, target.clone());
                    let content = content(&mut cx, &mut spec.view);
                    let draw = style.clone();
                    let mut own = spec.view.reactive_style.take();
                    spec.view.styles = Styles::default();
                    spec.view.reactive_style = Some(Box::new(move || {
                        let mut result = draw.sample();
                        if let Some(own) = &mut own {
                            result.merge(&own());
                        }
                        result
                    }));
                    let disabled = style.exiting.clone();
                    let mut own_disabled = spec.view.disabled.take();
                    spec.view.disabled = Some(Box::new(move || {
                        disabled.get() || own_disabled.as_mut().is_some_and(|f| f())
                    }));
                    let child_target = if matches!(spec.view.kind, Kind::Scroll { .. }) {
                        let target = runtime.signal(None);
                        spec.view.layout_target = Some(target.clone());
                        Some(target)
                    } else {
                        None
                    };
                    let node = mount(&mut ui, parent, spec.view, environment.clone());
                    let child_parent = child_target.and_then(|target| target.get()).unwrap_or(node);
                    cx.finish(&mut ui, node);
                    if !options.duration.is_zero()
                        && (!first || effect.as_ref().is_some_and(|e| e.enter_on_mount))
                    {
                        if let Some(animation) = effect.as_ref().and_then(|e| e.enter.as_ref()) {
                            let mut animation = animation.clone();
                            if effect.as_ref().is_some_and(|e| e.after_ancestor_reveal) {
                                animation.delay += inherited_delay;
                            }
                            authored(&style, &animation, true);
                        }
                    }
                    // Reveal whole entering branches after their first complete layout,
                    // before paint. Descendants keep their authored dimensions throughout.
                    if !first
                        && !options.duration.is_zero()
                        && effect.as_ref().is_none_or(|e| e.enter.is_none())
                        && spec.parent.as_ref().is_none_or(|id| existing.contains(id))
                    {
                        let bounds = ui.observe_layout_bounds(node);
                        let entering = style.clone();
                        let mut timing = options.clone();
                        let single = effect
                            .as_ref()
                            .and_then(|e| e.single_row_grow)
                            .filter(|d| !d.is_zero())
                            .filter(|_| {
                                spec.parent
                                    .as_ref()
                                    .is_some_and(|id| fresh_children.get(id) == Some(&1))
                            });
                        let grow_progress = effect.as_ref().and_then(|e| e.grow_progress.clone());
                        let row_fade = effect
                            .as_ref()
                            .and_then(|e| e.single_row_fade)
                            .unwrap_or(Duration::from_millis(220));
                        if let Some(duration) = single {
                            timing.duration = duration;
                            timing.curve = std::array::from_fn(|i| {
                                crate::motion::Easing::cubic_bezier(0.25, 0.8, 0.25, 1.)
                                    .sample(i as f32 / 48.)
                            });
                        }
                        if let Some(out) = &grow_progress {
                            let out = out.clone();
                            let motion = style.progress.clone();
                            ui.bind(node, move || {
                                out.set(motion.get());
                            });
                        }
                        let weak = ui.downgrade();
                        let mut started = false;
                        ui.bind(node, move || {
                            if started {
                                return;
                            }
                            let Some(bounds) = bounds.get() else {
                                return;
                            };
                            started = true;
                            if entering.exiting.get() {
                                return;
                            }
                            let Some(ui) = weak.upgrade() else {
                                return;
                            };
                            let scene = ui.scene.borrow();
                            let layout = match scene.kind(parent) {
                                NodeKind::Container(layout) | NodeKind::Panel { layout, .. } => {
                                    *layout
                                }
                                _ => Layout::Overlay,
                            };
                            let gap = scene.style(parent).gap;
                            let neighbour = scene
                                .children(parent)
                                .iter()
                                .filter(|n| !scene.style(**n).absolute)
                                .count()
                                > 1;
                            drop(scene);
                            footprint(
                                &entering,
                                target.clone(),
                                bounds,
                                layout,
                                gap,
                                neighbour,
                                true,
                                &timing,
                            );
                            if let Some(duration) =
                                single.filter(|_| neighbour && layout == Layout::Column)
                            {
                                entering.footprint.update(|value| {
                                    if let Some(value) = value {
                                        value.from.transform = Some(Default::default());
                                        value.from.scale = Some([1.; 2]);
                                    }
                                });
                                entering.row_reveal.set(true);
                                entering.reveal_opacity.set(0.);
                                entering.reveal_opacity.animate_to(
                                    1.,
                                    crate::motion::Transition::tween(
                                        row_fade,
                                        crate::motion::Easing::cubic_bezier(0., 0., 0.58, 1.),
                                    )
                                    .delay(duration),
                                );
                                if let Some(progress) = &grow_progress {
                                    let motion = entering.progress.clone();
                                    let out = progress.clone();
                                    ui.runtime.untracked(|| out.set(motion.get()));
                                }
                            }
                        });
                    }
                    order.entry(parent).or_default().push(node);
                    live.insert(
                        spec.id,
                        Mounted {
                            node,
                            child_parent,
                            parent: spec.parent,
                            tag: kind,
                            style,
                            content,
                            effect,
                        },
                    );
                }
                let dead: HashSet<_> = live
                    .keys()
                    .filter(|id| !ids.contains(*id))
                    .cloned()
                    .collect();
                let branches: Vec<_> = dead
                    .iter()
                    .filter(|id| {
                        live[*id]
                            .parent
                            .as_ref()
                            .is_none_or(|parent| !dead.contains(parent))
                    })
                    .cloned()
                    .collect();
                for id in branches {
                    if options.duration.is_zero() {
                        let node = live[&id].node;
                        let subtree: Vec<_> = live
                            .iter()
                            .filter(|(_, e)| {
                                e.node == node
                                    || ui.scene.borrow().ancestors(e.node).any(|p| p == node)
                            })
                            .map(|(id, _)| id.clone())
                            .collect();
                        ui.remove(node);
                        for part in subtree {
                            live.remove(&part);
                        }
                        continue;
                    }
                    let old = &live[&id];
                    if old.style.exiting.get() {
                        continue;
                    }
                    old.style.exiting.set(true);
                    old.style
                        .generation
                        .set(old.style.generation.get().wrapping_add(1));
                    let generation = old.style.generation.get();
                    let marker = old.style.generation.clone();
                    let from = old.style.presented.borrow().clone();
                    let bounds = ui.scene.borrow().bounds(old.node);
                    let parent = ui.scene.borrow().parent(old.node).unwrap_or(root);
                    let scene = ui.scene.borrow();
                    let layout = match scene.kind(parent) {
                        NodeKind::Container(layout) | NodeKind::Panel { layout, .. } => *layout,
                        _ => Layout::Overlay,
                    };
                    let gap = scene.style(parent).gap;
                    let neighbour = scene
                        .children(parent)
                        .iter()
                        .filter(|n| !scene.style(**n).absolute)
                        .count()
                        > 1;
                    drop(scene);
                    let animation = old.effect.as_ref().and_then(|e| e.exit.as_ref());
                    if let Some(animation) = animation {
                        if old.effect.as_ref().is_some_and(|e| e.pin_exit) {
                            let p = ui.scene.borrow().bounds(parent);
                            let pinned = from
                                .clone()
                                .absolute()
                                .left(bounds.x - p.x)
                                .top(bounds.y - p.y)
                                .w(bounds.width)
                                .h(bounds.height)
                                .m(0.);
                            old.style.endpoints.set((pinned.clone(), pinned));
                            old.style.progress.set(1.);
                            old.style.footprint.set(None);
                        }
                        authored(&old.style, animation, false);
                    } else {
                        footprint(
                            &old.style, from, bounds, layout, gap, neighbour, false, &options,
                        );
                    }
                    let weak = ui.downgrade();
                    let nodes = mounted.clone();
                    let removed = id.clone();
                    let delay = animation.map_or(options.duration, |a| a.duration + a.delay);
                    let mut cx = Context::new(runtime.clone(), environment.clone());
                    cx.tasks().spawn(async move {
                        crate::timer::sleep(delay).await;
                        if marker.get() != generation {
                            return;
                        }
                        let Some(mut ui) = weak.upgrade() else {
                            return;
                        };
                        let mut live = nodes.borrow_mut();
                        let Some(node) = live.get(&removed).map(|entry| entry.node) else {
                            return;
                        };
                        let subtree: Vec<_> = live
                            .iter()
                            .filter(|(_, entry)| {
                                entry.node == node
                                    || ui.scene.borrow().ancestors(entry.node).any(|p| p == node)
                            })
                            .map(|(id, _)| id.clone())
                            .collect();
                        ui.remove(node);
                        for id in subtree {
                            live.remove(&id);
                        }
                    });
                    cx.finish(&mut ui, old.node);
                }
                for (parent, wanted) in order {
                    let current = ui.scene.borrow().children(parent).to_vec();
                    let managed: HashSet<_> = live
                        .values()
                        .filter(|entry| ui.scene.borrow().parent(entry.node) == Some(parent))
                        .map(|entry| entry.node)
                        .collect();
                    let mut sorted: Vec<_> = current
                        .iter()
                        .copied()
                        .filter(|id| !managed.contains(id))
                        .collect();
                    // Keep disappearing slots next to their former preceding sibling.
                    for node in wanted {
                        sorted.push(node);
                        if let Some(i) = current.iter().position(|old| *old == node) {
                            for &after in &current[i + 1..] {
                                if !managed.contains(&after) {
                                    continue;
                                }
                                if live
                                    .values()
                                    .any(|entry| entry.node == after && entry.style.exiting.get())
                                {
                                    sorted.push(after);
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                    for &node in &current {
                        if !sorted.contains(&node) {
                            sorted.push(node);
                        }
                    }
                    assert!(
                        ui.scene.borrow_mut().reorder_children(parent, &sorted),
                        "compiled morph child permutation"
                    );
                }
            });
            previous = Some(key);
        });
    })))
}

fn interpolate(from: &Styles, to: &Styles, t: f32) -> Styles {
    let mut out = to.clone();
    fn number(a: Option<f32>, b: Option<f32>, t: f32) -> Option<f32> {
        match (a, b) {
            (Some(a), Some(b)) => Some(a + (b - a) * t),
            _ => b,
        }
    }
    macro_rules! scalar {($($key:ident),*)=>{$(out.$key=number(from.$key,to.$key,t);)*};}
    scalar!(
        width,
        height,
        min_width,
        max_width,
        min_height,
        max_height,
        gap,
        text_size,
        radius,
        border_width,
        blur,
        edge_fade
    );
    out.opacity = Some(
        number(
            Some(from.opacity.unwrap_or(1.)),
            Some(to.opacity.unwrap_or(1.)),
            t,
        )
        .unwrap()
        .clamp(0., 1.),
    );
    out.rotation = Some(
        from.rotation.unwrap_or(0.) + (to.rotation.unwrap_or(0.) - from.rotation.unwrap_or(0.)) * t,
    );
    out.width = out.width.map(|v| v.max(0.));
    out.height = out.height.map(|v| v.max(0.));
    out.radius = out.radius.map(|v| v.max(0.));
    out.border_width = out.border_width.map(|v| v.max(0.));
    fn insets(a: crate::scene::Insets, b: crate::scene::Insets, t: f32) -> crate::scene::Insets {
        crate::scene::Insets {
            top: a.top + (b.top - a.top) * t,
            right: a.right + (b.right - a.right) * t,
            bottom: a.bottom + (b.bottom - a.bottom) * t,
            left: a.left + (b.left - a.left) * t,
        }
    }
    if let (Some(a), Some(b)) = (from.padding, to.padding) {
        out.padding = Some(insets(a, b, t));
    }
    // Interrupted footprints carry flow/paint offsets absent from authored styles.
    if from.margin.is_some() || to.margin.is_some() {
        out.margin = Some(insets(
            from.margin.unwrap_or_default(),
            to.margin.unwrap_or_default(),
            t,
        ));
    }
    if from.clip_insets.flatten().is_some() || to.clip_insets.flatten().is_some() {
        out.clip_insets = Some(if t == 1. {
            to.clip_insets.flatten()
        } else {
            Some(insets(
                from.clip_insets.flatten().unwrap_or_default(),
                to.clip_insets.flatten().unwrap_or_default(),
                t,
            ))
        });
    }
    if from.transform.is_some() || to.transform.is_some() {
        let a = from.transform.unwrap_or_default();
        let b = to.transform.unwrap_or_default();
        out.transform = Some(crate::scene::Transform {
            x: a.x + (b.x - a.x) * t,
            y: a.y + (b.y - a.y) * t,
        });
    }
    if from.scale.is_some() || to.scale.is_some() {
        let a = from.scale.unwrap_or([1.; 2]);
        let b = to.scale.unwrap_or([1.; 2]);
        out.scale = Some(std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t));
    }
    fn color(a: crate::scene::Color, b: crate::scene::Color, t: f32) -> crate::scene::Color {
        let mix = |a: u8, b: u8| {
            (a as f32 + (b as f32 - a as f32) * t.clamp(0., 1.))
                .round()
                .clamp(0., 255.) as u8
        };
        crate::scene::Color(mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2), mix(a.3, b.3))
    }
    macro_rules! colors {($($key:ident),*)=>{$(if let(Some(a),Some(b))=(from.$key,to.$key){out.$key=Some(color(a,b,t));})*};}
    colors!(background, text_color, border_color);
    if let (Some(a), Some(b)) = (&from.layout_options, &to.layout_options) {
        let mut options = (**b).clone();
        fn length(
            a: Option<crate::layout::Length>,
            b: Option<crate::layout::Length>,
            t: f32,
        ) -> Option<crate::layout::Length> {
            match (a, b) {
                (Some(crate::layout::Length::Px(a)), Some(crate::layout::Length::Px(b))) => {
                    Some(crate::layout::Length::Px(a + (b - a) * t))
                }
                _ => b,
            }
        }
        options.basis = length(a.basis, b.basis, t);
        options.gap_x = length(a.gap_x, b.gap_x, t);
        options.gap_y = length(a.gap_y, b.gap_y, t);
        macro_rules! edges {
            ($key:ident) => {
                options.$key.top = length(a.$key.top, b.$key.top, t);
                options.$key.right = length(a.$key.right, b.$key.right, t);
                options.$key.bottom = length(a.$key.bottom, b.$key.bottom, t);
                options.$key.left = length(a.$key.left, b.$key.left, t);
            };
        }
        edges!(padding);
        edges!(margin);
        edges!(inset);
        out.layout_options = Some(std::sync::Arc::new(options));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_glyph_exit_keeps_the_replacement_and_neighbours_in_their_slots() {
        let mut ui = Ui::new(200., 200.);
        let selected = ui.signal(false);
        let read = selected.clone();
        let tasks = Rc::new(RefCell::new(crate::task::LocalExecutor::new()));
        let frames = crate::frame::FrameClock::new();
        let root = ui.mount(provide(
            TaskRunner::from_executor(tasks),
            provide(
                frames,
                compiled_morph(
                    move || read.get(),
                    move |changed| {
                        row()
                            .id("root")
                            .gap(8.)
                            .child(
                                div()
                                    .id(if changed { "new" } else { "old" })
                                    .size(20., 20.)
                                    .morph_effect(CompiledMorphEffect {
                                        enter: Some(CompiledMorphAnimation {
                                            duration: Duration::from_millis(320),
                                            delay: Duration::from_millis(40),
                                            curve: std::array::from_fn(|i| i as f32 / 48.),
                                            offset_y: 0.,
                                            scale: 0.85,
                                            blur: 3.,
                                        }),
                                        exit: Some(CompiledMorphAnimation {
                                            duration: Duration::from_millis(240),
                                            delay: Duration::ZERO,
                                            curve: std::array::from_fn(|i| i as f32 / 48.),
                                            offset_y: 0.,
                                            scale: 0.85,
                                            blur: 3.,
                                        }),
                                        pin_exit: true,
                                        ..Default::default()
                                    }),
                            )
                            .child(div().id("label").size(80., 20.))
                    },
                    || CompiledMorphTiming {
                        duration: Duration::from_millis(400),
                        crossfade: Duration::from_millis(140),
                        color_duration: Duration::from_millis(200),
                        curve: std::array::from_fn(|i| i as f32 / 48.),
                    },
                ),
            ),
        ));
        ui.prepare_frame();
        let label = root.find("label").unwrap();
        let initial = ui.scene.borrow().bounds(label);
        selected.set(true);
        ui.prepare_frame();
        assert!(
            ui.scene.borrow().style(root.find("old").unwrap()).absolute,
            "leaving glyph must stop contributing to flow"
        );
        assert_eq!(
            ui.scene.borrow().bounds(label).x,
            initial.x,
            "crossfading a glyph must not push its label sideways"
        );
        assert_eq!(
            ui.scene.borrow().layout_bounds(root.find("new").unwrap()).x,
            0.
        );
    }

    #[test]
    fn authored_row_waits_for_connector_growth_before_revealing() {
        use crate::{
            frame::{Frame, FrameClock},
            task::LocalExecutor,
        };
        let mut ui = Ui::new(200., 200.);
        let tasks = Rc::new(RefCell::new(LocalExecutor::new()));
        let frames = FrameClock::new();
        let selected = ui.signal(false);
        let read = selected.clone();
        let connector = ui.signal(1.);
        let progress = connector.clone();
        let root = ui.mount(provide(
            TaskRunner::from_executor(tasks.clone()),
            provide(
                frames.clone(),
                compiled_morph(
                    move || read.get(),
                    move |show| {
                        let enter = CompiledMorphAnimation {
                            duration: Duration::from_millis(100),
                            delay: Duration::from_millis(50),
                            curve: std::array::from_fn(|i| i as f32 / 48.),
                            offset_y: 4.,
                            scale: 1.,
                            blur: 0.,
                        };
                        let view = column().id("root").w(160.).gap(8.).child(
                            div().id("first").h(40.).child(
                                div().id("mount-item").size(20., 20.).morph_effect(
                                    CompiledMorphEffect {
                                        enter: Some(enter.clone()),
                                        enter_on_mount: true,
                                        ..Default::default()
                                    },
                                ),
                            ),
                        );
                        if show {
                            view.child(
                                div()
                                    .id("row")
                                    .h(40.)
                                    .morph_effect(CompiledMorphEffect {
                                        single_row_grow: Some(Duration::from_millis(200)),
                                        single_row_fade: Some(Duration::from_millis(220)),
                                        grow_progress: Some(progress.clone()),
                                        ..Default::default()
                                    })
                                    .child(div().id("plan-item").size(20., 20.).morph_effect(
                                        CompiledMorphEffect {
                                            enter: Some(enter),
                                            enter_on_mount: true,
                                            after_ancestor_reveal: true,
                                            ..Default::default()
                                        },
                                    )),
                            )
                        } else {
                            view
                        }
                    },
                    || CompiledMorphTiming {
                        duration: Duration::from_millis(400),
                        crossfade: Duration::from_millis(140),
                        color_duration: Duration::from_millis(200),
                        curve: std::array::from_fn(|i| i as f32 / 48.),
                    },
                ),
            ),
        ));
        ui.prepare_frame();
        assert_eq!(
            ui.scene
                .borrow()
                .effects(root.find("mount-item").unwrap())
                .opacity,
            0.,
            "authored mount animation was skipped"
        );
        selected.set(true);
        ui.prepare_frame();
        let row = root.find("row").unwrap();
        let mut time = std::time::Instant::now();
        let mut index = 0;
        let mut advance = |ui: &mut Ui, ms: u64| {
            tasks.borrow_mut().tick();
            index += 1;
            time += Duration::from_millis(ms);
            frames.deliver(Frame {
                index,
                time,
                interval: Duration::from_millis(ms),
            });
            tasks.borrow_mut().tick();
            ui.prepare_frame();
        };
        advance(&mut ui, 0);
        advance(&mut ui, 100);
        assert!(
            connector.get() > 0. && connector.get() < 1.,
            "connector must grow while the row is hidden"
        );
        assert_eq!(ui.scene.borrow().effects(row).opacity, 0.);
        advance(&mut ui, 110);
        let opacity = ui.scene.borrow().effects(row).opacity;
        assert!(
            opacity > 0. && opacity < 1.,
            "row should start fading after its connector opens: {opacity}"
        );
        assert_eq!(
            ui.scene
                .borrow()
                .effects(root.find("plan-item").unwrap())
                .opacity,
            0.,
            "plan child started before its parent's reveal delay"
        );
        advance(&mut ui, 60);
        assert!(
            ui.scene
                .borrow()
                .effects(root.find("plan-item").unwrap())
                .opacity
                > 0.
        );
        advance(&mut ui, 240);
        assert_eq!(ui.scene.borrow().effects(row).opacity, 1.);
        assert_eq!(ui.scene.borrow().layout_bounds(row).height, 40.);
    }

    #[test]
    fn entering_branch_reveals_without_rewrapping_and_can_be_interrupted() {
        use crate::{
            frame::{Frame, FrameClock},
            task::LocalExecutor,
        };
        let mut ui = Ui::new(300., 300.);
        let executor = Rc::new(RefCell::new(LocalExecutor::new()));
        let frames = FrameClock::new();
        let selected = ui.signal(false);
        let choice = selected.clone();
        let root = ui.mount(provide(
            TaskRunner::from_executor(executor.clone()),
            provide(
                frames.clone(),
                compiled_morph(
                    move || choice.get(),
                    |expanded| {
                        let mut view = column()
                            .id("root")
                            .w(220.)
                            .gap(8.)
                            .child(
                                div()
                                    .id("decoration")
                                    .absolute()
                                    .left(0.)
                                    .top(0.)
                                    .size(1., 1.),
                            )
                            .child(div().id("before").h(40.));
                        if expanded {
                            view = view.child(
                                column().id("entry").w(180.).h(48.).child(
                                    text("A stable sentence that wraps at the authored width")
                                        .id("copy")
                                        .text_wrap(true),
                                ),
                            );
                        }
                        view.child(div().id("after").h(20.))
                    },
                    || CompiledMorphTiming {
                        duration: Duration::from_millis(400),
                        crossfade: Duration::from_millis(100),
                        color_duration: Duration::from_millis(100),
                        curve: std::array::from_fn(|i| i as f32 / 48.),
                    },
                ),
            ),
        ));
        ui.prepare_frame();
        let after = root.find("after").unwrap();
        let initial_y = ui.scene.borrow().layout_bounds(after).y;
        let initial_height = ui
            .scene
            .borrow()
            .layout_bounds(root.find("root").unwrap())
            .height;
        selected.set(true);
        ui.prepare_frame();
        let entry = root.find("entry").unwrap();
        let copy = root.find("copy").unwrap();
        let full_copy = ui.scene.borrow().layout_bounds(copy);
        assert_eq!(ui.scene.borrow().layout_bounds(entry).height, 48.);
        assert!((ui.scene.borrow().layout_bounds(after).y - initial_y).abs() < 0.01);
        assert!(
            (ui.scene
                .borrow()
                .layout_bounds(root.find("root").unwrap())
                .height
                - initial_height)
                .abs()
                < 0.01,
            "the auto-height parent must not regain the cancelled gap"
        );
        let mut time = std::time::Instant::now();
        let mut index = 0;
        let mut advance = |ui: &mut Ui, ms: u64| {
            executor.borrow_mut().tick();
            index += 1;
            time += Duration::from_millis(ms);
            frames.deliver(Frame {
                index,
                time,
                interval: Duration::from_millis(ms),
            });
            executor.borrow_mut().tick();
            ui.prepare_frame();
        };
        advance(&mut ui, 200);
        let middle = ui.scene.borrow().layout_bounds(after).y;
        assert!(
            middle > initial_y && middle < initial_y + 56.,
            "reveal must animate its flow slot: {middle}"
        );
        assert_eq!(ui.scene.borrow().layout_bounds(entry).height, 48.);
        let middle_copy = ui.scene.borrow().layout_bounds(copy);
        assert_eq!(
            (middle_copy.width, middle_copy.height),
            (full_copy.width, full_copy.height)
        );
        assert!(ui.scene.borrow().style(entry).clip_insets.unwrap().bottom > 0.);
        selected.set(false);
        ui.prepare_frame();
        advance(&mut ui, 80);
        selected.set(true);
        ui.prepare_frame();
        assert_eq!(root.find("entry"), Some(entry));
        advance(&mut ui, 500);
        assert_eq!(ui.scene.borrow().layout_bounds(entry).height, 48.);
        assert!((ui.scene.borrow().layout_bounds(after).y - initial_y - 56.).abs() < 0.01);
        assert_eq!(ui.scene.borrow().style(entry).clip_insets, None);
        root.unmount();
        executor.borrow_mut().tick();
    }

    #[test]
    fn approval_reversal_preserves_presented_height_without_another_frame() {
        use crate::{
            frame::{Frame, FrameClock},
            task::LocalExecutor,
        };
        let mut ui = Ui::new(1440., 900.);
        let executor = Rc::new(RefCell::new(LocalExecutor::new()));
        let frames = FrameClock::new();
        let selected = ui.signal(false);
        let choice = selected.clone();
        let root = ui.mount(provide(
            TaskRunner::from_executor(executor.clone()),
            provide(
                frames.clone(),
                compiled_morph(
                    move || choice.get(),
                    |reasoning| {
                        let mut details = column().id("details").w_full().gap(7.5);
                        for i in 0..5 {
                            if !reasoning || i >= 3 {
                                details = details.child(
                                    row()
                                        .id(format!("detail-{i}"))
                                        .w_full()
                                        .child(div().id(format!("value-{i}")).size(100., 22.)),
                                );
                            }
                        }
                        let mut dock = column()
                            .id("dock")
                            .w_full()
                            .gap(15.)
                            .px(13.5)
                            .pt(16.5)
                            .pb(12.)
                            .child(div().id("header").h(27.))
                            .child(div().id("title").h(27.));
                        if reasoning {
                            dock = dock.child(
                                column()
                                    .id("rationale")
                                    .w_full()
                                    .child(div().id("rationale-copy").w_full().h(81.)),
                            );
                        }
                        dock = dock.child(details).child(div().id("actions").h(45.));
                        column()
                            .id("composer-root")
                            .absolute()
                            .left(441.5)
                            .bottom(318.5)
                            .w(594.)
                            .gap(15.)
                            .child(div().id("overlays").h(0.))
                            .child(
                                column()
                                    .id("composer")
                                    .w_full()
                                    .gap(9.)
                                    .child(
                                        column()
                                            .id("body")
                                            .w_full()
                                            .child(dock)
                                            .child(div().id("input").h(64.5)),
                                    )
                                    .child(div().id("footer").h(39.)),
                            )
                    },
                    || CompiledMorphTiming {
                        duration: Duration::from_millis(400),
                        crossfade: Duration::from_millis(140),
                        color_duration: Duration::from_millis(200),
                        curve: std::array::from_fn(|i| i as f32 / 48.),
                    },
                )
                .size(1440., 900.),
            ),
        ));
        ui.prepare_frame();
        let composer = root.find("composer-root").unwrap();
        let details = root.find("details").unwrap();
        let row = root.find("detail-0").unwrap();
        let initial = ui.scene.borrow().bounds(composer);
        assert_eq!(initial, crate::scene::Rect::new(441.5, 141.5, 594., 440.));
        let mut time = std::time::Instant::now();
        let mut index = 0;
        let mut advance = |ms: u64| {
            executor.borrow_mut().tick();
            index += 1;
            time += Duration::from_millis(ms);
            frames.deliver(Frame {
                index,
                time,
                interval: Duration::from_millis(ms),
            });
            executor.borrow_mut().tick();
            ui.prepare_frame();
        };
        selected.set(true);
        ui.prepare_frame();
        advance(0);
        advance(50);
        let before = ui.scene.borrow().bounds(composer);
        let before_details = ui.scene.borrow().layout_bounds(details);
        let before_row = ui.scene.borrow().bounds(row);
        let frame = frames.last().unwrap();
        assert!(before.height > initial.height && before.height < initial.height + 7.5);
        selected.set(false);
        ui.try_prepare_frame().unwrap();
        assert_eq!(frames.last().unwrap().index, frame.index);
        assert_eq!(frames.last().unwrap().time, frame.time);
        assert_eq!(root.find("detail-0"), Some(row));
        let same_bounds = |actual: crate::scene::Rect, expected: crate::scene::Rect| {
            for (actual, expected) in [
                (actual.x, expected.x),
                (actual.y, expected.y),
                (actual.width, expected.width),
                (actual.height, expected.height),
            ] {
                assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
            }
        };
        same_bounds(ui.scene.borrow().bounds(composer), before);
        same_bounds(ui.scene.borrow().layout_bounds(details), before_details);
        same_bounds(ui.scene.borrow().bounds(row), before_row);
        advance(0);
        same_bounds(ui.scene.borrow().bounds(composer), before);
        advance(500);
        same_bounds(ui.scene.borrow().bounds(composer), initial);
        assert_eq!(ui.scene.borrow().style(details).height, None);
        root.unmount();
        executor.borrow_mut().tick();
    }

    #[test]
    fn horizontal_scroll_allocates_fill_children_and_preserves_overflow() {
        let mut ui = Ui::new(240., 100.);
        let offset = ui.signal(0.);
        let root = ui.mount(
            scroll_x(offset.clone())
                .w(200.)
                .fit_cross_axis(true)
                .scrollbar(false)
                .gap(8.)
                .child(div().id("fixed").w(40.).h(32.).shrink_0())
                .child(div().id("fill").w(0.).grow().h(32.)),
        );
        ui.prepare_frame();
        assert!(
            (ui.scene
                .borrow()
                .layout_bounds(root.find("fill").unwrap())
                .width
                - 152.)
                .abs()
                < 0.01
        );
        root.unmount();
        let root = ui.mount(
            scroll_x(offset.clone())
                .w(200.)
                .fit_cross_axis(true)
                .scrollbar(false)
                .gap(8.)
                .child(div().w(140.).h(32.).shrink_0())
                .child(div().id("last").w(140.).h(32.).shrink_0()),
        );
        ui.prepare_frame();
        offset.set(1000.);
        ui.prepare_frame();
        assert!((offset.get() - 88.).abs() < 0.01);
        assert!((ui.scene.borrow().bounds(root.find("last").unwrap()).x - 60.).abs() < 0.01);
    }
    #[test]
    fn scroll_children_retain_identity_and_wheel_routing_across_variants() {
        use crate::{frame::FrameClock, input::InputEvent, task::LocalExecutor};
        let mut ui = Ui::new(240., 200.);
        let selected = ui.signal(false);
        let choice = selected.clone();
        let offset = ui.signal(0.);
        let read_offset = offset.clone();
        let root = ui.mount(provide(
            TaskRunner::from_executor(Rc::new(RefCell::new(LocalExecutor::new()))),
            provide(
                FrameClock::new(),
                compiled_morph(
                    move || choice.get(),
                    move |extra| {
                        let view = scroll_x(read_offset.clone())
                            .id("row")
                            .w(200.)
                            .fit_cross_axis(true)
                            .scrollbar(false)
                            .horizontal_wheel(true)
                            .gap(8.)
                            .child(div().id("first").w(140.).h(32.).shrink_0())
                            .child(button().id("second").w(140.).h(32.).shrink_0());
                        if extra {
                            view.child(div().id("third").w(140.).h(32.).shrink_0())
                        } else {
                            view
                        }
                    },
                    || CompiledMorphTiming {
                        duration: Duration::ZERO,
                        crossfade: Duration::ZERO,
                        color_duration: Duration::ZERO,
                        curve: std::array::from_fn(|i| i as f32 / 48.),
                    },
                ),
            ),
        ));
        ui.prepare_frame();
        let first = root.find("first").unwrap();
        assert!(
            (ui.scene
                .borrow()
                .layout_bounds(root.find("row").unwrap())
                .height
                - 32.)
                .abs()
                < 0.01
        );
        assert!((ui.scene.borrow().bounds(root.find("second").unwrap()).x - 148.).abs() < 0.01);
        ui.dispatch(InputEvent::Scroll {
            x: 20.,
            y: 16.,
            delta_x: 0.,
            delta_y: 50.,
        });
        ui.prepare_frame();
        assert_eq!(offset.get(), 50.);
        assert!((ui.scene.borrow().bounds(first).x + 50.).abs() < 0.01);
        selected.set(true);
        ui.prepare_frame();
        assert_eq!(root.find("first"), Some(first));
        assert_eq!(offset.get(), 50.);
        ui.dispatch(InputEvent::Scroll {
            x: 20.,
            y: 16.,
            delta_x: 0.,
            delta_y: 1000.,
        });
        ui.prepare_frame();
        assert!((offset.get() - 236.).abs() < 0.01);
        selected.set(false);
        ui.prepare_frame();
        assert!((offset.get() - 88.).abs() < 0.01);
    }
    #[test]
    fn image_sources_remain_reactive_and_rebind_on_variant_changes() {
        use crate::{frame::FrameClock, task::LocalExecutor};
        let mut ui = Ui::new(100., 100.);
        let selected = ui.signal(false);
        let first = ui.signal(20u8);
        let second = ui.signal(50u8);
        let choice = selected.clone();
        let a = first.clone();
        let b = second.clone();
        let root = ui.mount(provide(
            TaskRunner::from_executor(Rc::new(RefCell::new(LocalExecutor::new()))),
            provide(
                FrameClock::new(),
                compiled_morph(
                    move || choice.get(),
                    move |other| {
                        let value = if other { b.clone() } else { a.clone() };
                        image_signal("Live image", move || {
                            std::sync::Arc::new(
                                crate::image::ImageData::new(1, 1, vec![value.get(), 0, 0, 255])
                                    .unwrap(),
                            )
                        })
                        .id("image")
                        .size(10., 10.)
                    },
                    || CompiledMorphTiming {
                        duration: Duration::ZERO,
                        crossfade: Duration::ZERO,
                        color_duration: Duration::ZERO,
                        curve: std::array::from_fn(|i| i as f32 / 48.),
                    },
                ),
            ),
        ));
        let read = |ui: &mut Ui| {
            ui.prepare_frame();
            let scene = ui.scene.borrow();
            let mut nodes = vec![root.find("image").unwrap()];
            while let Some(node) = nodes.pop() {
                if let NodeKind::Image(data) = scene.kind(node) {
                    return data.pixels()[0];
                }
                nodes.extend_from_slice(scene.children(node));
            }
            panic!("expected image")
        };
        assert_eq!(read(&mut ui), 20);
        first.set(30);
        assert_eq!(read(&mut ui), 30);
        selected.set(true);
        assert_eq!(read(&mut ui), 50);
        second.set(60);
        assert_eq!(read(&mut ui), 60);
        first.set(40);
        assert_eq!(read(&mut ui), 60);
    }
    #[test]
    fn unchanged_destination_does_not_restart_motion_on_content_refresh() {
        use crate::{frame::FrameClock, task::LocalExecutor};
        let mut ui = Ui::new(600., 400.);
        let executor = Rc::new(RefCell::new(LocalExecutor::new()));
        let frames = FrameClock::new();
        let output = Rc::new(RefCell::new(None));
        let capture = output.clone();
        let _root = ui.mount(provide(
            TaskRunner::from_executor(executor),
            provide(
                frames,
                component(move |cx| {
                    *capture.borrow_mut() = Some(StyleTrack::new(cx, Styles::new().w(100.)));
                    div()
                }),
            ),
        ));
        let track = output.borrow().clone().unwrap();
        let timing = CompiledMorphTiming {
            duration: Duration::from_millis(400),
            crossfade: Duration::from_millis(140),
            color_duration: Duration::from_millis(200),
            curve: std::array::from_fn(|i| i as f32 / 48.),
        };
        track.update(Styles::new().w(300.), &timing, false);
        track.progress.set(0.4);
        let presented = track.sample();
        let generation = track.generation.get();
        track.update(Styles::new().w(300.), &timing, false);
        assert_eq!(track.progress.get(), 0.4);
        assert_eq!(track.generation.get(), generation);
        assert_eq!(track.sample(), presented);
        track.update(Styles::new().w(400.), &timing, false);
        assert!(track.generation.get() > generation);
    }

    #[test]
    fn variant_changes_retain_editor_focus_selection_and_cancel_a_pending_exit() {
        use crate::{
            frame::FrameClock,
            input::{InputEvent, Key},
            task::LocalExecutor,
        };
        let mut ui = Ui::new(600., 400.);
        let executor = Rc::new(RefCell::new(LocalExecutor::new()));
        let frames = FrameClock::new();
        let chosen = ui.signal(false);
        let draft = ui.signal("keep this draft".to_string());
        let handle = Rc::new(RefCell::new(None::<crate::widgets::EditorHandle>));
        let key = chosen.clone();
        let value = draft.clone();
        let output = handle.clone();
        let root = ui.mount(provide(
            TaskRunner::from_executor(executor.clone()),
            provide(
                frames.clone(),
                compiled_morph(
                    move || key.get(),
                    move |expanded| {
                        let value = value.clone();
                        let output = output.clone();
                        let mut view = column().id("composer").w(400.).gap(8.).child(
                            component(move |_| {
                                text_area("Reply", value)
                                    .id("native-editor")
                                    .h(40.)
                                    .on_editor(move |editor| *output.borrow_mut() = Some(editor))
                            })
                            .id("editor"),
                        );
                        if expanded {
                            view = view.child(
                                column()
                                    .id("approval")
                                    .h(40.)
                                    .child(text("Approve this action?").id("question")),
                            );
                        }
                        view
                    },
                    || CompiledMorphTiming {
                        duration: Duration::from_millis(120),
                        crossfade: Duration::from_millis(40),
                        color_duration: Duration::from_millis(40),
                        curve: std::array::from_fn(|i| i as f32 / 48.),
                    },
                ),
            ),
        ));
        ui.prepare_frame();
        let editor = handle.borrow().clone().unwrap();
        let editor_node = editor.node;
        editor.set_selection(5, 9);
        ui.input.focus(&ui.scene, Some(editor_node));
        chosen.set(true);
        ui.prepare_frame();
        let approval = root.find("approval").unwrap();
        assert_eq!(root.find("editor"), Some(editor_node));
        assert_eq!(ui.input.focused(), Some(editor_node));
        assert_eq!(editor.editor.borrow().selection().range(), 5..9);
        chosen.set(false);
        ui.prepare_frame();
        chosen.set(true);
        ui.prepare_frame();
        assert_eq!(root.find("approval"), Some(approval));
        ui.dispatch(InputEvent::KeyDown {
            key: Key::Character("x".into()),
            modifiers: Default::default(),
            repeat: false,
        });
        ui.dispatch(InputEvent::Text("x".into()));
        assert_eq!(draft.get(), "keep x draft");
        assert_eq!(root.find("editor"), Some(editor_node));
        assert_eq!(ui.input.focused(), Some(editor_node));
        root.unmount();
        executor.borrow_mut().tick();
    }
    #[test]
    fn interpolates_geometry_without_blending_structural_choices() {
        let a = Styles::new()
            .w(100.)
            .h(40.)
            .p(8.)
            .bg(crate::style::rgb(0x000000));
        let b = Styles::new()
            .w(200.)
            .h(80.)
            .p(16.)
            .bg(crate::style::rgb(0xffffff))
            .flex_col();
        let halfway = interpolate(&a, &b, 0.5);
        assert_eq!(halfway.width, Some(150.));
        assert_eq!(halfway.padding.unwrap().left, 12.);
        assert_eq!(halfway.background.unwrap().0, 128);
        assert_eq!(halfway.layout, Some(Layout::Column));
    }
    #[test]
    fn validates_all_variant_identities_before_mutation() {
        let mut out = vec![];
        let mut ids = HashSet::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            flatten(
                column()
                    .id("root")
                    .child(text("first").id("duplicate"))
                    .child(text("second").id("duplicate")),
                None,
                &mut out,
                &mut ids,
            )
        }));
        assert!(result.is_err());
    }
}
