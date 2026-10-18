//! Reconcile compiled native variants by permanent IDs. No design interpreter or browser runtime.
use super::*;
use crate::motion::{AnimationGroup, Completion, Keyframe, Keyframes, MotionValue, Playback};
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
    /// CSS pixels to logical pixels for imported source flow. `Some(1.)` is unzoomed.
    /// Applies destination gaps immediately and rounds footprint measurements like offsetWidth/Height.
    /// `None` preserves native fractional measurements and animated gaps.
    pub source_layout_scale: Option<f32>,
}
impl CompiledMorphTiming {
    fn layout_scale(&self) -> f32 {
        self.source_layout_scale.unwrap_or(1.)
    }
    fn measure(&self, logical: f32) -> f32 {
        self.source_layout_scale
            .map_or(logical, |scale| (logical / scale).round() * scale)
    }
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
    row_reveal: Signal<Option<f32>>,
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
            row_reveal: cx.state(None),
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
        let mut from = self.presented.borrow().clone();
        self.footprint.set(None);
        if from == target {
            return;
        }
        if timing.source_layout_scale.is_some() {
            // Source installs the destination shell before reconciling child footprints.
            from.gap = target.gap;
            if let (Some(from), Some(to)) = (&mut from.layout_options, &target.layout_options) {
                let from = std::sync::Arc::make_mut(from);
                from.gap_x = to.gap_x;
                from.gap_y = to.gap_y;
            }
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
                if let Some(offset) = self.row_reveal.get() {
                    sampled.transform = Some(crate::scene::Transform {
                        x: 0.,
                        y: offset * (1. - self.reveal_opacity.get()),
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
    track.row_reveal.set(None);
    let horizontal = layout == Layout::Row;
    let vertical = layout == Layout::Column;
    let mut open = original.clone();
    let mut margin = original.margin.unwrap_or_default();
    open.margin = Some(margin);
    open.margin_sides = 15;
    let leading = if horizontal { margin.left } else { margin.top };
    let size = timing.measure(if horizontal {
        bounds.width
    } else {
        bounds.height
    });
    // Keep the complete content allocation. Only its contribution to flow changes.
    if horizontal {
        open.width = Some(if original.flex_grow.unwrap_or(0.) > 0. {
            timing.measure(bounds.width)
        } else {
            bounds.width
        });
    } else if vertical {
        open.height = Some(if original.flex_grow.unwrap_or(0.) > 0. {
            timing.measure(bounds.height)
        } else {
            bounds.height
        });
    }
    if horizontal || vertical {
        open.flex_grow = Some(0.);
        open.flex_shrink = Some(0.);
    }
    open.clip_insets = Some(Some(crate::scene::Insets::all(
        -20. * timing.layout_scale(),
    )));
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
            right: open.width.unwrap_or(bounds.width),
            ..Default::default()
        }
    } else if vertical {
        crate::scene::Insets {
            bottom: open.height.unwrap_or(bounds.height),
            ..Default::default()
        }
    } else {
        crate::scene::Insets::default()
    }));
    shut.transform = Some(crate::scene::Transform {
        x: 0.,
        y: if horizontal {
            0.
        } else {
            6. * timing.layout_scale()
        },
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
        open.height = Some(timing.measure(bounds.height));
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
        from: if enter {
            shut.clone()
        } else if timing.source_layout_scale.is_some() {
            // Source starts a new exit effect over the current flow margins.
            open.clone()
        } else {
            leaving
        },
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
    source: Rc<RefCell<TextSource>>,
    revision: Signal<u64>,
    timing: Rc<RefCell<CompiledMorphTiming>>,
}
type TextSource = Box<dyn FnMut() -> DisplayText>;
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
        Kind::Text(_) | Kind::RichText(_) => 1,
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
fn text_source(kind: &mut Kind) -> TextSource {
    match kind {
        Kind::Text(text) => {
            let text = text.clone();
            Box::new(move || DisplayText::Plain(text.clone()))
        }
        Kind::RichText(read) => {
            let mut read = std::mem::replace(read, Box::new(Vec::new));
            Box::new(move || DisplayText::Rich(read()))
        }
        _ => unreachable!("compiled text source"),
    }
}
// A ghost belongs to its label's lifetime but paints beside its shrinking clip.
struct TextGhost {
    ui: WeakUi,
    node: Signal<Option<NodeId>>,
    base_opacity: Cell<f32>,
}
impl TextGhost {
    fn clear(&self) {
        let node = self.node.with_untracked(|node| *node);
        if node.is_some() {
            self.node.set(None);
        }
        if let Some(node) = node
            && let Some(mut ui) = self.ui.upgrade()
        {
            ui.remove(node);
        }
    }
    fn snapshot(&self, owner: NodeId, paragraph: NodeId) {
        self.clear();
        let Some(mut ui) = self.ui.upgrade() else {
            return;
        };
        let (
            parent,
            owner_bounds,
            parent_bounds,
            glyph_bounds,
            owner_style,
            glyph_style,
            owner_effects,
            paragraph_opacity,
            mut glyph_effects,
            kind,
            font,
            transform,
            paint,
            origin,
            projection,
        ) = {
            let scene = ui.scene.borrow();
            let Some(parent) = scene.parent(owner) else {
                return;
            };
            let glyph = scene.children(paragraph)[0];
            (
                parent,
                scene.layout_bounds(owner),
                scene.layout_bounds(parent),
                scene.layout_bounds(glyph),
                scene.style(owner),
                scene.style(glyph),
                scene.effects(owner),
                scene.effects(paragraph).opacity,
                scene.effects(glyph),
                scene.kind(glyph).clone(),
                scene.font(glyph).clone(),
                scene.transform(owner),
                scene.paint_transform(owner),
                scene.paint_transform_origin(owner),
                scene.projection_transform(owner),
            )
        };
        let positioned = |bounds: crate::scene::Rect, x, y| crate::scene::Style {
            absolute: true,
            width: Some(bounds.width),
            height: Some(bounds.height),
            layout_options: Some(std::sync::Arc::new(crate::layout::LayoutOptions {
                inset: crate::layout::EdgeLengths {
                    left: Some(crate::layout::Length::Px(x)),
                    top: Some(crate::layout::Length::Px(y)),
                    ..Default::default()
                },
                ..Default::default()
            })),
            ..Default::default()
        };
        let mut style = positioned(
            owner_bounds,
            owner_bounds.x - parent_bounds.x,
            owner_bounds.y - parent_bounds.y,
        );
        style.clip = owner_style.clip;
        style.clip_insets = owner_style.clip_insets;
        style.z_index = owner_style.z_index;
        if let Some(options) = owner_style.layout_options {
            let target = std::sync::Arc::make_mut(style.layout_options.as_mut().unwrap());
            target.clip_x = options.clip_x;
            target.clip_y = options.clip_y;
            target.visible = options.visible;
            target.display = options.display;
        }
        let ghost = ui.container(parent, Layout::Overlay, style);
        let mut style = positioned(
            glyph_bounds,
            glyph_bounds.x - owner_bounds.x,
            glyph_bounds.y - owner_bounds.y,
        );
        style.text_wrap = glyph_style.text_wrap;
        style.text_options = glyph_style.text_options;
        let text = ui.scene.borrow_mut().append(ghost, kind, style);
        {
            let mut scene = ui.scene.borrow_mut();
            scene.set_font(text, font);
            scene.set_effects(ghost, owner_effects);
            // The internal paragraph contributes enter opacity; keep nonlinear leaf effects local.
            glyph_effects.opacity *= paragraph_opacity;
            scene.set_effects(text, glyph_effects);
            scene.set_transform(ghost, transform);
            scene.set_paint_transform_origin(ghost, paint, origin);
            scene.set_projection_transform(ghost, projection);
        }
        ui.input.set_pointer_transparent(ghost, true);
        ui.input.set_pointer_transparent(text, true);
        self.base_opacity.set(owner_effects.opacity);
        self.node.set(Some(ghost));
    }
}
impl Drop for TextGhost {
    fn drop(&mut self) {
        self.clear();
    }
}

fn content(
    ui: &mut Ui,
    cx: &mut Context,
    view: &mut View,
    timing: &CompiledMorphTiming,
) -> ContentTrack {
    match &mut view.kind {
        kind @ (Kind::Text(_) | Kind::RichText(_)) => {
            let source = Rc::new(RefCell::new(text_source(kind)));
            let revision = cx.state(0u64);
            let timing = Rc::new(RefCell::new(timing.clone()));
            let owner = Rc::new(Cell::new(None));
            let paragraph_node = Rc::new(Cell::new(None));
            let ghost = Rc::new(TextGhost {
                ui: ui.downgrade(),
                node: cx.state(None),
                base_opacity: Cell::new(1.),
            });
            let enter = MotionValue::new(cx, 1.);
            let leave = MotionValue::new(cx, 1.);
            let ghost_owner = ghost.clone();
            let mounted_owner = owner.clone();
            let paint_opacity = leave.clone();
            view.mounted.push(Box::new(move |node| {
                mounted_owner.set(Some(node));
                let Some(mut ui) = ghost_owner.ui.upgrade() else {
                    return;
                };
                let paint = ghost_owner.clone();
                ui.bind(node, move || {
                    let ghost_node = paint.node.get();
                    let opacity = paint_opacity.get();
                    if opacity >= 1. {
                        paint.clear();
                    } else if let Some(node) = ghost_node
                        && let Some(ui) = paint.ui.upgrade()
                    {
                        let mut scene = ui.scene.borrow_mut();
                        let mut effects = scene.effects(node);
                        effects.opacity = paint.base_opacity.get() * (1. - opacity);
                        scene.set_effects(node, effects);
                    }
                });
                ui.retain(node, ghost_owner);
            }));
            let read = source.clone();
            let changed = revision.clone();
            let options = timing.clone();
            let old_owner = owner.clone();
            let old_paragraph = paragraph_node.clone();
            let enter_write = enter.clone();
            let leave_write = leave.clone();
            let mut current = None;
            let paragraph = View::new(Kind::DisplayText(Box::new(move || {
                let _ = changed.get();
                let next = (read.borrow_mut())();
                if let Some(old) = current.replace(next.clone())
                    && old != next
                {
                    let timing = options.borrow();
                    if timing.crossfade.is_zero() {
                        ghost.clear();
                    } else if let (Some(owner), Some(paragraph)) =
                        (old_owner.get(), old_paragraph.get())
                    {
                        ghost.snapshot(owner, paragraph);
                    }
                    leave_write.set(0.);
                    enter_write.set(0.);
                    leave_write.animate_to(
                        1.,
                        crate::motion::Transition::tween(
                            timing.crossfade,
                            crate::motion::Easing::EaseOut,
                        ),
                    );
                    enter_write.animate_to(
                        1.,
                        crate::motion::Transition::tween(
                            timing.crossfade.mul_f32(1.3),
                            crate::motion::Easing::EaseOut,
                        )
                        .delay(timing.crossfade.mul_f32(0.2)),
                    );
                }
                next
            })))
            .h_full();
            let opacity = enter.clone();
            view.kind = Kind::Component(Box::new(move |_| {
                overlay().child(
                    paragraph
                        .on_mount(move |node| paragraph_node.set(Some(node)))
                        .reactive_style(move || Styles::new().opacity(opacity.get())),
                )
            }));
            ContentTrack::Text(TextTrack {
                source,
                revision,
                timing,
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
        (ContentTrack::Text(track), kind @ (Kind::Text(_) | Kind::RichText(_))) => {
            *track.source.borrow_mut() = text_source(kind);
            *track.timing.borrow_mut() = timing.clone();
            track
                .revision
                .update(|revision| *revision = revision.wrapping_add(1));
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
        let retiring = Rc::new(RefCell::new(HashSet::<NodeId>::new()));
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
            assert!(
                options
                    .source_layout_scale
                    .is_none_or(|scale| scale.is_finite() && scale > 0.)
            );
            let first = previous.is_none();
            runtime.untracked(|| {
                let mut live = mounted.borrow_mut();
                retiring
                    .borrow_mut()
                    .retain(|node| ui.scene.borrow().contains(*node));
                if options.source_layout_scale.is_some() {
                    // Source reconcile excludes m-exit nodes and mounts a new copy on reentry.
                    let returning: Vec<_> = live
                        .iter()
                        .filter(|(id, entry)| ids.contains(*id) && entry.style.exiting.get())
                        .map(|(_, entry)| entry.node)
                        .collect();
                    for node in returning {
                        retiring.borrow_mut().insert(node);
                        let scene = ui.scene.borrow();
                        // Components can register descendants absent from the flattened specs.
                        environment.registry.borrow_mut().retain(|_, nodes| {
                            nodes.retain(|n| *n != node && !scene.ancestors(*n).any(|p| p == node));
                            !nodes.is_empty()
                        });
                        live.retain(|_, entry| {
                            entry.node != node && !scene.ancestors(entry.node).any(|p| p == node)
                        });
                    }
                }
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
                    let content = content(&mut ui, &mut cx, &mut spec.view, &options);
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
                        let parent_target = options.source_layout_scale.and_then(|_| {
                            spec.parent.as_ref().map(|id| {
                                live[id]
                                    .style
                                    .endpoints
                                    .with_untracked(|(_, to)| to.clone())
                            })
                        });
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
                            let (layout, gap) = parent_target
                                .as_ref()
                                .map_or((layout, scene.style(parent).gap), |style| {
                                    source_flow(style, layout)
                                });
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
                                entering.row_reveal.set(Some(3. * timing.layout_scale()));
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
                    let (layout, gap) = if options.source_layout_scale.is_some() {
                        old.parent.as_ref().and_then(|id| live.get(id)).map_or(
                            (layout, scene.style(parent).gap),
                            |parent| {
                                parent
                                    .style
                                    .endpoints
                                    .with_untracked(|(_, to)| source_flow(to, layout))
                            },
                        )
                    } else {
                        (layout, scene.style(parent).gap)
                    };
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
                        // offsetWidth/Height exclude the current paint scale and translation.
                        let flow_bounds = if options.source_layout_scale.is_some() {
                            ui.scene.borrow().layout_bounds(old.node)
                        } else {
                            bounds
                        };
                        footprint(
                            &old.style,
                            from,
                            flow_bounds,
                            layout,
                            gap,
                            neighbour,
                            false,
                            &options,
                        );
                    }
                    let weak = ui.downgrade();
                    let nodes = mounted.clone();
                    let retiring = retiring.clone();
                    let node = old.node;
                    let exit = if animation.is_some() {
                        AnimationGroup::new([old.style.decoration_progress.animation()])
                    } else {
                        AnimationGroup::new([
                            old.style.progress.animation(),
                            old.style.reveal_opacity.animation(),
                        ])
                    };
                    let mut cx = Context::new(runtime.clone(), environment.clone());
                    cx.tasks().spawn(async move {
                        if exit.finished().await != Completion::Finished
                            || marker.get() != generation
                        {
                            return;
                        }
                        let Some(mut ui) = weak.upgrade() else {
                            return;
                        };
                        let mut live = nodes.borrow_mut();
                        if !ui.scene.borrow().contains(node) {
                            return;
                        }
                        let subtree: Vec<_> = live
                            .iter()
                            .filter(|(_, entry)| {
                                entry.node == node
                                    || ui.scene.borrow().ancestors(entry.node).any(|p| p == node)
                            })
                            .map(|(id, _)| id.clone())
                            .collect();
                        ui.remove(node);
                        retiring.borrow_mut().remove(&node);
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
                        .chain(
                            retiring
                                .borrow()
                                .iter()
                                .copied()
                                .filter(|node| ui.scene.borrow().parent(*node) == Some(parent)),
                        )
                        .collect();
                    let is_exiting = |node| {
                        retiring.borrow().contains(&node)
                            || live
                                .values()
                                .any(|entry| entry.node == node && entry.style.exiting.get())
                    };
                    let mut sorted: Vec<_> = current
                        .iter()
                        .copied()
                        .filter(|id| !managed.contains(id))
                        .collect();
                    // Exiting prefix slots must stay ahead of the first surviving sibling.
                    for &node in &current {
                        if !managed.contains(&node) {
                            continue;
                        }
                        if !is_exiting(node) {
                            break;
                        }
                        sorted.push(node);
                    }
                    // Keep disappearing slots next to their former preceding sibling.
                    for node in wanted {
                        sorted.push(node);
                        if let Some(i) = current.iter().position(|old| *old == node) {
                            for &after in &current[i + 1..] {
                                if !managed.contains(&after) {
                                    continue;
                                }
                                if is_exiting(after) {
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

fn source_flow(style: &Styles, fallback: Layout) -> (Layout, f32) {
    let layout = style.layout.unwrap_or(fallback);
    let axis_gap = style.layout_options.as_ref().and_then(|options| {
        if layout == Layout::Row {
            options.gap_x
        } else {
            options.gap_y
        }
    });
    let gap = match axis_gap {
        Some(crate::layout::Length::Px(value)) => value,
        _ => style.gap.unwrap_or(0.),
    };
    (layout, gap)
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
    fn compiled_text_applies_wrapping_only_changes_without_replacing_paragraphs() {
        for limit_only in [false, true] {
            let mut ui = Ui::new(400., 200.);
            let wrap = ui.signal(false);
            let selected = wrap.clone();
            let root = ui.mount(compiled_morph(
                move || selected.get(),
                move |wrap| {
                    let view = text("Alpha beta gamma delta")
                        .id("copy")
                        .text_size(10.)
                        .line_height(20.)
                        .text_wrap(wrap);
                    if limit_only {
                        view.max_w(60.)
                    } else {
                        view.w(60.)
                    }
                },
                || CompiledMorphTiming {
                    source_layout_scale: None,
                    duration: Duration::ZERO,
                    crossfade: Duration::ZERO,
                    color_duration: Duration::ZERO,
                    curve: std::array::from_fn(|i| i as f32 / 48.),
                },
            ));
            ui.prepare_frame();
            let owner = root.find("copy").unwrap();
            let paragraph = {
                let scene = ui.scene.borrow();
                scene.children(scene.children(owner)[0])[0]
            };
            assert!(!ui.scene.borrow().style(paragraph).text_wrap);
            assert_eq!(ui.scene.borrow().layout_bounds(owner).height, 20.);
            assert!(ui.scene.borrow().layout_bounds(paragraph).width > 60.);
            for value in [true, false, true] {
                wrap.set(value);
                ui.prepare_frame();
                let scene = ui.scene.borrow();
                assert_eq!(root.find("copy"), Some(owner));
                assert_eq!(scene.children(scene.children(owner)[0])[0], paragraph);
                assert_eq!(scene.style(paragraph).text_wrap, value);
                let height = scene.layout_bounds(owner).height;
                if value {
                    assert!(
                        height > 20.,
                        "owner={:?}, paragraph={:?}",
                        scene.layout_bounds(owner),
                        scene.layout_bounds(paragraph)
                    );
                } else {
                    assert_eq!(height, 20.);
                    assert!(scene.layout_bounds(paragraph).width > 60.);
                }
            }
        }
    }

    #[test]
    fn compiled_text_retains_paragraphs_across_plain_rich_decorations_and_reactive_runs() {
        use crate::{rich_text::Decoration, scene::Color, text_layout::WordWrap};
        let mut ui = Ui::new(400., 200.);
        let phase = ui.signal(0);
        let selected = phase.clone();
        let word = ui.signal("beta".to_owned());
        let read_word = word.clone();
        let root = ui.mount(compiled_morph(
            move || selected.get(),
            move |phase| {
                let view = match phase {
                    0 | 5 => text("Alpha beta"),
                    1..=3 => {
                        let (offset, thickness, color) = match phase {
                            1 => (4., 2., Color(255, 0, 0, 255)),
                            2 => (-2., 1., Color(0, 0, 255, 255)),
                            _ => (3., 0., Color(0, 255, 0, 255)),
                        };
                        rich_text()
                            .child(
                                text_span("Alpha beta").underline(
                                    Decoration::default()
                                        .with_baseline_offset(offset)
                                        .with_thickness(thickness)
                                        .color(color),
                                ),
                            )
                            .into()
                    }
                    _ => {
                        let read_word = read_word.clone();
                        rich_text_signal(move || {
                            vec![
                                text_span("Alpha "),
                                text_span(read_word.get())
                                    .font_weight(700)
                                    .text_size(20.)
                                    .line_height(30.)
                                    .background(Color(20, 30, 40, 255))
                                    .strikethrough(Decoration::new(2.)),
                            ]
                        })
                    }
                };
                view.id("copy")
                    .w(160.)
                    .text_size(10.)
                    .line_height(20.)
                    .text_wrap(true)
                    .word_wrap(WordWrap::Word)
            },
            || CompiledMorphTiming {
                source_layout_scale: None,
                duration: Duration::ZERO,
                crossfade: Duration::ZERO,
                color_duration: Duration::ZERO,
                curve: std::array::from_fn(|i| i as f32 / 48.),
            },
        ));
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        let owner = root.find("copy").unwrap();
        let paragraph = {
            let scene = ui.scene.borrow();
            scene.children(scene.children(owner)[0])[0]
        };
        let initial = ui.scene.borrow().layout_bounds(paragraph);
        assert!(matches!(
            ui.scene.borrow().kind(paragraph),
            NodeKind::Text { .. }
        ));
        for next in 1..=5 {
            phase.set(next);
            ui.prepare_frame();
            let frame = ui.scene.borrow_mut().flush();
            assert!(!frame.damage.is_empty());
            if next == 3 {
                assert_eq!(
                    frame.layout_nodes, 0,
                    "decoration-only updates preserve layout"
                );
            }
            assert_eq!(root.find("copy"), Some(owner));
            let scene = ui.scene.borrow();
            let children = scene.children(owner);
            assert_eq!(scene.children(children[0])[0], paragraph);
            assert_eq!(children.len(), 1);
            let bounds = scene.layout_bounds(owner);
            assert_eq!(bounds.width, 160.);
            assert_eq!(bounds.height, if next == 4 { 30. } else { 20. });
            match scene.kind(paragraph) {
                NodeKind::Text { text, .. } => {
                    assert_eq!(next, 5);
                    assert_eq!(text.as_ref(), "Alpha beta");
                    assert_eq!(scene.layout_bounds(paragraph), initial);
                }
                NodeKind::RichText { text } => {
                    assert_eq!(text.text(), "Alpha beta");
                    if next < 4 {
                        assert_eq!(text.runs().len(), 1);
                        let decoration = text.runs()[0].underline.unwrap();
                        assert_eq!(
                            decoration.baseline_offset(),
                            Some([4., -2., 3.][next as usize - 1])
                        );
                        assert_eq!(decoration.thickness(), [2., 1., 0.][next as usize - 1]);
                        assert_eq!(scene.layout_bounds(paragraph), initial);
                    } else {
                        assert_eq!(text.runs().len(), 2);
                        assert_eq!(text.runs()[1].font_size, 20.);
                        assert_eq!(text.runs()[1].font.weight, 700);
                        assert!(text.runs()[1].strikethrough.is_some());
                    }
                }
                _ => panic!("retained native text paragraph"),
            }
            drop(scene);
            if next == 4 {
                word.set("zeta".to_owned());
                ui.prepare_frame();
                let scene = ui.scene.borrow();
                let NodeKind::RichText { text } = scene.kind(paragraph) else {
                    panic!("rich paragraph")
                };
                assert_eq!(text.text(), "Alpha zeta");
                assert_eq!(text.runs()[1].font.weight, 700);
                assert_eq!(scene.layout_bounds(owner).height, 30.);
            }
        }
        root.unmount();
        word.set("disposed".to_owned());
        assert!(!ui.scene.borrow().contains(paragraph));
    }

    #[test]
    fn frame_clock_exit_completion_removes_zero_height_slots_and_their_gaps() {
        use crate::{
            frame::{Frame, FrameClock},
            task::LocalExecutor,
        };
        for (line_height, gap, count, authored_exit) in [(30., 6., 4, true), (39., 9., 5, false)] {
            let mut ui = Ui::new(200., 300.);
            let executor = Rc::new(RefCell::new(LocalExecutor::new()));
            let frames = FrameClock::new();
            let selected = ui.signal(true);
            let choice = selected.clone();
            let root = ui.mount(provide(
                TaskRunner::from_executor(executor.clone()),
                provide(
                    frames.clone(),
                    compiled_morph(
                        move || choice.get(),
                        move |expanded| {
                            let mut view = column()
                                .id("stack")
                                .w(100.)
                                .gap(gap)
                                .child(div().id("first").h(line_height))
                                .child(div().id("second").h(line_height));
                            if expanded {
                                for i in 0..count {
                                    let mut slot = div().id(format!("exit-{i}")).h(0.);
                                    if authored_exit {
                                        slot = slot.morph_effect(CompiledMorphEffect {
                                            exit: Some(CompiledMorphAnimation {
                                                duration: Duration::from_millis(120),
                                                delay: Duration::ZERO,
                                                curve: std::array::from_fn(|i| i as f32 / 48.),
                                                offset_y: 0.,
                                                scale: 1.,
                                                blur: 0.,
                                            }),
                                            ..Default::default()
                                        });
                                    }
                                    view = view.child(slot);
                                }
                            }
                            view
                        },
                        || CompiledMorphTiming {
                            source_layout_scale: None,
                            duration: Duration::from_millis(120),
                            crossfade: Duration::from_millis(40),
                            color_duration: Duration::from_millis(40),
                            curve: std::array::from_fn(|i| i as f32 / 48.),
                        },
                    ),
                ),
            ));
            ui.prepare_frame();
            let stack = root.find("stack").unwrap();
            let first = root.find("first").unwrap();
            let second = root.find("second").unwrap();
            let exits: Vec<_> = (0..count)
                .map(|i| {
                    (
                        format!("exit-{i}"),
                        root.find(&format!("exit-{i}")).unwrap(),
                    )
                })
                .collect();
            let settled_height = 2. * line_height + gap;
            assert_eq!(
                ui.scene.borrow().layout_bounds(stack).height,
                settled_height + count as f32 * gap
            );
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
                executor.borrow_mut().tick();
                ui.prepare_frame();
            };
            selected.set(false);
            advance(50);
            for (id, node) in &exits {
                assert_eq!(root.find(id), Some(*node), "{id} removed before completion");
            }
            selected.set(true);
            advance(800);
            for (id, node) in &exits {
                assert_eq!(
                    root.find(id),
                    Some(*node),
                    "{id} removed by a cancelled exit"
                );
            }
            selected.set(false);
            advance(800);
            for (id, node) in &exits {
                assert_eq!(root.find(id), None, "{id} retained after frame completion");
                assert!(!ui.scene.borrow().contains(*node));
            }
            let scene = ui.scene.borrow();
            assert_eq!(scene.children(stack), &[first, second]);
            assert_eq!(scene.layout_bounds(stack).height, settled_height);
            assert_eq!(
                scene.layout_bounds(second).y - scene.layout_bounds(first).y,
                line_height + gap
            );
            drop(scene);
            root.unmount();
            executor.borrow_mut().tick();
        }
    }

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
                        source_layout_scale: None,
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
                        source_layout_scale: None,
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
                            .gap(if expanded { 21. } else { 12. })
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
                        source_layout_scale: Some(1.),
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
        assert_eq!(ui.scene.borrow().style(root.find("root").unwrap()).gap, 21.);
        let entry = root.find("entry").unwrap();
        assert_eq!(ui.scene.borrow().style(entry).margin.bottom, -69.);
        let copy = root.find("copy").unwrap();
        let full_copy = ui.scene.borrow().layout_bounds(copy);
        assert_eq!(ui.scene.borrow().layout_bounds(entry).height, 48.);
        assert!((ui.scene.borrow().layout_bounds(after).y - initial_y - 9.).abs() < 0.01);
        assert!(
            (ui.scene
                .borrow()
                .layout_bounds(root.find("root").unwrap())
                .height
                - initial_height
                - 9.)
                .abs()
                < 0.01,
            "only the surviving gap changes immediately, the entering gap stays cancelled"
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
            executor.borrow_mut().tick();
            ui.prepare_frame();
        };
        advance(&mut ui, 200);
        let middle = ui.scene.borrow().layout_bounds(after).y;
        assert!(
            middle > initial_y + 9. && middle < initial_y + 78.,
            "reveal must animate its flow slot: {middle}"
        );
        assert_eq!(ui.scene.borrow().layout_bounds(entry).height, 48.);
        let middle_copy = ui.scene.borrow().layout_bounds(copy);
        assert_eq!(
            (middle_copy.width, middle_copy.height),
            (full_copy.width, full_copy.height)
        );
        assert!(ui.scene.borrow().style(entry).clip_insets.unwrap().bottom > 0.);
        let before_exit = ui.scene.borrow().style(entry).clone();
        selected.set(false);
        ui.prepare_frame();
        assert_eq!(ui.scene.borrow().style(root.find("root").unwrap()).gap, 12.);
        let exit_start = ui.scene.borrow().style(entry).clone();
        // Source footprint() restarts exit paint from open, retaining the current flow margin.
        assert_eq!(
            exit_start.clip_insets,
            Some(crate::scene::Insets::all(-20.))
        );
        assert_eq!(exit_start.margin.bottom, before_exit.margin.bottom);
        advance(&mut ui, 80);
        let leaving = ui.scene.borrow().style(entry).clone();
        let clip_before = exit_start.clip_insets.unwrap().bottom;
        let exit_progress =
            (leaving.clip_insets.unwrap().bottom - clip_before) / (48. - clip_before);
        assert!(exit_progress > 0. && exit_progress < 1.);
        let margin_before = before_exit.margin.bottom;
        let expected_margin = margin_before + (-48. - 12. - margin_before) * exit_progress;
        assert!(
            (leaving.margin.bottom - expected_margin).abs() < 0.01,
            "exit collapse must use the destination gap before its scene binding runs"
        );
        selected.set(true);
        ui.prepare_frame();
        // Source reconcile() excludes m-exit parts, so the old copy keeps leaving beside a fresh entry.
        let fresh_entry = root.find("entry").unwrap();
        let fresh_copy = root.find("copy").unwrap();
        assert_ne!(fresh_entry, entry);
        assert_ne!(fresh_copy, copy);
        assert!(ui.scene.borrow().contains(entry));
        let fresh_bounds = ui.scene.borrow().layout_bounds(fresh_copy);
        assert_eq!(
            (fresh_bounds.width, fresh_bounds.height),
            (full_copy.width, full_copy.height)
        );
        advance(&mut ui, 500);
        assert!(!ui.scene.borrow().contains(entry));
        assert!(!ui.scene.borrow().contains(copy));
        assert_eq!(root.find("entry"), Some(fresh_entry));
        assert_eq!(root.find("copy"), Some(fresh_copy));
        assert_eq!(ui.scene.borrow().layout_bounds(fresh_entry).height, 48.);
        assert!((ui.scene.borrow().layout_bounds(after).y - initial_y - 78.).abs() < 0.01);
        assert_eq!(ui.scene.borrow().style(fresh_entry).clip_insets, None);
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
                        source_layout_scale: None,
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
        let initial = ui.scene.borrow().bounds(composer);
        assert_eq!(initial, crate::scene::Rect::new(441.5, 141.5, 594., 440.));
        let parts = [
            "composer-root",
            "details",
            "detail-0",
            "detail-3",
            "detail-4",
            "value-0",
            "value-3",
            "value-4",
        ]
        .map(|id| (id, root.find(id).unwrap()));
        let snapshot = || {
            let scene = ui.scene.borrow();
            parts
                .iter()
                .map(|&(id, node)| (id, node, scene.layout_bounds(node), scene.bounds(node)))
                .collect::<Vec<_>>()
        };
        let same_parts =
            |phase: &str, expected: &[(&str, NodeId, crate::scene::Rect, crate::scene::Rect)]| {
                let scene = ui.scene.borrow();
                for &(id, node, layout, paint) in expected {
                    assert_eq!(root.find(id), Some(node), "{phase}/{id}/identity");
                    for (kind, actual, expected) in [
                        ("layout", scene.layout_bounds(node), layout),
                        ("paint", scene.bounds(node), paint),
                    ] {
                        for (axis, actual, expected) in [
                            ("x", actual.x, expected.x),
                            ("y", actual.y, expected.y),
                            ("width", actual.width, expected.width),
                            ("height", actual.height, expected.height),
                        ] {
                            assert!(
                                (actual - expected).abs() < 0.001,
                                "{phase}/{id}/{kind}/{axis}: {actual} != {expected}"
                            );
                        }
                    }
                }
            };
        let initial_parts = snapshot();
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
        same_parts("forward-retarget", &initial_parts);
        advance(0);
        advance(50);
        let before = ui.scene.borrow().bounds(composer);
        let before_parts = snapshot();
        let frame = frames.last().unwrap();
        assert!(before.height > initial.height && before.height < initial.height + 7.5);
        selected.set(false);
        ui.try_prepare_frame().unwrap();
        assert_eq!(frames.last().unwrap().index, frame.index);
        assert_eq!(frames.last().unwrap().time, frame.time);
        same_parts("reverse-retarget", &before_parts);
        advance(0);
        same_parts("reverse-same-time-frame", &before_parts);
        advance(500);
        same_parts("reverse-settled", &initial_parts);
        assert_eq!(ui.scene.borrow().style(details).height, None);
        root.unmount();
        executor.borrow_mut().tick();
    }

    #[test]
    fn source_flow_reversal_keeps_exiting_copy_until_completion() {
        use crate::{
            frame::{Frame, FrameClock},
            task::LocalExecutor,
        };
        let mut ui = Ui::new(300., 200.);
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
                    |away| {
                        let mut rows = column()
                            .id("rows")
                            .absolute()
                            .left(0.)
                            .top(0.)
                            .w(300.)
                            .gap(6.);
                        if !away {
                            rows = rows.child(
                                column().id("returning").h(30.).child(
                                    component(|_| {
                                        column().child(div().id("returning-internal").h(30.))
                                    })
                                    .id("returning-child")
                                    .h(30.),
                                ),
                            );
                        }
                        rows.child(div().id("survivor").h(30.))
                    },
                    || CompiledMorphTiming {
                        source_layout_scale: Some(1.5),
                        duration: Duration::from_millis(400),
                        crossfade: Duration::from_millis(140),
                        color_duration: Duration::from_millis(200),
                        curve: std::array::from_fn(|i| i as f32 / 48.),
                    },
                )
                .size(300., 200.),
            ),
        ));
        ui.prepare_frame();
        let rows = root.find("rows").unwrap();
        let old = root.find("returning").unwrap();
        let old_child = root.find("returning-child").unwrap();
        let old_internal = root.find("returning-internal").unwrap();
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
            executor.borrow_mut().tick();
            ui.prepare_frame();
        };
        selected.set(true);
        ui.prepare_frame();
        advance(0);
        advance(50);
        let before = ui.scene.borrow().layout_bounds(rows).height;
        assert!((before - 61.5).abs() < 0.001);
        selected.set(false);
        ui.prepare_frame();
        let fresh = root.find("returning").unwrap();
        let fresh_child = root.find("returning-child").unwrap();
        let fresh_internal = root.find("returning-internal").unwrap();
        assert_ne!(fresh, old);
        assert_ne!(fresh_child, old_child);
        assert_ne!(fresh_internal, old_internal);
        assert!(
            ui.scene
                .borrow()
                .ancestors(fresh_internal)
                .any(|p| p == fresh)
        );
        assert!(ui.scene.borrow().contains(old));
        assert!(ui.scene.borrow().contains(old_internal));
        assert!((ui.scene.borrow().layout_bounds(rows).height - before).abs() < 0.001);
        advance(0);
        advance(50);
        // The old copy has exited for 100ms, while the new copy has entered for 50ms.
        // Their flow contributions are 36 * .75 and 36 * .125, plus the 30px survivor.
        assert!((ui.scene.borrow().layout_bounds(rows).height - 61.5).abs() < 0.001);
        assert_eq!(ui.scene.borrow().children(rows).len(), 3);
        advance(300);
        assert!(!ui.scene.borrow().contains(old));
        assert!(!ui.scene.borrow().contains(old_child));
        assert!(!ui.scene.borrow().contains(old_internal));
        assert_eq!(root.find("returning"), Some(fresh));
        assert_eq!(root.find("returning-child"), Some(fresh_child));
        assert_eq!(root.find("returning-internal"), Some(fresh_internal));
        assert!(ui.scene.borrow().contains(fresh));
        advance(50);
        assert_eq!(ui.scene.borrow().children(rows).len(), 2);
        assert!((ui.scene.borrow().layout_bounds(rows).height - 66.).abs() < 0.001);
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
                        source_layout_scale: None,
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
    fn outgoing_text_keeps_old_paint_bounds_and_reanchors_on_retarget() {
        use crate::{
            frame::{Frame, FrameClock},
            scene::Rect,
            task::LocalExecutor,
        };
        use std::time::Instant;
        let mut ui = Ui::new(800., 300.);
        let executor = Rc::new(RefCell::new(LocalExecutor::new()));
        let frames = FrameClock::new();
        let selected = ui.signal(false);
        let read = selected.clone();
        let root = ui.mount(provide(
            TaskRunner::from_executor(executor.clone()),
            provide(
                frames.clone(),
                compiled_morph(
                    move || read.get(),
                    |short| {
                        column()
                            .id("parent")
                            .w(600.)
                            .h(100.)
                            .p(10.)
                            .overflow_hidden()
                            .child(
                                text(if short {
                                    "Saved card"
                                } else {
                                    "No charge unless cancelled <24h"
                                })
                                .id("label")
                                .w(if short { 90. } else { 360. })
                                .h(20.)
                                .text_size(24.)
                                .text_wrap(false)
                                .overflow_hidden(),
                            )
                    },
                    || CompiledMorphTiming {
                        source_layout_scale: Some(1.),
                        duration: Duration::from_millis(400),
                        crossfade: Duration::from_millis(140),
                        color_duration: Duration::ZERO,
                        curve: std::array::from_fn(|i| i as f32 / 48.),
                    },
                ),
            ),
        ));
        ui.prepare_frame();
        let owner = root.find("label").unwrap();
        let parent = root.find("parent").unwrap();
        let paragraph = ui.scene.borrow().children(owner)[0];
        let glyph = ui.scene.borrow().children(paragraph)[0];
        let original = ui.scene.borrow().bounds(owner);
        assert_eq!(original, Rect::new(10., 10., 360., 20.));
        let ghost = |ui: &Ui| {
            ui.scene
                .borrow()
                .children(parent)
                .iter()
                .copied()
                .find(|node| *node != owner)
                .unwrap()
        };
        selected.set(true);
        ui.prepare_frame();
        let outgoing = ghost(&ui);
        let outgoing_glyph = ui.scene.borrow().children(outgoing)[0];
        let mut time = Instant::now();
        let mut index = 0;
        let mut advance = |ui: &Ui, millis| {
            executor.borrow_mut().tick();
            time += Duration::from_millis(millis);
            index += 1;
            frames.deliver(Frame {
                index,
                time,
                interval: Duration::from_millis(16),
            });
            executor.borrow_mut().tick();
            ui.prepare_frame();
        };
        advance(&ui, 50);
        let interrupted = {
            let scene = ui.scene.borrow();
            assert_eq!(root.find("label"), Some(owner));
            assert_eq!(scene.children(owner)[0], paragraph);
            assert_eq!(scene.children(paragraph)[0], glyph);
            assert!(scene.style(owner).clip && scene.style(outgoing).clip);
            assert_eq!(scene.bounds(outgoing), original);
            assert!(scene.bounds(owner).width < original.width);
            let visible = scene.visible_bounds(outgoing_glyph).unwrap();
            assert!(visible.x + visible.width > scene.bounds(owner).x + scene.bounds(owner).width);
            assert!(scene.layout_bounds(glyph).width > 90.);
            scene.bounds(owner)
        };
        selected.set(false);
        ui.prepare_frame();
        assert!(!ui.scene.borrow().contains(outgoing));
        let returning_ghost = ghost(&ui);
        assert_eq!(ui.scene.borrow().bounds(returning_ghost), interrupted);
        let returning_glyph = ui.scene.borrow().children(returning_ghost)[0];
        assert!(
            matches!(ui.scene.borrow().kind(returning_glyph), NodeKind::Text { text, .. } if text.as_ref() == "Saved card")
        );
        advance(&ui, 50);
        assert_eq!(ui.scene.borrow().bounds(returning_ghost), interrupted);
        root.unmount();
        assert!(!ui.scene.borrow().contains(returning_ghost));
        executor.borrow_mut().tick();
        assert!(!frames.wants_frame());
    }

    #[test]
    fn outgoing_text_preserves_visibility_effects_and_interrupted_enter_opacity() {
        use crate::{
            frame::{Frame, FrameClock},
            scene::Effects,
            task::LocalExecutor,
        };
        use std::time::Instant;
        for (base_opacity, visible, displayed, blur) in [
            (0., true, true, 0.),
            (0.4, true, true, 0.),
            (1., false, true, 0.),
            (1., true, false, 0.),
            (0.6, true, true, 3.),
        ] {
            let mut ui = Ui::new(600., 200.);
            let executor = Rc::new(RefCell::new(LocalExecutor::new()));
            let frames = FrameClock::new();
            let selected = ui.signal(false);
            let read = selected.clone();
            let root = ui.mount(provide(
                TaskRunner::from_executor(executor.clone()),
                provide(
                    frames.clone(),
                    compiled_morph(
                        move || read.get(),
                        move |changed| {
                            let content = if changed { "Replacement" } else { "Original" };
                            row()
                                .id("root")
                                .size(500., 100.)
                                .child(
                                    column().id("paint-parent").size(220., 50.).child(
                                        text(content)
                                            .id("label")
                                            .size(180., 24.)
                                            .opacity(base_opacity)
                                            .blur(blur)
                                            .when(!visible, |view| view.invisible())
                                            .when(!displayed, |view| view.hidden()),
                                    ),
                                )
                                .child(
                                    column()
                                        .id("reference-parent")
                                        .size(220., 50.)
                                        .child(text(content).id("reference").size(180., 24.)),
                                )
                        },
                        || CompiledMorphTiming {
                            source_layout_scale: Some(1.),
                            duration: Duration::from_millis(400),
                            crossfade: Duration::from_millis(140),
                            color_duration: Duration::ZERO,
                            curve: std::array::from_fn(|i| i as f32 / 48.),
                        },
                    ),
                ),
            ));
            ui.prepare_frame();
            let owner = root.find("label").unwrap();
            let parent = root.find("paint-parent").unwrap();
            let reference = root.find("reference").unwrap();
            let reference_parent = root.find("reference-parent").unwrap();
            let paragraph = ui.scene.borrow().children(owner)[0];
            let glyph = ui.scene.borrow().children(paragraph)[0];
            let leaf_effects = Effects {
                opacity: 0.7,
                blur_radius: 1.25,
                edge_fade: 0.2,
            };
            ui.scene.borrow_mut().set_effects(glyph, leaf_effects);
            let ghost = |ui: &Ui, parent, owner| {
                ui.scene
                    .borrow()
                    .children(parent)
                    .iter()
                    .copied()
                    .find(|node| *node != owner)
                    .unwrap()
            };
            selected.set(true);
            ui.prepare_frame();
            let first_ghost = ghost(&ui, parent, owner);
            let mut time = Instant::now();
            let mut index = 0;
            let mut advance = |ui: &Ui| {
                executor.borrow_mut().tick();
                time += Duration::from_millis(50);
                index += 1;
                frames.deliver(Frame {
                    index,
                    time,
                    interval: Duration::from_millis(16),
                });
                executor.borrow_mut().tick();
                ui.prepare_frame();
            };
            advance(&ui);
            let entering_opacity = {
                let scene = ui.scene.borrow();
                let reference_ghost = ghost(&ui, reference_parent, reference);
                let fade = scene.effects(reference_ghost).opacity;
                assert!(fade > 0. && fade < 1.);
                let actual = scene.effects(first_ghost);
                assert!((actual.opacity - base_opacity * fade).abs() < 0.000001);
                assert_eq!(actual.blur_radius, blur);
                let ghost_glyph = scene.children(first_ghost)[0];
                assert_eq!(scene.effects(ghost_glyph), leaf_effects);
                let options = scene.style(first_ghost).layout_options.unwrap();
                let original = scene.style(owner).layout_options;
                assert_eq!(
                    options.visible,
                    original.as_deref().and_then(|options| options.visible)
                );
                assert_eq!(
                    options.display,
                    original.as_deref().and_then(|options| options.display)
                );
                if base_opacity == 0. || !visible || !displayed {
                    assert!(scene.visible_bounds(ghost_glyph).is_none());
                } else {
                    assert!(scene.visible_bounds(ghost_glyph).is_some());
                }
                let entering = scene.effects(paragraph).opacity;
                assert!(entering > 0. && entering < 1.);
                entering
            };
            selected.set(false);
            ui.prepare_frame();
            assert!(!ui.scene.borrow().contains(first_ghost));
            let next_ghost = ghost(&ui, parent, owner);
            let next_glyph = ui.scene.borrow().children(next_ghost)[0];
            {
                let scene = ui.scene.borrow();
                assert_eq!(root.find("label"), Some(owner));
                assert_eq!(scene.children(owner)[0], paragraph);
                assert_eq!(scene.children(paragraph)[0], glyph);
                assert_eq!(scene.effects(next_ghost).opacity, base_opacity);
                assert_eq!(scene.effects(next_ghost).blur_radius, blur);
                let leaf = scene.effects(next_glyph);
                assert!((leaf.opacity - leaf_effects.opacity * entering_opacity).abs() < 0.000001);
                assert_eq!(leaf.blur_radius, leaf_effects.blur_radius);
                assert_eq!(leaf.edge_fade, leaf_effects.edge_fade);
            }
            advance(&ui);
            {
                let scene = ui.scene.borrow();
                let reference_ghost = ghost(&ui, reference_parent, reference);
                let fade = scene.effects(reference_ghost).opacity;
                assert!((scene.effects(next_ghost).opacity - base_opacity * fade).abs() < 0.000001);
                assert_eq!(scene.effects(next_ghost).blur_radius, blur);
                assert!(
                    (scene.effects(next_glyph).opacity - leaf_effects.opacity * entering_opacity).abs()
                        < 0.000001
                );
                if base_opacity == 0. || !visible || !displayed {
                    assert!(scene.visible_bounds(next_glyph).is_none());
                }
            }
            root.unmount();
            assert!(!ui.scene.borrow().contains(next_ghost));
            executor.borrow_mut().tick();
            assert!(!frames.wants_frame());
        }
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
                        source_layout_scale: None,
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
    fn source_flow_footprints_measure_css_pixels_at_import_scale() {
        use crate::{frame::FrameClock, task::LocalExecutor};
        for source_layout_scale in [None, Some(1.), Some(1.5), Some(2.)] {
            let scale = source_layout_scale.unwrap_or(1.);
            let mut ui = Ui::new(300., 300.);
            let output = Rc::new(RefCell::new(None));
            let capture = output.clone();
            let _root = ui.mount(provide(
                TaskRunner::from_executor(Rc::new(RefCell::new(LocalExecutor::new()))),
                provide(
                    FrameClock::new(),
                    component(move |cx| {
                        *capture.borrow_mut() = Some(StyleTrack::new(cx, Styles::new()));
                        div()
                    }),
                ),
            ));
            let track = output.borrow().clone().unwrap();
            let timing = CompiledMorphTiming {
                source_layout_scale,
                duration: Duration::from_millis(400),
                crossfade: Duration::from_millis(140),
                color_duration: Duration::from_millis(200),
                curve: std::array::from_fn(|i| i as f32 / 48.),
            };
            let bounds = crate::scene::Rect::new(0., 0., 20.4 * scale, 35.6 * scale);
            let original = Styles::new()
                .w(bounds.width)
                .h(bounds.height)
                .m(2.5 * scale);
            for layout in [Layout::Column, Layout::Row] {
                footprint(
                    &track,
                    original.clone(),
                    bounds,
                    layout,
                    14. * scale,
                    true,
                    true,
                    &timing,
                );
                let reveal = track.footprint.get().unwrap();
                let horizontal = layout == Layout::Row;
                let css_size: f32 = if horizontal { 20.4 } else { 35.6 };
                let measured = if source_layout_scale.is_some() {
                    css_size.round()
                } else {
                    css_size
                };
                let trail = reveal.from.margin.unwrap();
                assert!(
                    ((if horizontal {
                        trail.right
                    } else {
                        trail.bottom
                    }) + (measured + 2.5 + 14.) * scale)
                        .abs()
                        < 0.0001
                );
                assert_eq!(
                    reveal.from.transform.unwrap().y,
                    if horizontal { 0. } else { 6. * scale }
                );
                assert_eq!(reveal.to.clip_insets.flatten().unwrap().top, -20. * scale);
                if horizontal {
                    let height = if source_layout_scale.is_some() {
                        36.
                    } else {
                        35.6
                    } * scale;
                    assert!((reveal.to.height.unwrap() - height).abs() < 0.0001);
                } else {
                    assert_eq!(reveal.to.height, original.height);
                }
                assert_eq!(reveal.restore, Some(original.clone()));
                footprint(
                    &track,
                    original.clone(),
                    bounds,
                    layout,
                    14. * scale,
                    true,
                    false,
                    &timing,
                );
                let exit = track.footprint.get().unwrap();
                assert_eq!(exit.to.margin, reveal.from.margin);
                assert_eq!(exit.restore, None);
            }
        }
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
            source_layout_scale: None,
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
                        source_layout_scale: None,
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
