//! Declarative, retained components and children. Construction runs once per mount;
//! reactive properties update individual nodes without rebuilding their component.
use crate::{
    compose_style::{Typography, Variants, mount_style_with_intrinsic},
    input::{EventContext, EventPhase, InputEvent},
    reactive::{Runtime, ServiceScope, Signal},
    scene::{Layout, NodeId, NodeKind},
    semantics::{Role, SemanticNode},
    style::{Styled, Styles},
    widgets::{Ui, WeakUi},
};
use std::{
    any::Any,
    cell::RefCell,
    collections::HashMap,
    future::Future,
    pin::Pin,
    rc::{Rc, Weak},
};

#[path = "compose_compiled_morph.rs"]
mod compiled_morph;
pub use compiled_morph::{CompiledMorphAnimation, CompiledMorphEffect, CompiledMorphTiming, compiled_morph};

#[path = "compose_animation.rs"]
mod animation;
#[path = "compose_variable.rs"]
mod variable;
pub use variable::{VariableHeights, measured_rows, measured_virtual_list, variable_virtual_list};

/// A local UI future, supplied to a platform's wake-driven executor.
pub type LocalFuture = Pin<Box<dyn Future<Output = ()>>>;
trait Lease {
    fn finished(&self) -> bool;
}
struct CheckedLease<G, F> {
    guard: G,
    check: F,
}
impl<G, F: Fn(&G) -> bool> Lease for CheckedLease<G, F> {
    fn finished(&self) -> bool {
        (self.check)(&self.guard)
    }
}
/// Erases a cancel-on-drop executor guard without losing completion information.
pub struct TaskToken(Box<dyn Lease>);
impl TaskToken {
    pub fn new<G: 'static>(guard: G, finished: impl Fn(&G) -> bool + 'static) -> Self {
        Self(Box::new(CheckedLease {
            guard,
            check: finished,
        }))
    }
}
/// Provider bridging components to a native or headless executor.
#[derive(Clone)]
pub struct TaskRunner(Rc<dyn Fn(LocalFuture) -> TaskToken>);
impl TaskRunner {
    pub(crate) fn spawn_owned(&self, future: impl Future<Output = ()> + 'static) -> TaskToken {
        (self.0)(Box::pin(future))
    }
    pub fn new(spawn: impl Fn(LocalFuture) -> TaskToken + 'static) -> Self {
        Self(Rc::new(spawn))
    }
    pub fn from_executor(executor: Rc<RefCell<crate::task::LocalExecutor>>) -> Self {
        let spawner = executor.borrow().spawner();
        Self::new(move |future| {
            TaskToken::new(spawner.spawn_scoped(future), |task| {
                task.as_ref().is_none_or(|task| task.is_finished())
            })
        })
    }
}
#[derive(Default)]
struct TaskOwner {
    tasks: RefCell<Vec<TaskToken>>,
}
/// Cloneable event-handler capability. It never keeps its component alive.
#[derive(Clone)]
pub struct Tasks {
    owner: Weak<TaskOwner>,
    runner: TaskRunner,
}
impl Tasks {
    pub fn spawn(&self, future: impl Future<Output = ()> + 'static) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        // Completion callbacks and guard destructors belong to the executor.
        // They can spawn another task through this same capability; detach the
        // current batch before invoking either, preserving reentrant additions.
        let mut pending = std::mem::take(&mut *owner.tasks.borrow_mut());
        pending.retain(|task| !task.0.finished());
        // Reuse the existing task vector on the ordinary non-reentrant path.
        let reentrant = owner.tasks.replace(pending);
        owner.tasks.borrow_mut().extend(reentrant);
        let task = (self.runner.0)(Box::pin(future));
        owner.tasks.borrow_mut().push(task);
    }
}
type Registry = Rc<RefCell<HashMap<String, Vec<NodeId>>>>;
#[derive(Clone)]
struct Environment {
    services: ServiceScope,
    typography: Signal<Typography>,
    registry: Registry,
    portal: Option<(NodeId, Signal<bool>)>,
    menu: Option<Rc<crate::compose_menu::MenuContext>>,
    measure: crate::rich_text::TextMeasure,
}
/// Mount-time component context. State is allocated once; there are no positional hooks.
pub struct Context {
    runtime: Runtime,
    environment: Environment,
    resources: Vec<Box<dyn Any>>,
    tasks: Option<Rc<TaskOwner>>,
}
impl Context {
    fn new(runtime: Runtime, environment: Environment) -> Self {
        Self {
            runtime,
            environment,
            resources: Vec::new(),
            tasks: None,
        }
    }
    pub fn runtime(&self) -> Runtime {
        self.runtime.clone()
    }
    pub fn state<T: 'static>(&self, initial: T) -> Signal<T> {
        self.runtime.signal(initial)
    }
    pub fn try_service<T: 'static>(&self) -> Option<Rc<T>> {
        self.environment.services.get()
    }
    pub fn service<T: 'static>(&self) -> Rc<T> {
        self.try_service().unwrap_or_else(|| {
            panic!(
                "missing component provider for {}",
                std::any::type_name::<T>()
            )
        })
    }
    pub fn retain(&mut self, resource: impl Any) {
        self.resources.push(Box::new(resource));
    }
    pub fn tasks(&mut self) -> Tasks {
        let runner = self.service::<TaskRunner>();
        let owner = self
            .tasks
            .get_or_insert_with(|| Rc::new(TaskOwner::default()));
        Tasks {
            owner: Rc::downgrade(owner),
            runner: (*runner).clone(),
        }
    }
    /// The window's display-paced frame clock. Await it from [`Context::tasks`]
    /// to animate once per refresh; see [`crate::frame`].
    /// Measures rich text as this tree's layout does.
    pub fn text_measure(&self) -> crate::rich_text::TextMeasure {
        self.environment.measure.clone()
    }
    pub fn frames(&self) -> crate::frame::FrameClock {
        (*self.service::<crate::frame::FrameClock>()).clone()
    }
    /// Captures the caller's providers. The receiver owns the mounted content;
    /// visual styles inherit at the insertion point, while lexical captures stay intact.
    pub fn slot(&self, build: impl FnOnce(&mut Context) -> View + 'static) -> Slot {
        Slot {
            build: Box::new(build),
            services: self.environment.services.clone(),
        }
    }
    fn finish(mut self, ui: &mut Ui, root: NodeId) {
        if let Some(tasks) = self.tasks.take() {
            self.resources.push(Box::new(tasks));
        }
        if !self.resources.is_empty() {
            ui.retain(root, self.resources);
        }
    }
}
/// A single-use child slot with lexical providers and ordinary Rust captures.
pub struct Slot {
    build: Box<dyn FnOnce(&mut Context) -> View>,
    services: ServiceScope,
}
impl From<Slot> for View {
    fn from(slot: Slot) -> Self {
        View::new(Kind::Slot(slot))
    }
}
type Builder = Box<dyn FnOnce(&mut Context) -> View>;
type RegionBuilder = Box<dyn FnOnce(&mut Ui, NodeId, Environment)>;
type VirtualBuilder = Box<dyn FnOnce(&mut Ui, NodeId, Environment, ScrollbarStyle, bool)>;
/// Reactive display content sharing one retained paragraph across plain and rich paint.
#[derive(Clone, PartialEq)]
pub enum DisplayText {
    Plain(String),
    Rich(Vec<TextSpan>),
}
enum Kind {
    Container(Layout),
    Text(String),
    BoundText(Box<dyn FnMut() -> String>),
    RichText(Box<dyn FnMut() -> Vec<crate::compose_rich::TextSpan>>),
    DisplayText(Box<dyn FnMut() -> DisplayText>),
    Button,
    MenuItem(String),
    Submenu {
        label: String,
        open: Signal<bool>,
        anchor: Option<Box<View>>,
    },
    Menu {
        label: String,
        open: Signal<bool>,
        anchor: Box<View>,
    },
    Modal {
        label: String,
        open: Signal<bool>,
        anchor: Option<Box<View>>,
    },
    Scroll {
        offset: Signal<f32>,
        horizontal: bool,
    },
    Canvas(Box<dyn FnMut((f32, f32)) -> crate::canvas::Canvas>),
    Svg {
        label: String,
        source: Box<dyn FnMut() -> std::sync::Arc<crate::svg::SvgData>>,
    },
    #[cfg(target_os = "macos")]
    NativeSurface {
        label: String,
        source: Box<dyn FnMut() -> std::rc::Rc<crate::native_surface::NativeSurface>>,
    },
    Image {
        label: String,
        source: Box<dyn FnMut() -> std::sync::Arc<crate::image::ImageData>>,
    },
    Progress {
        label: String,
        value: Signal<f32>,
    },
    Checkbox {
        label: String,
        value: Signal<bool>,
    },
    Slider {
        label: String,
        value: Signal<f32>,
        range: std::ops::RangeInclusive<f32>,
    },
    Editor {
        label: String,
        value: Signal<String>,
        multiline: bool,
    },
    Component(Builder),
    Provider(Box<dyn FnOnce(&mut Context)>, Box<View>),
    Slot(Slot),
    Region(RegionBuilder),
    VirtualList(VirtualBuilder),
}
type EventListener = Box<dyn FnMut(&mut EventContext)>;

#[derive(Clone)]
pub(crate) struct AboveAnchor {
    pub bounds: Signal<crate::scene::Rect>,
    pub align_right: Option<Signal<bool>>,
    pub gap: f32,
    pub below: bool,
}

/// Optional portal presentation. Focus and hit routing follow logical visibility.
#[derive(Clone, Copy)]
pub struct PopupMotion {
    pub enter: crate::motion::Transition,
    pub exit: crate::motion::Transition,
    pub enter_scale: f32,
    /// Normalized transform origin within the panel's unscaled rectangle.
    pub origin: [f32; 2],
}

/// Scrollbar presentation; all visible styles share the same retained input control.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollbarStyle {
    Hidden,
    #[default]
    Overlay,
    /// Persistent 15 px track. Content gutters remain the caller's layout choice.
    Classic,
}

/// A mount description, consumed into owned retained nodes and subscriptions.
/// `.child`/`.children` compose views; `.style` and fluent methods apply to their root.
pub struct View {
    kind: Kind,
    styles: Styles,
    children: Vec<View>,
    variants: Variants,
    reactive_style: Option<Box<dyn FnMut() -> Styles>>,
    accessibility: Option<Box<dyn FnMut(&mut SemanticNode)>>,
    disabled: Option<Box<dyn FnMut() -> bool>>,
    read_only: Option<Box<dyn FnMut() -> bool>>,
    selection_style: Option<(crate::scene::Color, f32, crate::scene::Insets)>,
    select_all_on_focus: Option<bool>,
    on_editor: Option<Box<dyn FnOnce(crate::widgets::EditorHandle)>>,
    mounted: Vec<Box<dyn FnOnce(NodeId)>>,
    click: Option<Box<dyn FnMut()>>,
    events: Vec<EventListener>,
    keymap: crate::actions::Keymap,
    pub(crate) drag_preview: Option<crate::compose_drag::PreviewBuilder>,
    focusable: Option<bool>,
    id: Option<String>,
    scrollbar: Option<ScrollbarStyle>,
    scroll_fit_cross_axis: Option<bool>,
    horizontal_wheel: Option<bool>,
    slider_chrome: Option<bool>,
    slider_step: Option<f32>,
    slider_thumb_width: Option<f32>,
    animation: Option<animation::Animated>,
    keyboard_navigation: Option<bool>,
    dismiss_backdrop: Option<bool>,
    anchor_above: Option<AboveAnchor>,
    popup_motion: Option<Box<dyn FnMut() -> PopupMotion>>,
    layout_target: Option<Signal<Option<NodeId>>>,
    trigger_id: Option<String>,
    menu_trigger: Option<Signal<bool>>,
    menu_checked: Option<bool>,
    visibility: Vec<Signal<bool>>,
    bounds: Vec<Signal<crate::scene::Rect>>,
    layout_bounds: Vec<Signal<Option<crate::scene::Rect>>>,
    layout_motion: Option<crate::motion::Transition>,
    layout_id: Option<String>,
    scroll_progress: Option<Signal<f32>>,
    scroll_fades: Option<(f32, Rc<dyn Fn() -> std::time::Duration>)>,
    morph_effect: Option<CompiledMorphEffect>,
}
impl View {
    pub fn popup_motion(mut self, policy: impl FnMut() -> PopupMotion + 'static) -> Self {
        self.popup_motion = Some(Box::new(policy));
        self
    }
    /// Set the controlled checked state of a menu item, including accessibility.
    pub fn menu_checked(mut self, checked: bool) -> Self {
        assert!(
            matches!(self.kind, Kind::MenuItem(_)),
            "menu_checked requires a menu item"
        );
        self.menu_checked = Some(checked);
        self
    }
    /// Transform decoded image/SVG painting around its allocated center without
    /// changing layout. Use `image_signal` and `ImageData::transformed` for a
    /// reactive matrix. This method requires an image view.
    pub fn with_transformation(mut self, transformation: crate::affine::Affine) -> Self {
        if let Some(animated) = &mut self.animation {
            let frames = animated
                .animation
                .frames()
                .iter()
                .map(|frame| crate::animation::Frame {
                    image: std::sync::Arc::new(frame.image.transformed(transformation)),
                    duration: frame.duration,
                })
                .collect();
            animated.animation = std::sync::Arc::new(
                crate::animation::Animation::new(frames, animated.animation.loop_count())
                    .expect("transforms preserve animation limits"),
            );
        }
        self.kind = match self.kind {
            Kind::Image { label, mut source } => Kind::Image {
                label,
                source: Box::new(move || std::sync::Arc::new(source().transformed(transformation))),
            },
            Kind::Svg { label, mut source } => Kind::Svg {
                label,
                source: Box::new(move || std::sync::Arc::new(source().transformed(transformation))),
            },
            _ => panic!("with_transformation requires an image or SVG view"),
        };
        self
    }
    pub fn svg_tint(mut self, color: crate::scene::Color) -> Self {
        self.kind = match self.kind {
            Kind::Svg { label, mut source } => Kind::Svg {
                label,
                source: Box::new(move || std::sync::Arc::new(source().tinted(color))),
            },
            _ => panic!("svg_tint requires an SVG view"),
        };
        self
    }
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            styles: Styles::new(),
            children: Vec::new(),
            variants: Variants::default(),
            reactive_style: None,
            accessibility: None,
            disabled: None,
            read_only: None,
            selection_style: None,
            select_all_on_focus: None,
            on_editor: None,
            mounted: Vec::new(),
            click: None,
            events: Vec::new(),
            keymap: crate::actions::Keymap::new(),
            drag_preview: None,
            focusable: None,
            id: None,
            scrollbar: None,
            scroll_fit_cross_axis: None,
            horizontal_wheel: None,
            slider_chrome: None,
            slider_step: None,
            slider_thumb_width: None,
            animation: None,
            keyboard_navigation: None,
            dismiss_backdrop: None,
            anchor_above: None,
            popup_motion: None,
            layout_target: None,
            trigger_id: None,
            menu_trigger: None,
            menu_checked: None,
            visibility: Vec::new(),
            bounds: Vec::new(),
            layout_bounds: Vec::new(),
            layout_motion: None,
            layout_id: None,
            scroll_progress: None,
            scroll_fades: None,
            morph_effect: None,
        }
    }
    /// Overlay scrollbars are enabled by default; pass false to hide one.
    pub fn scrollbar(mut self, visible: bool) -> Self {
        self.scrollbar = Some(if visible {
            ScrollbarStyle::Overlay
        } else {
            ScrollbarStyle::Hidden
        });
        self
    }
    /// Select persistent classic chrome or the default fading overlay.
    pub fn scrollbar_style(mut self, style: ScrollbarStyle) -> Self {
        self.scrollbar = Some(style);
        self
    }
    /// Keep slider input and semantics while the caller supplies its paint.
    pub fn slider_chrome(mut self, visible: bool) -> Self {
        assert!(matches!(self.kind, Kind::Slider { .. }), "slider_chrome requires a slider");
        self.slider_chrome = Some(visible);
        self
    }
    /// Quantize pointer, keyboard and accessibility changes to this interval.
    pub fn slider_step(mut self, step: f32) -> Self {
        assert!(matches!(self.kind, Kind::Slider { .. }) && step.is_finite() && step > 0., "slider_step requires a slider and positive finite interval");
        self.slider_step = Some(step);
        self
    }
    /// Match the pointer travel to a custom painted thumb, or use zero for a fill bar.
    pub fn slider_thumb_width(mut self, width: f32) -> Self {
        assert!(matches!(self.kind, Kind::Slider { .. }) && width.is_finite() && width >= 0., "slider_thumb_width requires a slider and nonnegative finite width");
        self.slider_thumb_width = Some(width);
        self
    }
    pub fn morph_effect(mut self,effect:CompiledMorphEffect)->Self {
        self.morph_effect=Some(effect);self
    }
    /// Let a scroll viewport measure its non-scrolling axis from its content.
    pub fn fit_cross_axis(mut self, enabled: bool) -> Self {
        self.scroll_fit_cross_axis = Some(enabled);
        self
    }
    /// Let horizontal scrolling consume vertical wheel gestures.
    pub fn horizontal_wheel(mut self, enabled: bool) -> Self {
        assert!(matches!(self.kind, Kind::Scroll { horizontal: true, .. }), "horizontal_wheel requires scroll_x");
        self.horizontal_wheel = Some(enabled);
        self
    }
    /// Observe clipped/offscreen visibility and native presentation. The signal
    /// updates on geometry/presentation changes, without per-frame scene walks.
    pub fn observe_visibility(mut self, visible: Signal<bool>) -> Self {
        self.visibility.push(visible);
        self
    }
    /// Observe settled world bounds, published by layout without per-frame scene walks.
    pub fn observe_bounds(mut self, bounds: Signal<crate::scene::Rect>) -> Self {
        self.bounds.push(bounds);
        self
    }
    /// Observe untransformed layout geometry for sizing shaders and stacked
    /// content. Animated paint transforms never feed back into this measurement.
    pub fn observe_layout_bounds(mut self, bounds: Signal<Option<crate::scene::Rect>>) -> Self {
        self.layout_bounds.push(bounds);
        self
    }

    /// Decorate native parts bottom-up, retaining the existing root identity.
    pub fn map_parts(self, mut map: impl FnMut(&str, View) -> View) -> Self {
        fn visit(mut view: View, map: &mut impl FnMut(&str, View) -> View) -> View {
            view.children = view.children.into_iter().map(|child| visit(child, map)).collect();
            let id = view.id.clone();
            let mut result = map(id.as_deref().unwrap_or(""), view);
            if result.id.is_none() { result.id = id; }
            result
        }
        visit(self, &mut map)
    }
    /// Refine compiled native parts before they enter a retained variant.
    pub fn map_part_styles(mut self, mut map: impl FnMut(&str, &mut Styles)) -> Self {
        fn visit(view: &mut View, map: &mut impl FnMut(&str, &mut Styles)) {
            map(view.id.as_deref().unwrap_or(""), &mut view.styles);
            for child in &mut view.children {
                visit(child, map);
            }
        }
        visit(&mut self, &mut map);
        self
    }

    /// Omit explicitly removed parts while retaining all other permanent identities.
    pub fn retain_parts(mut self, mut include: impl FnMut(&str) -> bool) -> Self {
        fn visit(view: &mut View, include: &mut impl FnMut(&str) -> bool) {
            view.children
                .retain(|child| include(child.id.as_deref().unwrap_or("")));
            for child in &mut view.children {
                visit(child, include);
            }
        }
        visit(&mut self, &mut include);
        self
    }

    /// Change flow order for explicitly identified parts while retaining their native nodes.
    pub fn order_part_children(mut self, parent: &str, mut order: impl FnMut(&str) -> i32) -> Self {
        fn visit(view: &mut View, parent: &str, order: &mut impl FnMut(&str) -> i32) {
            if view.id.as_deref() == Some(parent) {
                view.children
                    .sort_by_key(|child| order(child.id.as_deref().unwrap_or("")));
            }
            for child in &mut view.children {
                visit(child, parent, order);
            }
        }
        visit(&mut self, parent, &mut order);
        self
    }
    /// Project from previous layout position and size without per-frame layout.
    /// Size changes apply immediately; this projects position only.
    pub fn layout_motion(mut self, transition: crate::motion::Transition) -> Self {
        transition.validate();
        self.layout_motion = Some(transition);
        self
    }
    /// Match an earlier mount inside an explicit SharedLayoutScope provider.
    /// Combine with layout_motion to choose its transition.
    pub fn layout_id(mut self, id: impl Into<String>) -> Self {
        let id = id.into();
        assert!(
            !id.is_empty() && id.len() <= 256,
            "layout ID needs 1..=256 bytes"
        );
        self.layout_id = Some(id);
        self
    }
    /// Publish normalized progress using the scroll view's measured extent.
    pub fn scroll_progress(mut self, progress: Signal<f32>) -> Self {
        assert!(
            matches!(self.kind, Kind::Scroll { .. }),
            "scroll_progress requires a scroll view"
        );
        self.scroll_progress = Some(progress);
        self
    }
    /// Enable bounded keyboard row navigation on a virtual list.
    pub fn keyboard_navigation(mut self, enabled: bool) -> Self {
        self.keyboard_navigation = Some(enabled);
        self
    }
    pub fn scroll_fades(self, band: f32) -> Self {
        self.scroll_fades_with_duration(band, || std::time::Duration::from_millis(180))
    }
    /// Animate scroll edge masks using the caller's motion timing policy.
    pub fn scroll_fades_with_duration(mut self, band: f32, duration: impl Fn() -> std::time::Duration + 'static) -> Self {
        assert!(matches!(self.kind, Kind::Scroll {..}) && band.is_finite() && band >= 0.);
        self.scroll_fades=Some((band, Rc::new(duration)));
        self
    }
    /// Configure whether clicking outside a modal or popover panel dismisses it.
    pub fn dismiss_on_backdrop(mut self, enabled: bool) -> Self {
        self.dismiss_backdrop = Some(enabled);
        self
    }
    /// Assign an ID to a submenu's internally owned menu-item trigger.
    pub fn trigger_id(mut self, id: impl Into<String>) -> Self {
        self.trigger_id = Some(id.into());
        self
    }
    pub fn child(mut self, child: impl Into<View>) -> Self {
        self.children.push(child.into());
        self
    }
    pub fn children(mut self, children: impl IntoIterator<Item = View>) -> Self {
        self.children.extend(children);
        self
    }
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
    pub fn on_click(mut self, callback: impl FnMut() + 'static) -> Self {
        self.click = Some(Box::new(callback));
        self
    }
    /// Append an owned listener for capture, target and bubble events.
    /// Inspect `EventContext::phase` to select a phase. Listeners run in declaration
    /// order, before built-in handlers; component wrapper listeners follow inner ones.
    pub fn on_event(mut self, callback: impl FnMut(&mut EventContext) + 'static) -> Self {
        self.events.push(Box::new(callback));
        self
    }
    /// Add scoped bindings and literal key contexts to this retained component.
    pub fn keymap(mut self, keymap: crate::actions::Keymap) -> Self {
        self.keymap.extend(keymap);
        self
    }
    /// Handle a typed action at the target or during bubbling. The handler may
    /// stop propagation; leaving it unstopped allows an ancestor fallback.
    pub fn on_action<T: 'static>(
        self,
        mut callback: impl FnMut(&T, &mut EventContext) + 'static,
    ) -> Self {
        self.on_event(move |cx| {
            if cx.phase == EventPhase::Capture {
                return;
            }
            if let InputEvent::Action(action) = cx.event.clone()
                && let Some(value) = action.downcast_ref::<T>()
            {
                callback(value, cx);
            }
        })
    }
    /// Override whether this root accepts focus and participates in Tab traversal.
    /// Registering an event listener alone does not make a passive view focusable.
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = Some(focusable);
        self
    }
    /// Present above an external retained trigger. Repeated trigger presses
    /// dismiss the old menu and route the press back to that trigger.
    pub fn anchor_above(mut self, bounds: Signal<crate::scene::Rect>) -> Self {
        self.anchor_above = Some(AboveAnchor {
            bounds,
            align_right: None,
            gap: 8.,
            below: false,
        });
        self
    }
    /// Align an external anchored panel to either edge of its trigger.
    pub fn anchor_above_aligned(
        mut self,
        bounds: Signal<crate::scene::Rect>,
        align_right: Signal<bool>,
        gap: f32,
    ) -> Self {
        assert!(gap.is_finite() && gap >= 0.);
        self.anchor_above = Some(AboveAnchor {
            bounds,
            align_right: Some(align_right),
            gap,
            below: false,
        });
        self
    }
    /// Align a panel below an external trigger without changing its logical owner.
    pub fn anchor_below_aligned(
        mut self,
        bounds: Signal<crate::scene::Rect>,
        align_right: Signal<bool>,
        gap: f32,
    ) -> Self {
        assert!(gap.is_finite() && gap >= 0.);
        self.anchor_above = Some(AboveAnchor { bounds, align_right: Some(align_right), gap, below: true });
        self
    }
    pub fn hover(mut self, style: impl FnOnce(Styles) -> Styles) -> Self {
        self.variants.hover = Some(style(Styles::new()));
        self
    }
    pub fn active(mut self, style: impl FnOnce(Styles) -> Styles) -> Self {
        self.variants.active = Some(style(Styles::new()));
        self
    }
    pub fn focus(mut self, style: impl FnOnce(Styles) -> Styles) -> Self {
        self.variants.focus = Some(style(Styles::new()));
        self
    }
    pub fn disabled_style(mut self, style: impl FnOnce(Styles) -> Styles) -> Self {
        self.variants.disabled = Some(style(Styles::new()));
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = Some(Box::new(move || disabled));
        self
    }
    pub fn disabled_when(mut self, disabled: impl FnMut() -> bool + 'static) -> Self {
        self.disabled = Some(Box::new(disabled));
        self
    }
    /// Keep an editor selectable and focusable while preventing user edits.
    /// External model updates remain permitted. Only editor roots accept this.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = Some(Box::new(move || read_only));
        self
    }
    /// Rounded selection paint for editor views. Padding is optical paint only.
    pub fn selection_style(
        mut self,
        color: crate::scene::Color,
        radius: f32,
        padding: crate::scene::Insets,
    ) -> Self {
        self.selection_style = Some((color, radius, padding));
        self
    }
    pub fn select_all_on_focus(mut self, select: bool) -> Self {
        self.select_all_on_focus = Some(select);
        self
    }
    /// Capture the retained native editor for programmatic selection and editing.
    pub fn on_editor(
        mut self,
        callback: impl FnOnce(crate::widgets::EditorHandle) + 'static,
    ) -> Self {
        self.on_editor = Some(Box::new(callback));
        self
    }
    /// Reactively control an editor's read-only state without remounting it.
    pub fn read_only_when(mut self, read_only: impl FnMut() -> bool + 'static) -> Self {
        self.read_only = Some(Box::new(read_only));
        self
    }
    /// Supply reactive semantics for a styled control while retaining native
    /// focus, disabled state and event routing. Its explicit name is independent
    /// of decorative or overlapping presentation labels.
    pub fn accessibility(mut self, update: impl FnMut(&mut SemanticNode) + 'static) -> Self {
        self.accessibility = Some(Box::new(update));
        self
    }
    pub fn reactive_style(mut self, style: impl FnMut() -> Styles + 'static) -> Self {
        self.reactive_style = Some(Box::new(style));
        self
    }
    /// Observe the actual retained root after its identity and input options exist.
    pub fn on_mount(mut self, callback: impl FnOnce(NodeId) + 'static) -> Self {
        self.mounted.push(Box::new(callback));
        self
    }
    // Wrapper properties refine the component's actual root without an extra layout box.
    fn refine(self, mut inner: View) -> View {
        inner.mounted.extend(self.mounted);
        inner.visibility.extend(self.visibility);
        inner.bounds.extend(self.bounds);
        inner.scroll_fades = self.scroll_fades.or(inner.scroll_fades);
        inner.layout_bounds.extend(self.layout_bounds);
        inner.morph_effect = self.morph_effect.or(inner.morph_effect);
        inner.scroll_fit_cross_axis = self.scroll_fit_cross_axis.or(inner.scroll_fit_cross_axis);
        inner.horizontal_wheel = self.horizontal_wheel.or(inner.horizontal_wheel);
        inner.slider_chrome = self.slider_chrome.or(inner.slider_chrome);
        inner.slider_step = self.slider_step.or(inner.slider_step);
        inner.slider_thumb_width = self.slider_thumb_width.or(inner.slider_thumb_width);
        if self.accessibility.is_some() { inner.accessibility = self.accessibility; }
        inner.layout_motion = self.layout_motion.or(inner.layout_motion);
        inner.layout_id = self.layout_id.or(inner.layout_id);
        inner.scroll_progress = self.scroll_progress.or(inner.scroll_progress);
        if self.trigger_id.is_some() {
            inner.trigger_id = self.trigger_id;
        }
        if self.dismiss_backdrop.is_some() {
            inner.dismiss_backdrop = self.dismiss_backdrop;
        }
        if self.anchor_above.is_some() {
            inner.anchor_above = self.anchor_above;
        }
        inner.popup_motion = self.popup_motion.or(inner.popup_motion);
        if self.keyboard_navigation.is_some() {
            inner.keyboard_navigation = self.keyboard_navigation;
        }
        if self.scrollbar.is_some() {
            inner.scrollbar = self.scrollbar;
        }
        inner.events.extend(self.events);
        inner.keymap.extend(self.keymap);
        if self.drag_preview.is_some() {
            inner.drag_preview = self.drag_preview;
        }
        if self.focusable.is_some() {
            inner.focusable = self.focusable;
        }
        inner.styles.merge(&self.styles);
        inner.children.extend(self.children);
        fn merge_variant(base: &mut Option<Styles>, patch: Option<Styles>) {
            if let Some(patch) = patch {
                base.get_or_insert_with(Styles::new).merge(&patch);
            }
        }
        merge_variant(&mut inner.variants.hover, self.variants.hover);
        merge_variant(&mut inner.variants.active, self.variants.active);
        merge_variant(&mut inner.variants.focus, self.variants.focus);
        merge_variant(&mut inner.variants.disabled, self.variants.disabled);
        if let Some(mut outer) = self.reactive_style {
            inner.reactive_style = Some(match inner.reactive_style.take() {
                Some(mut original) => Box::new(move || {
                    let mut style = original();
                    style.merge(&outer());
                    style
                }),
                None => outer,
            });
        }
        if self.disabled.is_some() {
            inner.disabled = self.disabled;
        }
        if self.read_only.is_some() {
            inner.read_only = self.read_only;
        }
        if self.selection_style.is_some() {
            inner.selection_style = self.selection_style;
        }
        if self.select_all_on_focus.is_some() {
            inner.select_all_on_focus = self.select_all_on_focus;
        }
        if let Some(outer) = self.on_editor {
            inner.on_editor = Some(match inner.on_editor.take() {
                Some(original) => Box::new(move |editor| {
                    original(editor.clone());
                    outer(editor);
                }),
                None => outer,
            });
        }
        if self.click.is_some() {
            inner.click = self.click;
        }
        if self.id.is_some() {
            inner.id = self.id;
        }
        inner
    }
}
impl Styled for View {
    fn styles_mut(&mut self) -> &mut Styles {
        &mut self.styles
    }
}
impl From<&str> for View {
    fn from(value: &str) -> Self {
        text(value)
    }
}
impl From<String> for View {
    fn from(value: String) -> Self {
        text(value)
    }
}
/// Retained vector content. The callback tracks reactive reads and receives the
/// allocated content size. Drawing is clipped to that box; children are rejected.
pub fn canvas(draw: impl FnMut((f32, f32)) -> crate::canvas::Canvas + 'static) -> View {
    View::new(Kind::Canvas(Box::new(draw)))
}
pub fn row() -> View {
    View::new(Kind::Container(Layout::Row))
}
pub fn column() -> View {
    View::new(Kind::Container(Layout::Column))
}
pub fn div() -> View {
    column()
}
pub fn overlay() -> View {
    View::new(Kind::Container(Layout::Overlay))
}
pub fn text(value: impl Into<String>) -> View {
    View::new(Kind::Text(value.into()))
}
pub fn text_signal(compute: impl FnMut() -> String + 'static) -> View {
    View::new(Kind::BoundText(Box::new(compute)))
}
pub use crate::compose_rich::{RichTextView, TextSpan, rich_text, text_span};
/// Reactive display content; plain values retain the native plain-text shaping path.
pub fn display_text_signal(compute: impl FnMut() -> DisplayText + 'static) -> View {
    View::new(Kind::DisplayText(Box::new(compute)))
}
/// Reactive inline children; equal resolved content preserves paragraph layout/paint.
pub fn rich_text_signal(compute: impl FnMut() -> Vec<TextSpan> + 'static) -> View {
    View::new(Kind::RichText(Box::new(compute)))
}
pub(crate) fn with_rich_spans(mut view: View, spans: Vec<TextSpan>) -> View {
    view.kind = Kind::RichText(Box::new(move || spans.clone()));
    view
}
/// An immutable decoded image. Unspecified dimensions use the source pixel size;
/// explicit dimensions stretch its pixels into the allocated content box.
/// Decoded frames with owned, visibility-aware playback. Offscreen, clipped and
/// transparent allocations pause and remove their timer; removal cancels it.
pub fn animated_image(
    label: impl Into<String>,
    animation: std::sync::Arc<crate::animation::Animation>,
) -> View {
    let mut view = image(label, animation.frames()[0].image.clone());
    view.animation = Some(animation::Animated {
        animation,
        playing: None,
    });
    view
}
pub fn animated_image_controlled(
    label: impl Into<String>,
    animation: std::sync::Arc<crate::animation::Animation>,
    playing: Signal<bool>,
) -> View {
    let mut view = animated_image(label, animation);
    view.animation.as_mut().unwrap().playing = Some(playing);
    view
}
#[cfg(target_os = "macos")]
pub fn native_surface(
    label: impl Into<String>,
    source: std::rc::Rc<crate::native_surface::NativeSurface>,
) -> View {
    native_surface_signal(label, move || source.clone())
}
#[cfg(target_os = "macos")]
pub fn native_surface_signal(
    label: impl Into<String>,
    source: impl FnMut() -> std::rc::Rc<crate::native_surface::NativeSurface> + 'static,
) -> View {
    View::new(Kind::NativeSurface {
        label: label.into(),
        source: Box::new(source),
    })
    .object_fit(crate::style::ObjectFit::Contain)
}
pub fn image(label: impl Into<String>, source: std::sync::Arc<crate::image::ImageData>) -> View {
    image_signal(label, move || source.clone())
}
/// A retained image whose decoded source follows reactive reads in `source`.
/// Image views own their pixels and reject additional children.
/// Load with a shared bounded cache. The component owns its request; unmounting
/// cancels transport when no other component is waiting for the same key.
/// Give it a size before its pixels arrive (a width with `aspect_ratio`, as
/// an API or markdown often states them) and it lays out once: loading shows
/// in that box and the image fits it, so nothing around it moves.
pub fn async_image<
    F: Future<
            Output = Result<
                std::sync::Arc<crate::image::ImageData>,
                crate::image_cache::ImageLoadError,
            >,
        > + 'static,
>(
    label: impl Into<String>,
    cache: crate::image_cache::ImageCache,
    key: impl Into<String>,
    fetch: impl FnOnce() -> F + 'static,
    loading: View,
    mut error: impl FnMut(crate::image_cache::ImageLoadError) -> View + 'static,
) -> View {
    let label = label.into();
    let key = key.into();
    component(move |cx| {
        let state = cx.state(None);
        let write = state.clone();
        let request = cache.load(key, fetch);
        cx.tasks().spawn(async move {
            write.set(Some(request.await));
        });
        let mut loading = Some(loading);
        switch(
            move || state.get(),
            move |value, _| match value {
                None => loading.take().expect("initial loading view"),
                Some(Ok(source)) => image(label.clone(), source),
                Some(Err(failure)) => error(failure),
            },
        )
    })
}
/// SVG source parsed and rasterized by the renderer at the allocated device size.
/// Defaults to a 24-pixel square; size, tint and transforms retain their node.
pub fn svg(label: impl Into<String>, source: std::sync::Arc<crate::svg::SvgData>) -> View {
    svg_signal(label, move || source.clone())
}
pub fn svg_signal(
    label: impl Into<String>,
    source: impl FnMut() -> std::sync::Arc<crate::svg::SvgData> + 'static,
) -> View {
    View::new(Kind::Svg {
        label: label.into(),
        source: Box::new(source),
    })
    .size(24., 24.)
}
pub fn image_signal(
    label: impl Into<String>,
    source: impl FnMut() -> std::sync::Arc<crate::image::ImageData> + 'static,
) -> View {
    View::new(Kind::Image {
        label: label.into(),
        source: Box::new(source),
    })
}
/// A vertical viewport for ordinary retained children, stacked in a column.
/// Child geometry determines its scroll extent; `offset` is clamped after layout.
pub fn scroll(offset: Signal<f32>) -> View {
    View::new(Kind::Scroll {
        offset,
        horizontal: false,
    })
}
/// A horizontal viewport for retained children, arranged in a row.
/// Only horizontal wheel deltas are consumed; vertical scrolling bubbles.
pub fn scroll_x(offset: Signal<f32>) -> View {
    View::new(Kind::Scroll {
        offset,
        horizontal: true,
    })
}
/// A signal-controlled modal portal. Children and fluent styles describe its panel.
pub fn modal(label: impl Into<String>, open: Signal<bool>) -> View {
    View::new(Kind::Modal {
        label: label.into(),
        open,
        anchor: None,
    })
}
/// An anchored popup with retained panel children. The anchor stays in ordinary layout.
pub fn popover(label: impl Into<String>, open: Signal<bool>, anchor: View) -> View {
    View::new(Kind::Modal {
        label: label.into(),
        open,
        anchor: Some(Box::new(anchor)),
    })
}
/// An anchored menu whose retained children may be components or keyed menu items.
pub fn menu(label: impl Into<String>, open: Signal<bool>, anchor: View) -> View {
    View::new(Kind::Menu {
        label: label.into(),
        open,
        anchor: Box::new(anchor),
    })
}
/// A menu action. Activation closes its menu before running `.on_click`.
/// A side-opening child menu with an automatically owned menu-item trigger.
pub fn submenu(label: impl Into<String>, open: Signal<bool>) -> View {
    View::new(Kind::Submenu {
        label: label.into(),
        open,
        anchor: None,
    })
}
/// A side-opening submenu with the caller's styled, accessible trigger.
pub fn submenu_with_anchor(label: impl Into<String>, open: Signal<bool>, anchor: View) -> View {
    View::new(Kind::Submenu {
        label: label.into(),
        open,
        anchor: Some(Box::new(anchor)),
    })
}
pub fn menu_item(label: impl Into<String>) -> View {
    View::new(Kind::MenuItem(label.into()))
}
pub fn button() -> View {
    View::new(Kind::Button)
}
/// A checkbox bound bidirectionally to `value`.
/// `label` supplies both its visible text and accessible name. The control owns
/// its indicator and label; additional `.child`/`.children` are rejected at mount.
/// An optional `.on_click` callback runs after the value has toggled.
pub fn checkbox(label: impl Into<String>, value: Signal<bool>) -> View {
    View::new(Kind::Checkbox {
        label: label.into(),
        value,
    })
}
/// A determinate progress indicator. Values normalize to 0..=1 (NaN becomes 0).
/// The accessible label is not drawn. This leaf owns its clipped fill;
/// `text_color` controls the fill color and `bg` controls the track.
pub fn progress(label: impl Into<String>, value: Signal<f32>) -> View {
    View::new(Kind::Progress {
        label: label.into(),
        value,
    })
}
/// A horizontal numeric control bound bidirectionally to `value`.
/// The finite range must be increasing. This leaf owns its rail and thumb;
/// `label` supplies the accessible name, and typography color styles the thumb.
pub fn slider(
    label: impl Into<String>,
    value: Signal<f32>,
    range: std::ops::RangeInclusive<f32>,
) -> View {
    assert!(
        range.start().is_finite() && range.end().is_finite() && range.start() < range.end(),
        "slider range must be finite and increasing"
    );
    View::new(Kind::Slider {
        label: label.into(),
        value,
        range,
    })
}
/// A single-line editor bound bidirectionally to `value`.
/// `label` is its accessible name; dimensions and typography use ordinary styles.
pub fn text_input(label: impl Into<String>, value: Signal<String>) -> View {
    View::new(Kind::Editor {
        label: label.into(),
        value,
        multiline: false,
    })
}
/// A multiline editor with selection, undo, clipboard and IME support.
pub fn text_area(label: impl Into<String>, value: Signal<String>) -> View {
    View::new(Kind::Editor {
        label: label.into(),
        value,
        multiline: true,
    })
}
pub fn component(build: impl FnOnce(&mut Context) -> View + 'static) -> View {
    View::new(Kind::Component(Box::new(build)))
}
pub fn provide<T: 'static>(value: T, child: View) -> View {
    provide_with(move |_| value, child)
}
pub fn provide_with<T: 'static>(
    factory: impl FnOnce(&mut Context) -> T + 'static,
    child: View,
) -> View {
    View::new(Kind::Provider(
        Box::new(move |cx| {
            let value = factory(cx);
            cx.environment.services.provide(value);
        }),
        Box::new(child),
    ))
}
/// A conditional region. Equal selector values retain their component and local state.
pub fn switch<K: PartialEq + Clone + 'static>(
    mut select: impl FnMut() -> K + 'static,
    mut build: impl FnMut(K, &mut Context) -> View + 'static,
) -> View {
    View::new(Kind::Region(Box::new(move |ui, root, environment| {
        let weak = ui.downgrade();
        let runtime = ui.runtime.clone();
        let mut selected: Option<K> = None;
        let mut mounted = None;
        ui.bind(root, move || {
            let key = select();
            if selected.as_ref() == Some(&key) {
                return;
            }
            let Some(mut ui) = weak.upgrade() else {
                return;
            };
            if !ui.scene.borrow().contains(root) {
                return;
            }
            runtime.untracked(|| {
                let mut cx = Context::new(runtime.clone(), environment.clone());
                let view = build(key.clone(), &mut cx);
                let mut transaction = MountTransaction::new(&ui, root);
                let node = mount(&mut ui, root, view, environment.clone());
                cx.finish(&mut ui, node);
                transaction.commit();
                if let Some(old) = mounted.replace(node) {
                    ui.remove(old);
                }
            });
            selected = Some(key);
        });
    })))
}
/// Keyed children retain their local state and subscriptions across reorder.
/// Duplicate keys are rejected before mutating the mounted children.
/// Constructor panics clean up new children and preserve the previous mounted
/// set. Application state written by a constructor is not rolled back.
pub fn keyed<K: Eq + std::hash::Hash + Clone + 'static>(
    mut keys: impl FnMut() -> Vec<K> + 'static,
    mut build: impl FnMut(K, &mut Context) -> View + 'static,
) -> View {
    View::new(Kind::Region(Box::new(move |ui, root, environment| {
        let weak = ui.downgrade();
        let runtime = ui.runtime.clone();
        let mut entries: HashMap<K, NodeId> = HashMap::new();
        ui.bind(root, move || {
            let keys = keys();
            let unique: std::collections::HashSet<_> = keys.iter().collect();
            assert_eq!(unique.len(), keys.len(), "duplicate component child key");
            let Some(mut ui) = weak.upgrade() else {
                return;
            };
            if !ui.scene.borrow().contains(root) {
                return;
            }
            runtime.untracked(|| {
                let mut transaction = MountTransaction::new(&ui, root);
                // Stage every new child before changing the committed registry
                // or removing old children. The mount guard disposes staged
                // roots if any later constructor panics.
                let mut added = Vec::new();
                for key in &keys {
                    if entries
                        .get(key)
                        .is_some_and(|node| ui.scene.borrow().contains(*node))
                    {
                        continue;
                    }
                    let mut cx = Context::new(runtime.clone(), environment.clone());
                    let view = build(key.clone(), &mut cx);
                    let node = mount(&mut ui, root, view, environment.clone());
                    cx.finish(&mut ui, node);
                    added.push((key.clone(), node));
                }
                entries.extend(added);
                entries.retain(|key, node| {
                    if unique.contains(key) {
                        true
                    } else {
                        ui.remove(*node);
                        false
                    }
                });
                let order: Vec<_> = keys.iter().map(|key| entries[key]).collect();
                assert!(ui.scene.borrow_mut().reorder_children(root, &order));
                transaction.commit();
            });
        });
    })))
}
/// Fixed-height keyed rows; only the visible range plus overscan is mounted.
/// `count` and `key` track reactive reads. `build` runs once per mounted key;
/// its index signal updates when that key moves. Offscreen rows are disposed,
/// so persistent row state belongs in the model. Keys must be unique among
/// visible rows. `offset` is a bidirectional logical-pixel scroll position.
pub fn virtual_list<K: Eq + std::hash::Hash + Clone + 'static>(
    offset: Signal<f32>,
    row_height: f32,
    overscan: usize,
    mut count: impl FnMut() -> usize + 'static,
    mut key: impl FnMut(usize) -> K + 'static,
    mut build: impl FnMut(Signal<usize>, K, &mut Context) -> View + 'static,
) -> View {
    assert!(row_height.is_finite() && row_height > 0.0);
    View::new(Kind::VirtualList(Box::new(
        move |ui, root, environment, scrollbar, keyboard_navigation| {
            let size = ui.observe_content_size(root);
            let extent = ui.signal(0.0_f32);
            // Clip rows to the allocated content box, keeping root padding clear.
            let viewport = ui.container(root, Layout::Overlay, Default::default());
            let content = ui.container(viewport, Layout::Overlay, Default::default());
            ui.scene.borrow_mut().set_scroll_copy(content, true);
            let navigation = keyboard_navigation.then(|| {
                crate::compose_virtual_keyboard::Navigation::new(
                    ui,
                    root,
                    offset.clone(),
                    size.clone(),
                    row_height,
                )
            });
            let wheel_offset = offset.clone();
            let wheel_size = size.clone();
            let wheel_extent = extent.clone();
            ui.on_event(root, false, move |cx| {
                if cx.phase == EventPhase::Capture || cx.default_prevented() {
                    return;
                }
                if let InputEvent::Scroll { delta_y, .. } = cx.event {
                    if !delta_y.is_finite() {
                        return;
                    }
                    let before = wheel_offset.get();
                    let after = (before + delta_y)
                        .max(0.0)
                        .min((wheel_extent.get() - wheel_size.get().1).max(0.0));
                    if before != after {
                        wheel_offset.set(after);
                        cx.prevent_default();
                        cx.stop_propagation();
                    }
                }
            });
            if scrollbar != ScrollbarStyle::Hidden {
                mount_scrollbar(
                    ui,
                    &environment,
                    root,
                    offset.clone(),
                    size.clone(),
                    extent.clone(),
                    false,
                    scrollbar,
                );
            }
            let weak = ui.downgrade();
            let runtime = ui.runtime.clone();
            let mut entries: HashMap<K, (NodeId, Signal<usize>)> = HashMap::new();
            let mut handled_request = 0;
            ui.bind(root, move || {
                let length = count();
                let (width, height) = size.get();
                let model = crate::virtual_list::VirtualList::new(length, row_height, overscan);
                let requested = offset.get();
                let mut position = if requested.is_finite() {
                    requested.max(0.0)
                } else {
                    0.0
                }
                .min((model.content_height() - height).max(0.0));
                let request = navigation
                    .as_ref()
                    .map(|navigation| navigation.request.get());
                let target = request
                    .filter(|(serial, _)| *serial != handled_request)
                    .and_then(|(_, index)| {
                        (length > 0).then_some(index.min(length.saturating_sub(1)))
                    });
                if let Some(index) = target {
                    let top = model.row_offset(index);
                    let bottom = (top as f64 + row_height as f64).min(f32::MAX as f64) as f32;
                    position = if top < position {
                        top
                    } else if bottom > position + height {
                        (bottom - height).max(0.)
                    } else {
                        position
                    };
                    position = position.min((model.content_height() - height).max(0.));
                }
                let range = model.visible_range(position, height);
                let keys: Vec<_> = range.map(|index| (key(index), index)).collect();
                let unique: std::collections::HashSet<_> =
                    keys.iter().map(|(key, _)| key).collect();
                assert_eq!(unique.len(), keys.len(), "duplicate virtual list child key");
                let Some(mut ui) = weak.upgrade() else {
                    return;
                };
                if !ui.scene.borrow().contains(root) {
                    return;
                }
                runtime.untracked(|| {
                    let mut transaction = MountTransaction::new(&ui, content);
                    // Construct first: a failed row constructor leaves old rows intact.
                    let mut added = Vec::new();
                    for (key, index) in &keys {
                        if entries
                            .get(key)
                            .is_some_and(|(node, _)| ui.scene.borrow().contains(*node))
                        {
                            continue;
                        }
                        let index_signal = ui.signal(*index);
                        let mut cx = Context::new(runtime.clone(), environment.clone());
                        let view = build(index_signal.clone(), key.clone(), &mut cx);
                        let row = ui.container(content, Layout::Overlay, Default::default());
                        let node = mount(&mut ui, row, view, environment.clone());
                        cx.finish(&mut ui, node);
                        if let Some(navigation) = &navigation {
                            navigation.mount_row(&mut ui, row, row_height);
                        }
                        added.push((key.clone(), (row, index_signal)));
                    }
                    entries.extend(added);
                    let mut removed_focus = false;
                    entries.retain(|key, (node, _)| {
                        if unique.contains(key) {
                            true
                        } else {
                            if navigation.is_some()
                                && let Some(focused) = ui.input.focused()
                            {
                                let scene = ui.scene.borrow();
                                removed_focus |= focused == *node
                                    || (scene.contains(focused)
                                        && scene
                                            .ancestors(focused)
                                            .any(|ancestor| ancestor == *node));
                            }
                            ui.remove(*node);
                            false
                        }
                    });
                    for (key, index) in &keys {
                        entries[key].1.set(*index);
                    }
                    {
                        let mut scene = ui.scene.borrow_mut();
                        scene.set_style(
                            viewport,
                            crate::scene::Style {
                                clip: true,
                                ..crate::widgets::fixed(width, height)
                            },
                        );
                        scene.set_style(
                            content,
                            crate::widgets::fixed(width, model.content_height()),
                        );
                        scene.set_transform(
                            content,
                            crate::scene::Transform {
                                x: 0.0,
                                y: -position,
                            },
                        );
                        for (key, index) in &keys {
                            let node = entries[key].0;
                            scene.set_style(node, crate::widgets::fixed(width, row_height));
                            scene.set_transform(
                                node,
                                crate::scene::Transform {
                                    x: 0.0,
                                    y: model.row_offset(*index),
                                },
                            );
                        }
                        let order: Vec<_> = keys.iter().map(|(key, _)| entries[key].0).collect();
                        assert!(scene.reorder_children(content, &order));
                    }
                    transaction.commit();
                    extent.set(model.content_height());
                    let unchanged_request = offset.get().to_bits() == requested.to_bits();
                    if unchanged_request {
                        offset.set(position);
                    }
                    if let Some(navigation) = &navigation {
                        navigation.update(
                            &ui,
                            root,
                            length,
                            keys.iter()
                                .map(|(key, index)| (*index, entries[key].0))
                                .collect(),
                        );
                        if let Some((serial, _)) = request {
                            handled_request = serial;
                        }
                        if let Some(index) = target.filter(|_| unchanged_request) {
                            if let Some((key, _)) =
                                keys.iter().find(|(_, candidate)| *candidate == index)
                            {
                                ui.input.focus(&ui.scene, Some(entries[key].0));
                            }
                        } else if removed_focus && ui.input.focused().is_none() {
                            ui.input.focus(&ui.scene, Some(root));
                        }
                    }
                });
            });
        },
    )))
}
// Roll back newly appended roots if a component constructor or initial binding
// panics. Existing siblings are never removed by this guard. This is ownership
// cleanup, not rollback of arbitrary application state writes.
struct MountTransaction {
    ui: WeakUi,
    parent: NodeId,
    before: Vec<NodeId>,
    committed: bool,
}
impl MountTransaction {
    fn new(ui: &Ui, parent: NodeId) -> Self {
        Self {
            ui: ui.downgrade(),
            parent,
            before: ui.scene.borrow().children(parent).to_vec(),
            committed: false,
        }
    }
    fn commit(&mut self) {
        self.committed = true;
    }
}
impl Drop for MountTransaction {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let Some(mut ui) = self.ui.upgrade() else {
            return;
        };
        if !ui.scene.borrow().contains(self.parent) {
            return;
        }
        let children = ui.scene.borrow().children(self.parent).to_vec();
        for child in children {
            if !self.before.contains(&child) {
                ui.remove(child);
            }
        }
    }
}

struct IdRegistration {
    registry: Registry,
    id: String,
    node: NodeId,
}
impl Drop for IdRegistration {
    fn drop(&mut self) {
        let mut registry = self.registry.borrow_mut();
        if let Some(nodes) = registry.get_mut(&self.id) {
            nodes.retain(|id| *id != self.node);
            if nodes.is_empty() {
                registry.remove(&self.id);
            }
        }
    }
}
fn mount(ui: &mut Ui, parent: NodeId, mut view: View, mut environment: Environment) -> NodeId {
    // Extract closures while retaining the root refinements on their wrapper.
    let kind = std::mem::replace(&mut view.kind, Kind::Container(Layout::Column));
    match kind {
        Kind::Component(build) => {
            let mut cx = Context::new(ui.runtime.clone(), environment.clone());
            let inner = ui.runtime.untracked(|| build(&mut cx));
            let root = mount(ui, parent, view.refine(inner), environment);
            cx.finish(ui, root);
            root
        }
        Kind::Provider(provide, child) => {
            environment.services = environment.services.child();
            let mut cx = Context::new(ui.runtime.clone(), environment);
            ui.runtime.untracked(|| provide(&mut cx));
            let environment = cx.environment.clone();
            let root = mount(ui, parent, view.refine(*child), environment);
            cx.finish(ui, root);
            root
        }
        Kind::Slot(slot) => {
            environment.services = slot.services;
            let mut cx = Context::new(ui.runtime.clone(), environment.clone());
            let inner = ui.runtime.untracked(|| (slot.build)(&mut cx));
            let root = mount(ui, parent, view.refine(inner), environment);
            cx.finish(ui, root);
            root
        }
        Kind::Modal {
            label,
            open,
            anchor,
        } => mount_portal(
            ui,
            parent,
            label,
            open,
            anchor,
            view,
            environment,
            false,
            false,
        ),
        Kind::Menu {
            label,
            open,
            anchor,
        } => mount_portal(
            ui,
            parent,
            label,
            open,
            Some(anchor),
            view,
            environment,
            true,
            false,
        ),
        Kind::Submenu { label, open, anchor } => {
            assert!(
                environment.menu.is_some(),
                "submenu requires an enclosing menu"
            );
            let mut anchor = anchor.map_or_else(|| menu_item(label.clone()), |view| *view);
            anchor.menu_trigger = Some(open.clone());
            if let Some(id) = view.trigger_id.take() {
                anchor.id = Some(id);
            }
            mount_portal(
                ui,
                parent,
                label,
                open,
                Some(Box::new(anchor)),
                view,
                environment,
                true,
                true,
            )
        }
        primitive => mount_element(ui, parent, primitive, view, environment),
    }
}
#[allow(clippy::too_many_arguments)]
fn mount_portal(
    ui: &mut Ui,
    parent: NodeId,
    label: String,
    open: Signal<bool>,
    anchor: Option<Box<View>>,
    mut view: View,
    mut environment: Environment,
    is_menu: bool,
    is_submenu: bool,
) -> NodeId {
    let anchored = anchor.is_some();
    let external_anchor = view.anchor_above.clone();
    let presentation = view.popup_motion.take().map(|policy| {
        let mut cx = Context::new(ui.runtime.clone(), environment.clone());
        let presentation = crate::compose_modal::Presentation::new(&mut cx, policy);
        (cx, presentation)
    });
    let owner = ui.container(
        parent,
        Layout::Overlay,
        crate::scene::Style {
            absolute: !anchored,
            align: if is_submenu {
                crate::scene::Align::Stretch
            } else {
                crate::scene::Align::Start
            },
            width: (!anchored).then_some(0.),
            height: (!anchored).then_some(0.),
            ..Default::default()
        },
    );
    if is_submenu && let Some(mut disabled) = view.disabled.take() {
        let weak = ui.downgrade();
        ui.bind(owner, move || {
            let disabled = disabled();
            if let Some(mut ui) = weak.upgrade() {
                ui.set_disabled(owner, disabled);
            }
        });
    }
    let anchor = anchor.map(|anchor| mount(ui, owner, *anchor, environment.clone()));
    let portal_parent = environment
        .portal
        .as_ref()
        .map_or(ui.root(), |(node, _)| *node);
    let parent_visible = environment
        .portal
        .as_ref()
        .map(|(_, visible)| visible.clone());
    let portal = crate::compose_modal::create(ui, owner, portal_parent);
    if !anchored {
        ui.scene.borrow_mut().set_kind(
            portal.overlay,
            NodeKind::Panel {
                layout: Layout::Overlay,
                quad: crate::scene::QuadStyle {
                    fill: crate::scene::Color(0, 0, 0, 130),
                    ..Default::default()
                },
            },
        );
    }
    environment.portal = Some((portal.overlay, portal.visible.clone()));
    let menu_context = is_menu.then(|| {
        crate::compose_menu::MenuContext::new(
            open.clone(),
            environment.menu.clone(),
            anchor,
            is_submenu,
        )
    });
    if let Some(menu) = &menu_context {
        environment.menu = Some(menu.clone());
    }

    let mut defaults = Styles::new()
        .w(if anchored { 240. } else { 360. })
        .p(16.)
        .bg(ui.theme.surface)
        .rounded(ui.theme.radius)
        .overflow_hidden();
    if is_menu {
        defaults = defaults.items_stretch();
    }
    defaults.merge(&view.styles);
    view.styles = defaults;
    let has_max_width = view.styles.max_width.is_some();
    let has_max_height = view.styles.max_height.is_some();
    let viewport = ui.observe_bounds(ui.root());
    let mut dynamic = view.reactive_style.take();
    view.reactive_style = Some(Box::new(move || {
        let mut styles = dynamic
            .as_mut()
            .map_or_else(Styles::new, |compute| compute());
        let bounds = viewport.get();
        let margin = if anchored { 8. } else { 32. };
        if !has_max_width && styles.max_width.is_none() {
            styles.max_width = Some((bounds.width - margin).max(0.));
        }
        if !has_max_height && styles.max_height.is_none() {
            styles.max_height = Some((bounds.height - margin).max(0.));
        }
        styles
    }));
    let dismiss_backdrop = view.dismiss_backdrop.take().unwrap_or(true);
    let panel_focusable = view.focusable.unwrap_or(true);
    let layout_target = is_menu.then(|| ui.signal(None));
    view.layout_target = layout_target.clone();
    let menu_children = if is_menu {
        std::mem::take(&mut view.children)
    } else {
        Vec::new()
    };
    let panel = mount_element(
        ui,
        portal.position,
        Kind::Container(Layout::Column),
        view,
        environment.clone(),
    );
    if let Some(menu) = menu_context.clone() {
        menu.panel.set(Some(panel));
        let viewport = ui.container(
            panel,
            Layout::Overlay,
            crate::scene::Style {
                clip: true,
                flex_shrink: 1.,
                flex_grow: 1.,
                ..Default::default()
            },
        );
        let content = ui.container(
            viewport,
            Layout::Column,
            crate::scene::Style {
                flex_shrink: 0.,
                align: crate::scene::Align::Stretch,
                ..Default::default()
            },
        );
        layout_target.as_ref().unwrap().set(Some(content));
        for child in menu_children {
            mount(ui, content, child, environment.clone());
        }
        let mut scrollbar_context = Context::new(ui.runtime.clone(), environment.clone());
        crate::compose_menu_scroll::mount(ui, &mut scrollbar_context, panel, viewport, content);
        scrollbar_context.finish(ui, viewport);
        crate::compose_menu::mount(
            ui,
            panel,
            menu,
            anchor.expect("menu anchor"),
            portal.overlay,
            dismiss_backdrop,
            external_anchor.clone(),
        );
    }
    if let Some(anchor) = anchor {
        if let Some(bounds) = external_anchor {
            crate::compose_popover::mount_above(
                ui,
                owner,
                bounds,
                portal.overlay,
                portal.position,
                panel,
            );
        } else if is_submenu {
            crate::compose_popover::mount_submenu(
                ui,
                owner,
                anchor,
                portal.overlay,
                portal.position,
                panel,
            );
        } else {
            crate::compose_popover::mount(
                ui,
                owner,
                anchor,
                portal.overlay,
                portal.position,
                panel,
            );
        }
    } else {
        crate::compose_modal::center(ui, owner, portal.position);
    }
    let presentation = presentation.map(|(cx, presentation)| {
        cx.finish(ui, owner);
        presentation
    });
    crate::compose_modal::activate(
        ui,
        owner,
        &portal,
        panel,
        label,
        open,
        parent_visible,
        dismiss_backdrop,
        if is_menu { Role::Menu } else { Role::Dialog },
        menu_context
            .as_ref()
            .and_then(|menu| menu.parent.as_ref())
            .and_then(|parent| parent.panel.get()),
        if is_submenu { anchor } else { None },
        panel_focusable,
        presentation,
    );
    owner
}
fn mount_element(
    ui: &mut Ui,
    parent: NodeId,
    kind: Kind,
    mut view: View,
    mut environment: Environment,
) -> NodeId {
    let kind = match kind {
        Kind::RichText(mut read) => Kind::DisplayText(Box::new(move || DisplayText::Rich(read()))),
        kind => kind,
    };
    assert!(
        view.trigger_id.is_none(),
        "trigger_id is only supported on submenu views"
    );
    assert!(
        view.dismiss_backdrop.is_none(),
        "dismiss_on_backdrop is only supported on modal and popover views"
    );
    assert!(
        view.keyboard_navigation.is_none() || matches!(kind, Kind::VirtualList(_)),
        "keyboard_navigation is only supported on virtual_list views"
    );
    let keyboard_navigation = view.keyboard_navigation.unwrap_or(false);
    assert!(
        view.scrollbar.is_none() || matches!(kind, Kind::Scroll { .. } | Kind::VirtualList(_)),
        "scrollbar is only supported on scroll, scroll_x, and virtual_list views"
    );
    let scrollbar = view.scrollbar.unwrap_or_default();
    // Decorated/padded text gets an actual layout box. Plain text stays a single
    // node, keeping common reactive labels small. Dynamic/state styles may add
    // decoration later, so reserve that box at mount rather than remount on hover.
    if matches!(kind, Kind::Text(_) | Kind::BoundText(_)) {
        assert!(
            view.children.is_empty(),
            "text views cannot contain child views"
        );
        let boxed = view.styles.padding.is_some()
            || view.styles.background.is_some()
            || view.styles.radius.is_some()
            || view.styles.border_width.is_some()
            || view.styles.shadow.is_some()
            || view.reactive_style.is_some()
            || view.variants.hover.is_some()
            || view.variants.active.is_some()
            || view.variants.focus.is_some()
            || view.variants.disabled.is_some();
        if boxed {
            let label = View::new(kind);
            view.children.push(label);
            return mount_element(
                ui,
                parent,
                Kind::Container(Layout::Column),
                view,
                environment,
            );
        }
    }
    let is_scroll = matches!(kind, Kind::Scroll { .. });
    let mut scroll_offset = None;
    let is_image = matches!(kind, Kind::Image { .. });
    #[cfg(target_os = "macos")]
    let is_image = is_image || matches!(kind, Kind::NativeSurface { .. });
    #[cfg(target_os = "macos")]
    let mut native_source = None;
    let mut image_source = None;
    let mut image_intrinsic = None;
    let is_menu_item = matches!(kind, Kind::MenuItem(_));
    let is_submenu_trigger = is_menu_item && view.menu_trigger.is_some();
    let item_menu = is_menu_item.then(|| {
        environment
            .menu
            .clone()
            .expect("menu_item requires an enclosing menu")
    });
    let is_editor = matches!(kind, Kind::Editor { .. });
    assert!(
        view.read_only.is_none() || is_editor,
        "read_only is only supported on editor views"
    );
    if is_editor {
        assert!(
            view.children.is_empty(),
            "editor views cannot contain child views"
        );
    }
    let is_slider = matches!(kind, Kind::Slider { .. });
    let is_progress = matches!(kind, Kind::Progress { .. });
    let mut slider_state = None;
    let mut progress_state = None;
    let is_checkbox = matches!(kind, Kind::Checkbox { .. });
    let interactive = matches!(
        kind,
        Kind::Button | Kind::Checkbox { .. } | Kind::MenuItem(_)
    ) || view.click.is_some();
    let mut checkbox_value = None;
    let mut editor = None;
    let mut region = None;
    let mut compute = None;
    let mut rich_compute = None;
    let mut rich_paragraph = None;
    let mut canvas_compute = None;
    let mut svg_compute = None;
    let root = match kind {
        Kind::Canvas(draw) => {
            assert!(view.children.is_empty(), "canvas views own their drawing");
            canvas_compute = Some(draw);
            ui.container(parent, Layout::Overlay, Default::default())
        }
        Kind::Svg { label, source } => {
            assert!(view.children.is_empty(), "SVG views own their artwork");
            let root = ui.container(parent, Layout::Overlay, Default::default());
            ui.semantics
                .borrow_mut()
                .set(root, SemanticNode::new(Role::Image, label));
            svg_compute = Some(source);
            root
        }
        #[cfg(target_os = "macos")]
        Kind::NativeSurface { label, mut source } => {
            assert!(view.children.is_empty(), "native surfaces own their pixels");
            let root = ui.container(parent, Layout::Overlay, Default::default());
            ui.semantics
                .borrow_mut()
                .set(root, SemanticNode::new(Role::Image, label));
            let intrinsic = ui.signal((0., 0.));
            let intrinsic_write = intrinsic.clone();
            let data = ui.signal(None);
            let write = data.clone();
            ui.bind(root, move || {
                let frame = source();
                intrinsic_write.set((frame.width() as f32, frame.height() as f32));
                write.set(Some(frame));
            });
            native_source = Some(data);
            image_intrinsic = Some(intrinsic);
            root
        }
        Kind::Image { label, mut source } => {
            assert!(view.children.is_empty(), "image views own their pixels");
            let root = ui.container(parent, Layout::Overlay, Default::default());
            ui.semantics
                .borrow_mut()
                .set(root, SemanticNode::new(Role::Image, label));
            let intrinsic = ui.signal((0., 0.));
            let intrinsic_write = intrinsic.clone();
            let data = ui.signal(None);
            let data_write = data.clone();
            ui.bind(root, move || {
                let next = source();
                intrinsic_write.set((next.width() as f32, next.height() as f32));
                data_write.set(Some(next));
            });
            image_source = Some(data);
            image_intrinsic = Some(intrinsic);
            root
        }
        Kind::DisplayText(binding) => {
            assert!(
                view.children.is_empty(),
                "rich text uses inline TextSpan children"
            );
            rich_compute = Some(binding);
            let root = ui.container(parent, Layout::Overlay, Default::default());
            let paragraph = ui.label(
                root,
                "",
                crate::scene::Style {
                    height_percent: Some(1.),
                    ..Default::default()
                },
            );
            rich_paragraph = Some(paragraph);
            ui.scene.borrow_mut().set_kind(
                paragraph,
                NodeKind::RichText {
                    text: std::sync::Arc::new(
                        crate::rich_text::RichText::new("", Vec::new()).unwrap(),
                    ),
                },
            );
            root
        }
        Kind::Text(text) => ui.label(parent, text, Default::default()),
        Kind::BoundText(binding) => {
            compute = Some(binding);
            ui.label(parent, "", Default::default())
        }
        Kind::Container(layout) => ui.container(parent, layout, Default::default()),
        Kind::MenuItem(label) => {
            assert!(view.children.is_empty(), "menu items own their label");
            let mut defaults = Styles::new()
                .px(12.)
                .py(8.)
                .items_center()
                .bg(ui.theme.surface);
            defaults.merge(&view.styles);
            view.styles = defaults;
            if view.variants.hover.is_none() {
                view.variants.hover = Some(Styles::new().bg(ui.theme.hover));
            }
            if view.variants.focus.is_none() {
                view.variants.focus = Some(
                    Styles::new()
                        .bg(ui.theme.hover)
                        .border(1.)
                        .border_color(ui.theme.accent),
                );
            }
            let root = ui.container(parent, Layout::Row, Default::default());
            let mut semantic = SemanticNode::new(Role::MenuItem, label.clone());
            semantic.checked = view.menu_checked;
            ui.semantics.borrow_mut().set(root, semantic);
            if is_submenu_trigger {
                view.children.push(text(label).flex_grow(1.));
            } else {
                view.children.push(text(label));
            }
            root
        }
        Kind::Button => {
            // Definite dimensions already reserve the control's hit area. Auto
            // padding must not squeeze fixed-height labels or square icons.
            let fixed_height = view.styles.height.is_some() || view.styles.height_percent.is_some();
            let square = view
                .styles
                .width
                .zip(view.styles.height)
                .is_some_and(|(w, h)| w > 0. && w == h);
            let single_content = view
                .children
                .iter()
                .filter(|child| !child.styles.absolute.unwrap_or(false))
                .count()
                == 1;
            let mut defaults = Styles::new()
                .px(if square { 0. } else { 12. })
                .py(if fixed_height { 0. } else { 8. })
                .gap(6.)
                .items_center()
                .bg(ui.theme.surface)
                .rounded(ui.theme.radius);
            if single_content {
                defaults = defaults.justify_center();
            }
            defaults.merge(&view.styles);
            view.styles = defaults;
            if view.variants.hover.is_none() {
                view.variants.hover = Some(Styles::new().bg(ui.theme.hover));
            }
            if view.variants.focus.is_none() {
                view.variants.focus = Some(Styles::new().border(1.).border_color(ui.theme.accent));
            }
            ui.container(parent, Layout::Row, Default::default())
        }
        Kind::Checkbox { label, value } => {
            assert!(
                view.children.is_empty(),
                "checkbox views own their indicator and label"
            );
            let mut defaults = Styles::new().gap(6.).items_center().px(4.).py(4.);
            defaults.merge(&view.styles);
            view.styles = defaults;
            if view.variants.hover.is_none() {
                view.variants.hover = Some(Styles::new().bg(ui.theme.hover));
            }
            if view.variants.focus.is_none() {
                view.variants.focus = Some(Styles::new().border(1.).border_color(ui.theme.accent));
            }
            let root = ui.container(parent, Layout::Row, Default::default());
            let mut semantics = SemanticNode::new(Role::CheckBox, label.clone());
            semantics.checked = Some(value.get());
            ui.semantics.borrow_mut().set(root, semantics);
            let checked = value.clone();
            let semantic_tree = ui.semantics.clone();
            ui.bind(root, move || {
                let checked = checked.get();
                semantic_tree
                    .borrow_mut()
                    .update(root, |node| node.checked = Some(checked));
            });
            let checked = value.clone();
            view.children
                .push(text_signal(move || if checked.get() { "☑" } else { "☐" }.into()).min_w(24.));
            view.children.push(text(label));
            checkbox_value = Some(value);
            root
        }
        Kind::Progress { label, value } => {
            assert!(view.children.is_empty(), "progress views own their fill");
            let mut defaults = Styles::new().w(240.).h(8.).bg(ui.theme.surface);
            defaults.merge(&view.styles);
            view.styles = defaults;
            let root = ui.container(parent, Layout::Overlay, Default::default());
            let mut semantic = SemanticNode::new(Role::Progress, label);
            semantic.min = Some(0.);
            semantic.max = Some(1.);
            ui.semantics.borrow_mut().set(root, semantic);
            progress_state = Some(value);
            root
        }
        Kind::Slider {
            label,
            value,
            range,
        } => {
            assert!(
                view.children.is_empty(),
                "slider views own their rail and thumb"
            );
            let mut defaults = Styles::new()
                .w(240.)
                .h(ui.theme.control_height)
                .px(8.)
                .py(6.)
                .bg(ui.theme.surface)
                .rounded(ui.theme.radius);
            defaults.merge(&view.styles);
            view.styles = defaults;
            if view.variants.focus.is_none() {
                view.variants.focus = Some(Styles::new().border(1.).border_color(ui.theme.accent));
            }
            let root = ui.container(parent, Layout::Overlay, Default::default());
            let mut semantic = SemanticNode::new(Role::Slider, label);
            semantic.min = Some(*range.start() as f64);
            semantic.max = Some(*range.end() as f64);
            ui.semantics.borrow_mut().set(root, semantic);
            slider_state = Some((value, range));
            root
        }
        Kind::Editor {
            label,
            value,
            multiline,
        } => {
            let mut defaults = Styles::new().bg(ui.theme.surface).rounded(ui.theme.radius);
            defaults.merge(&view.styles);
            view.styles = defaults;
            // The caret/selection indicate editing focus. A composed field
            // can opt into a border with its own explicit focus variant.
            let handle = ui.text_input_styled(parent, label, value, multiline);
            if let Some((color, radius, padding)) = view.selection_style {
                handle.set_selection_style(color, radius, padding);
            }
            let root = handle.node;
            editor = Some(handle);
            root
        }
        Kind::Scroll { offset, horizontal } => {
            let fit = view.scroll_fit_cross_axis.unwrap_or(false);
            let mut defaults = Styles::new().overflow_hidden();
            if horizontal || !fit { defaults = defaults.w(320.); }
            if !horizontal || !fit { defaults = defaults.h(240.); }
            defaults.merge(&view.styles);
            view.styles = defaults;
            let root = ui.container(parent, Layout::Overlay, Default::default());
            let mut semantics = SemanticNode::new(Role::ScrollView, "");
            semantics.scroll_axis = Some(if horizontal {
                crate::semantics::ScrollAxis::Horizontal
            } else {
                crate::semantics::ScrollAxis::Vertical
            });
            ui.semantics.borrow_mut().set(root, semantics);
            scroll_offset = Some((offset, horizontal));
            root
        }
        Kind::VirtualList(build) => {
            assert!(
                view.children.is_empty(),
                "virtual lists build children through their row factory"
            );
            let mut defaults = Styles::new().w(320.).h(240.).overflow_hidden();
            defaults.merge(&view.styles);
            view.styles = defaults;
            if keyboard_navigation && view.variants.focus.is_none() {
                view.variants.focus = Some(Styles::new().border(1.).border_color(ui.theme.accent));
            }
            region = Some(Box::new(move |ui: &mut Ui, root, environment| {
                build(ui, root, environment, scrollbar, keyboard_navigation)
            }) as RegionBuilder);
            let root = ui.container(parent, Layout::Overlay, Default::default());
            ui.semantics
                .borrow_mut()
                .set(root, SemanticNode::new(Role::ScrollView, ""));
            root
        }
        Kind::Region(build) => {
            region = Some(build);
            ui.container(parent, Layout::Column, Default::default())
        }
        _ => unreachable!(),
    };
    let focusable = view
        .focusable
        .unwrap_or(interactive || is_editor || is_slider || keyboard_navigation);
    // Prepend in reverse so declaration order is preserved even when a primitive
    // constructor (such as an editor) already registered its default handlers.
    if let Some(preview) = view.drag_preview {
        crate::compose_drag::mount(ui, root, preview);
    }
    for callback in view.events.into_iter().rev() {
        let binding = ui.input.listen_first(root, callback);
        ui.retain(root, binding);
    }
    if !view.keymap.bindings.is_empty()
        || !view.keymap.contexts.is_empty()
        || !view.keymap.context.entries.is_empty()
    {
        let binding = ui.input.bind_keys(root, view.keymap);
        ui.retain(root, binding);
    }
    if interactive {
        if !is_editor
            && !is_checkbox
            && !is_slider
            && !is_progress
            && !is_image
            && !is_scroll
            && !is_menu_item
        {
            ui.semantics
                .borrow_mut()
                .set(root, SemanticNode::new(Role::Button, ""));
        }
        let mut click = view.click.take();
        let menu_trigger = view.menu_trigger.take();
        ui.on_event(root, true, move |cx| {
            if cx.phase != EventPhase::Capture
                && matches!(cx.event, InputEvent::Activate)
                && !cx.default_prevented()
            {
                // A styled child may be the hit target. Its activation belongs
                // to the nearest clickable owner, not every outer button.
                cx.stop_propagation();
                if let Some(value) = &checkbox_value {
                    value.set(!value.get());
                }
                if let Some(open) = &menu_trigger {
                    open.set(true);
                } else if let Some(menu) = &item_menu {
                    menu.close_chain();
                }
                if let Some(click) = &mut click {
                    click();
                }
            }
        });
    }
    let scroll_layout_target = is_scroll.then(|| view.layout_target.clone().unwrap_or_else(|| ui.signal(None)));
    if is_scroll {
        view.layout_target = scroll_layout_target.clone();
    }
    let image_fit = is_image.then(|| ui.signal(crate::style::ObjectFit::default()));
    let projection = view.layout_motion.map(|transition| {
        let mut cx = Context::new(ui.runtime.clone(), environment.clone());
        let signal =
            crate::motion::project_layout(ui, root, &mut cx, transition, view.layout_id.take());
        cx.finish(ui, root);
        signal
    });
    assert!(view.layout_id.is_none(), "layout_id requires layout_motion");
    environment.typography = mount_style_with_intrinsic(
        ui,
        root,
        view.styles,
        environment.typography,
        view.reactive_style,
        view.variants,
        view.disabled,
        focusable,
        image_intrinsic,
        view.layout_target,
        image_fit.clone(),
        projection,
    );
    if let Some(mut compute) = rich_compute {
        let paragraph = rich_paragraph.unwrap();
        let content_state = ui.signal(crate::compose_rich::Content {
            rich: std::sync::Arc::new(crate::rich_text::RichText::new("", Vec::new()).unwrap()),
            links: Vec::new(),
        });
        crate::compose_rich::mount_links(ui, root, paragraph, content_state.clone());
        let weak = ui.downgrade();
        let typography = environment.typography.clone();
        let mut previous_plain = false;
        let mut previous_wrap = None;
        ui.bind(root, move || {
            let base = typography.get();
            let value = compute();
            let plain = matches!(&value, DisplayText::Plain(_));
            let spans = match value {
                DisplayText::Plain(text) => vec![text_span(text)],
                DisplayText::Rich(spans) => spans,
            };
            let mut content = String::new();
            let mut runs = Vec::new();
            let mut links = Vec::new();
            for span in spans {
                if span.text.is_empty() {
                    continue;
                }
                let start = content.len();
                content.push_str(&span.text);
                if let Some(action) = span.action {
                    links.push(crate::compose_rich::Link {
                        range: start..content.len(),
                        action,
                    });
                }
                let mut font = base.font.clone();
                if let Some(value) = span.styles.font_fallbacks {
                    font.fallbacks = value;
                }
                if let Some(value) = span.styles.font_features {
                    font.features = value;
                }
                if let Some(value) = span.styles.font_family {
                    font.family = value;
                }
                if let Some(value) = span.styles.font_weight {
                    font.weight = value;
                }
                if let Some(value) = span.styles.italic {
                    font.italic = value;
                }
                if let Some(value) = span.styles.line_height {
                    font.line_height = value;
                }
                if let Some(value) = span.styles.letter_spacing {
                    font.letter_spacing = value;
                }
                runs.push(crate::rich_text::TextRun {
                    background: span.background,
                    underline: span.underline,
                    strikethrough: span.strikethrough,
                    range: start..content.len(),
                    font,
                    font_size: span.styles.text_size.unwrap_or(base.size),
                    color: span.styles.text_color.unwrap_or(base.color),
                });
            }
            let rich = crate::rich_text::RichText::new(content.as_str(), runs)
                .expect("normalized inline spans")
                .with_options(base.display);
            // Recomputed but unchanged (its inputs animate for a sibling):
            // nothing to lay out, paint, describe or relabel.
            if plain == previous_plain
                && previous_wrap == Some(base.wrap)
                && content_state
                    .with_untracked(|current| *current.rich == rich && current.links == links)
            {
                return;
            }
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if !ui.scene.borrow().contains(root) {
                return;
            }
            let rich = std::sync::Arc::new(rich);
            {
                let mut scene = ui.scene.borrow_mut();
                let mut style = scene.style(paragraph);
                style.text_wrap = base.wrap;
                style.text_options = if plain {
                    base.display
                } else {
                    Default::default()
                };
                scene.set_style(paragraph, style);
                scene.set_font(paragraph, base.font.clone());
                scene.set_kind(
                    paragraph,
                    if plain {
                        NodeKind::Text {
                            text: rich.text_arc(),
                            font_size: base.size,
                            color: base.color,
                        }
                    } else {
                        NodeKind::RichText { text: rich.clone() }
                    },
                );
            }
            previous_plain = plain;
            previous_wrap = Some(base.wrap);
            content_state.set(crate::compose_rich::Content { rich, links });
            ui.semantics.borrow_mut().update(paragraph, |node| {
                node.label = content.clone();
                node.value = Some(content);
            });
            update_button_labels(&ui, root);
        });
    }
    if let Some(mut source) = svg_compute {
        let child = ui.container(
            root,
            Layout::Overlay,
            crate::scene::Style {
                absolute: true,
                ..Default::default()
            },
        );
        let size = ui.observe_content_size_deferred(root);
        let scene = ui.scene.clone();
        ui.bind(root, move || {
            let (width, height) = size.get();
            if !width.is_finite() || !height.is_finite() {
                return;
            }
            let data = source();
            let mut scene = scene.borrow_mut();
            scene.set_kind(child, NodeKind::Svg(data));
            scene.set_style(
                child,
                crate::scene::Style {
                    width: Some(width),
                    height: Some(height),
                    absolute: true,
                    ..Default::default()
                },
            );
        });
    }
    if let Some(mut draw) = canvas_compute {
        let size = ui.observe_content_size_deferred(root);
        let drawing = ui.scene.borrow_mut().append(
            root,
            NodeKind::Canvas(Default::default()),
            crate::scene::Style {
                absolute: true,
                ..Default::default()
            },
        );
        let scene = ui.scene.clone();
        ui.bind(root, move || {
            let (width, height) = size.get();
            if !width.is_finite() || !height.is_finite() {
                return;
            }
            let content = if width > 0. && height > 0. {
                draw((width, height))
            } else {
                crate::canvas::Canvas::new()
            };
            let mut scene = scene.borrow_mut();
            scene.set_kind(drawing, NodeKind::Canvas(std::sync::Arc::new(content)));
            scene.set_style(
                drawing,
                crate::scene::Style {
                    width: Some(width),
                    height: Some(height),
                    absolute: true,
                    ..Default::default()
                },
            );
        });
    }
    #[cfg(target_os = "macos")]
    if let Some(source) = native_source {
        mount_native_surface(
            ui,
            root,
            source,
            image_fit.clone().expect("surface fitting"),
        );
    }
    if let Some(source) = image_source {
        mount_image(ui, root, source.clone(), image_fit.expect("image fitting"));
        if let Some(animation) = view.animation.take() {
            animation::mount(ui, root, source, animation, &environment);
        }
    }
    if let Some(value) = progress_state {
        mount_progress(ui, root, value, environment.typography.clone());
    }
    if let Some((value, range)) = slider_state {
        mount_slider(ui, root, value, range, environment.typography.clone(), view.slider_chrome.unwrap_or(true), view.slider_step, view.slider_thumb_width.unwrap_or(10.));
    }
    if let Some(editor) = editor {
        if let Some(select) = view.select_all_on_focus {
            editor.set_select_all_on_focus(select);
        }
        if let Some(callback) = view.on_editor.take() {
            callback(editor.clone());
        }
        if let Some(mut read_only) = view.read_only.take() {
            let editor = editor.clone();
            ui.bind(root, move || editor.set_read_only(read_only()));
        }
        let typography = environment.typography.clone();
        ui.bind(root, move || {
            let style = typography.get();
            editor.set_text_style(style.color, style.size, style.font, style.wrap);
        });
    }
    if let Some(id) = view.id {
        ui.scene.borrow().profiler().label(root, id.clone());
        environment
            .registry
            .borrow_mut()
            .entry(id.clone())
            .or_default()
            .push(root);
        ui.retain(
            root,
            IdRegistration {
                registry: environment.registry.clone(),
                id,
                node: root,
            },
        );
    }
    let scroll_nodes = scroll_offset.as_ref().map(|(_, horizontal)| {
        let viewport = ui.container(root, Layout::Overlay, Default::default());
        let content = ui.container(
            viewport,
            if *horizontal {
                Layout::Row
            } else {
                Layout::Column
            },
            Default::default(),
        );
        ui.scene.borrow_mut().set_scroll_copy(content, true);
        scroll_layout_target.as_ref().unwrap().set(Some(content));
        (viewport, content)
    });
    let child_parent = scroll_nodes.map_or(root, |(_, content)| content);
    for child in view.children {
        mount(ui, child_parent, child, environment.clone());
    }
    if is_submenu_trigger {
        let indicator = mount(ui, root, text("›"), environment.clone());
        ui.semantics.borrow_mut().remove(indicator);
    }
    if let (Some((offset, horizontal)), Some((viewport, content))) = (scroll_offset, scroll_nodes) {
        mount_scroll(
            ui,
            root,
            viewport,
            content,
            offset,
            horizontal,
            scrollbar,
            view.scroll_progress,
            view.scroll_fades,
            view.scroll_fit_cross_axis.unwrap_or(false),
            view.horizontal_wheel.unwrap_or(false),
            &environment,
        );
    }
    if let Some(mut compute) = compute {
        let weak = ui.downgrade();
        ui.bind(root, move || {
            let value = compute();
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if !ui.scene.borrow().contains(root) {
                return;
            }
            ui.scene.borrow_mut().set_text(root, value.as_str());
            ui.semantics
                .borrow_mut()
                .update(root, |node| node.label = value.clone());
            update_button_labels(&ui, root);
        });
    }
    if let Some(build) = region {
        build(ui, root, environment);
    }
    let mut options = ui.input.options(root).unwrap_or_default();
    options.focusable = focusable;
    if view.focusable.is_some() || ui.input.options(root).is_some() {
        ui.input.set_options(root, options);
    }
    update_button_labels(ui, root);
    if let Some(mut update) = view.accessibility {
        if ui.semantics.borrow().get(root).is_none() {
            ui.semantics.borrow_mut().set(root, SemanticNode::new(Role::Group, ""));
        }
        let semantics = ui.semantics.clone();
        ui.bind(root, move || {
            semantics.borrow_mut().update(root, |node| {
                update(node);
                node.explicit_label = true;
            });
        });
    }
    if !view.visibility.is_empty() {
        let visible = ui.observe_visibility(root);
        let presented = ui.observe_presentation();
        let observers = view.visibility;
        ui.bind(root, move || {
            let visible = visible.get();
            let presented = presented.get();
            for observer in &observers {
                observer.set(visible && presented);
            }
        });
    }
    if !view.bounds.is_empty() {
        let bounds = ui.observe_bounds(root);
        let observers = view.bounds;
        ui.bind(root, move || {
            let bounds = bounds.get();
            for observer in &observers {
                observer.set(bounds);
            }
        });
    }
    if !view.layout_bounds.is_empty() {
        let bounds = ui.observe_layout_bounds(root);
        let observers = view.layout_bounds;
        ui.bind(root, move || {
            let bounds = bounds.get();
            for observer in &observers {
                observer.set(bounds);
            }
        });
    }
    if ui.scene.borrow().contains(root) {
        for callback in view.mounted {
            callback(root);
        }
    }
    root
}
#[allow(clippy::too_many_arguments)]
fn mount_scroll(
    ui: &mut Ui,
    root: NodeId,
    viewport: NodeId,
    content: NodeId,
    offset: Signal<f32>,
    horizontal: bool,
    scrollbar: ScrollbarStyle,
    progress: Option<Signal<f32>>,
    fade_band: Option<(f32, Rc<dyn Fn() -> std::time::Duration>)>,
    fit_cross_axis: bool,
    horizontal_wheel: bool,
    environment: &Environment,
) {
    use crate::scene::{Style, Transform};
    let main_size = move |size: (f32, f32)| if horizontal { size.0 } else { size.1 };
    let translation = move |offset: f32| {
        if horizontal {
            Transform { x: -offset, y: 0. }
        } else {
            Transform { x: 0., y: -offset }
        }
    };
    // Compiled morphs mount flattened parents before their children. Preparing
    // layout here would publish a partial candidate as its intrinsic size.
    let size = ui.observe_content_size_deferred(root);
    let extent = ui.observe_content_size_deferred(content);
    let ready = |size: (f32, f32), extent: (f32, f32)| {
        [size.0, size.1, extent.0, extent.1].into_iter().all(f32::is_finite)
    };
    if scrollbar != ScrollbarStyle::Hidden {
        let main_extent = ui.signal(f32::NAN);
        let read_size = size.clone();
        let read_extent = extent.clone();
        let write_extent = main_extent.clone();
        ui.bind(root, move || {
            let size = read_size.get();
            let extent = read_extent.get();
            if ready(size, extent) {
                write_extent.set(main_size(extent));
            }
        });
        mount_scrollbar(
            ui,
            environment,
            root,
            offset.clone(),
            size.clone(),
            main_extent,
            horizontal,
            scrollbar,
        );
    }
    let fades = fade_band.map(|(band, duration)| {
        let mut cx = Context::new(ui.runtime.clone(), environment.clone());
        let start = crate::motion::MotionValue::new(&mut cx, 0.);
        let end = crate::motion::MotionValue::new(&mut cx, 0.);
        let read_start = start.clone();
        let read_end = end.clone();
        let read_offset = offset.clone();
        let read_extent = extent.clone();
        let read_size = size.clone();
        let mut previous = (false, false);
        cx.retain(cx.runtime().effect(move || {
            let size = read_size.get();
            let extent = read_extent.get();
            if !ready(size, extent) {
                return;
            }
            let limit = (main_size(extent) - main_size(size)).max(0.);
            let position = read_offset.get().clamp(0., limit);
            let next = (position > 1., limit - position > 1.);
            let transition = crate::motion::Transition::tween(
                duration(), crate::motion::Easing::cubic_bezier(0., 0., 0.58, 1.),
            );
            if next.0 != previous.0 {
                read_start.animate_to(if next.0 { band } else { 0. }, transition);
            }
            if next.1 != previous.1 {
                read_end.animate_to(if next.1 { band } else { 0. }, transition);
            }
            previous = next;
        }));
        cx.finish(ui, root);
        (start, end)
    });
    let wheel_size = size.clone();
    let wheel_extent = extent.clone();
    let wheel_offset = offset.clone();
    let weak = ui.downgrade();
    ui.on_event(root, false, move |cx| {
        if cx.phase == EventPhase::Capture {
            return;
        }
        if matches!(cx.event, InputEvent::Focus) && cx.target != root {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            // Direct focus calls need the same settled allocation as native input.
            // World bounds include transforms immediately, so an outer scroller
            // observes the translation applied by an inner bubbling handler.
            ui.prepare_frame();
            // A target handler may redirect focus before this event bubbles.
            if ui.input.focused() != Some(cx.target) {
                return;
            }
            let size = wheel_size.get();
            let extent = wheel_extent.get();
            if !ready(size, extent) {
                return;
            }
            let movement = {
                let scene = ui.scene.borrow();
                if !scene.contains(cx.target) || !scene.contains(viewport) {
                    return;
                }
                let Some(inverse) = scene.world_paint_transform(viewport).inverse() else {
                    return;
                };
                let target = scene
                    .world_paint_transform(cx.target)
                    .then(inverse)
                    .bounds(scene.layout_bounds(cx.target));
                let visible = scene.layout_bounds(viewport);
                let (target_start, target_size, visible_start, visible_size) = if horizontal {
                    (target.x, target.width, visible.x, visible.width)
                } else {
                    (target.y, target.height, visible.y, visible.height)
                };
                let start_delta = target_start as f64 - visible_start as f64;
                let end_delta = target_start as f64 + target_size as f64
                    - visible_start as f64
                    - visible_size as f64;
                if start_delta < 0. && end_delta > 0. {
                    0. // Oversized targets already spanning the viewport stay put.
                } else if start_delta < 0. {
                    if target_size > visible_size {
                        end_delta
                    } else {
                        start_delta
                    }
                } else if end_delta > 0. {
                    if target_size > visible_size {
                        start_delta
                    } else {
                        end_delta
                    }
                } else {
                    0.
                }
            };
            let limit = (main_size(extent) as f64 - main_size(size) as f64).max(0.);
            let position = (wheel_offset.get() as f64 + movement).clamp(0., limit) as f32;
            wheel_offset.set(position);
            // A surrounding reactive batch can defer the geometry binding.
            // Publish this translation now for the next ancestor's focus handler.
            let mut scene = ui.scene.borrow_mut();
            if scene.contains(content) && ui.input.focused() == Some(cx.target) {
                scene.set_transform(content, translation(wheel_offset.get()));
            }
        }
        if let InputEvent::Scroll {
            delta_x, delta_y, ..
        } = cx.event
        {
            if cx.default_prevented() {
                return;
            }
            let delta = if horizontal {
                if horizontal_wheel && delta_y.abs() >= delta_x.abs() { delta_y } else { delta_x }
            } else { delta_y };
            if !delta.is_finite() {
                return;
            }
            let size = wheel_size.get();
            let extent = wheel_extent.get();
            if !ready(size, extent) {
                return;
            }
            let before = wheel_offset.get();
            let limit = (main_size(extent) as f64 - main_size(size) as f64).max(0.);
            let after = (before as f64 + delta as f64).clamp(0., limit) as f32;
            if before != after || horizontal && horizontal_wheel && limit > 0. {
                wheel_offset.set(after);
                cx.prevent_default();
                cx.stop_propagation();
            }
        }
    });
    // Padding can move this box without changing the allocated content size.
    let viewport_bounds = ui.observe_layout_bounds(viewport);
    let scene = ui.scene.clone();
    ui.bind(root, move || {
        let _ = viewport_bounds.get();
        let (width, height) = size.get();
        let extent = extent.get();
        if !ready((width, height), extent) {
            return;
        }
        let limit = (main_size(extent) - main_size((width, height))).max(0.);
        let requested = offset.get();
        let position = if requested.is_nan() {
            0.
        } else {
            requested.clamp(0., limit)
        };
        if requested != position {
            offset.set(position);
        }
        if let Some(progress) = &progress {
            progress.set(if limit > 0. { position / limit } else { 0. });
        }
        let mut scene = scene.borrow_mut();
        let padding = scene.padding(root);
        let padded = padding != crate::scene::Insets::default();
        scene.set_style(
            viewport,
            Style {
                width: (!fit_cross_axis || horizontal).then_some(width),
                height: (!fit_cross_axis || !horizontal).then_some(height),
                // A bounded mask includes authored padding without changing layout or ranges.
                clip: !padded,
                clip_insets: padded.then_some(crate::scene::Insets {
                    left: -padding.left,
                    right: -padding.right,
                    top: -padding.top,
                    bottom: -padding.bottom,
                }),
                fade_edges: fades.as_ref().map_or([0.;2], |(a,b)| {let bands=[a.get(),b.get()]; if horizontal {bands.map(|v|-v)} else {bands}}),
                ..Default::default()
            },
        );
        let mut content_style = scene.style(content);
        content_style.width = (!horizontal && !fit_cross_axis).then_some(width);
        content_style.height = (horizontal && !fit_cross_axis).then_some(height);
        if horizontal {
            content_style.min_width = Some(width);
        }
        content_style.flex_shrink = 0.;
        scene.set_style(content, content_style);
        scene.set_transform(content, translation(position));
    });
}
#[cfg(target_os = "macos")]
fn mount_native_surface(
    ui: &mut Ui,
    root: NodeId,
    source: Signal<Option<std::rc::Rc<crate::native_surface::NativeSurface>>>,
    fit: Signal<crate::style::ObjectFit>,
) {
    use crate::scene::{Style, Transform};
    let viewport = ui.container(
        root,
        Layout::Overlay,
        Style {
            absolute: true,
            clip: true,
            ..Default::default()
        },
    );
    let bitmap = ui.container(
        viewport,
        Layout::Overlay,
        Style {
            absolute: true,
            ..Default::default()
        },
    );
    let size = ui.observe_content_size(root);
    let scene = ui.scene.clone();
    ui.bind(root, move || {
        let frame = source.get().expect("mounted surface");
        let (width, height) = size.get();
        let (w, h) = fitted_image_size(
            width,
            height,
            frame.width() as f32,
            frame.height() as f32,
            fit.get(),
        );
        let mut scene = scene.borrow_mut();
        scene.set_kind(bitmap, NodeKind::NativeSurface(frame));
        scene.set_style(
            viewport,
            Style {
                width: Some(width),
                height: Some(height),
                absolute: true,
                clip: true,
                ..Default::default()
            },
        );
        scene.set_style(
            bitmap,
            Style {
                width: Some(w),
                height: Some(h),
                absolute: true,
                ..Default::default()
            },
        );
        scene.set_transform(
            bitmap,
            Transform {
                x: (width - w) * 0.5,
                y: (height - h) * 0.5,
            },
        );
    });
}
fn mount_image(
    ui: &mut Ui,
    root: NodeId,
    source: Signal<Option<std::sync::Arc<crate::image::ImageData>>>,
    fit: Signal<crate::style::ObjectFit>,
) {
    use crate::scene::{Style, Transform};
    let viewport = ui.container(
        root,
        Layout::Overlay,
        Style {
            absolute: true,
            clip: true,
            ..Default::default()
        },
    );
    // The source binding may be deferred by the surrounding mount batch.
    // Reserve a stable node now; that binding supplies its pixels before paint.
    let initial = source
        .get()
        .map(NodeKind::Image)
        .unwrap_or(NodeKind::Container(Layout::Overlay));
    let bitmap = ui.scene.borrow_mut().append(
        viewport,
        initial,
        Style {
            absolute: true,
            ..Default::default()
        },
    );
    let size = ui.observe_content_size(root);
    let scene = ui.scene.clone();
    ui.bind(root, move || {
        let data = source.get().expect("mounted image source");
        let (width, height) = size.get();
        let fitting = fit.get();
        let crop = data.transform() == crate::affine::Affine::IDENTITY
            || matches!(
                fitting,
                crate::style::ObjectFit::Cover | crate::style::ObjectFit::None
            );
        let (image_width, image_height) = fitted_image_size(
            width,
            height,
            data.width() as f32,
            data.height() as f32,
            fitting,
        );
        let mut scene = scene.borrow_mut();
        scene.set_kind(bitmap, NodeKind::Image(data));
        scene.set_style(
            viewport,
            Style {
                width: Some(width),
                height: Some(height),
                absolute: true,
                clip: crop,
                ..Default::default()
            },
        );
        scene.set_style(
            bitmap,
            Style {
                width: Some(image_width),
                height: Some(image_height),
                absolute: true,
                ..Default::default()
            },
        );
        scene.set_transform(
            bitmap,
            Transform {
                x: (width - image_width) * 0.5,
                y: (height - image_height) * 0.5,
            },
        );
    });
}
fn fitted_image_size(
    width: f32,
    height: f32,
    source_width: f32,
    source_height: f32,
    fit: crate::style::ObjectFit,
) -> (f32, f32) {
    use crate::style::ObjectFit;
    if width <= 0. || height <= 0. {
        return (0., 0.);
    }
    if fit == ObjectFit::Fill {
        return (width, height);
    }
    if fit == ObjectFit::None {
        return (source_width, source_height);
    }
    let horizontal = width as f64 / source_width as f64;
    let vertical = height as f64 / source_height as f64;
    let scale = match fit {
        ObjectFit::Cover => horizontal.max(vertical),
        ObjectFit::ScaleDown => horizontal.min(vertical).min(1.),
        _ => horizontal.min(vertical),
    };
    (
        (source_width as f64 * scale).min(f32::MAX as f64) as f32,
        (source_height as f64 * scale).min(f32::MAX as f64) as f32,
    )
}
fn mount_progress(ui: &mut Ui, root: NodeId, value: Signal<f32>, typography: Signal<Typography>) {
    use crate::scene::{Style, Transform};
    let size = ui.observe_content_size(root);
    let viewport = ui.container(
        root,
        Layout::Overlay,
        Style {
            clip: true,
            ..Default::default()
        },
    );
    let fill =
        ui.scene
            .borrow_mut()
            .append(viewport, NodeKind::Rect(ui.theme.text), Style::default());
    let scene = ui.scene.clone();
    let semantics = ui.semantics.clone();
    ui.bind(root, move || {
        let raw = value.get();
        let normalized = if raw.is_nan() { 0. } else { raw.clamp(0., 1.) };
        if raw != normalized {
            value.set(normalized);
        }
        let (width, height) = size.get();
        let mut scene = scene.borrow_mut();
        let geometry = Style {
            width: Some(width),
            height: Some(height),
            ..Default::default()
        };
        scene.set_style(
            viewport,
            Style {
                clip: true,
                ..geometry.clone()
            },
        );
        scene.set_style(fill, geometry);
        scene.set_kind(fill, NodeKind::Rect(typography.get().color));
        // A translated full-size fill stays clipped to the content viewport.
        // Value-only updates therefore avoid changing layout dimensions.
        scene.set_transform(
            fill,
            Transform {
                x: -width * (1. - normalized),
                y: 0.,
            },
        );
        semantics
            .borrow_mut()
            .update(root, |node| node.numeric_value = Some(normalized as f64));
    });
}
fn mount_slider(
    ui: &mut Ui,
    root: NodeId,
    value: Signal<f32>,
    range: std::ops::RangeInclusive<f32>,
    typography: Signal<Typography>,
    chrome: bool,
    step_size: Option<f32>,
    painted_thumb_width: f32,
) {
    use crate::{
        input::{Key, PointerButton},
        scene::{Style, Transform},
    };
    let (min, max) = (*range.start() as f64, *range.end() as f64);
    let span = max - min;
    let size = ui.observe_content_size(root);
    let rail = ui
        .scene
        .borrow_mut()
        .append(root, NodeKind::Rect(if chrome { ui.theme.hover } else { crate::scene::Color(0, 0, 0, 0) }), Style::default());
    let fill_viewport = ui.container(
        root,
        Layout::Overlay,
        Style {
            clip: true,
            ..Default::default()
        },
    );
    let fill = ui.scene.borrow_mut().append(
        fill_viewport,
        NodeKind::Rect(if chrome { ui.theme.accent } else { crate::scene::Color(0, 0, 0, 0) }),
        Style::default(),
    );
    let thumb = ui
        .scene
        .borrow_mut()
        .append(root, NodeKind::Rect(ui.theme.text), Style::default());
    let scene = ui.scene.clone();
    let semantics = ui.semantics.clone();
    let observed_value = value.clone();
    ui.bind(root, move || {
        let raw = observed_value.get();
        let val = if raw.is_nan() {
            min as f32
        } else {
            raw.clamp(min as f32, max as f32)
        };
        if raw != val {
            observed_value.set(val);
        }
        let (width, height) = size.get();
        let thumb_width = width.min(painted_thumb_width);
        let thumb_height = height.min(20.);
        let travel = (width - thumb_width).max(0.);
        let x = ((val as f64 - min) / span * travel as f64) as f32;
        let rail_height = height.min(4.);
        let fixed = |w, h| Style {
            width: Some(w),
            height: Some(h),
            ..Default::default()
        };
        let mut scene = scene.borrow_mut();
        scene.set_style(rail, fixed(travel, rail_height));
        scene.set_transform(
            rail,
            Transform {
                x: thumb_width / 2.,
                y: (height - rail_height) / 2.,
            },
        );
        scene.set_style(
            fill_viewport,
            Style {
                clip: true,
                ..fixed(travel, rail_height)
            },
        );
        scene.set_style(fill, fixed(travel, rail_height));
        scene.set_transform(
            fill,
            Transform {
                x: x - travel,
                y: 0.,
            },
        );
        scene.set_transform(
            fill_viewport,
            Transform {
                x: thumb_width / 2.,
                y: (height - rail_height) / 2.,
            },
        );
        scene.set_style(thumb, fixed(thumb_width, thumb_height));
        scene.set_transform(
            thumb,
            Transform {
                x,
                y: (height - thumb_height) / 2.,
            },
        );
        scene.set_kind(thumb, NodeKind::Rect(if chrome { typography.get().color } else { crate::scene::Color(0, 0, 0, 0) }));
        semantics
            .borrow_mut()
            .update(root, |node| node.numeric_value = Some(val as f64));
    });
    let scene = ui.scene.clone();
    let mut dragging = false;
    ui.on_event(root, true, move |cx| {
        if cx.phase == EventPhase::Capture {
            return;
        }
        if cx.default_prevented()
            && !matches!(
                cx.event,
                InputEvent::PointerUp { .. }
                    | InputEvent::PointerCancel
                    | InputEvent::Blur
                    | InputEvent::Focus
            )
        {
            return;
        }
        let at_position = |x: f32, y: f32| {
            let scene = scene.borrow();
            let bounds = scene.layout_bounds(rail);
            let (x, _) = scene.world_to_local(rail, x, y)?;
            if bounds.width <= 0. {
                return Some(min as f32);
            }
            let fraction = (x as f64 / bounds.width as f64).clamp(0., 1.);
            Some((min + fraction * span).clamp(min, max) as f32)
        };
        let step =
            |direction: f64| (value.get() as f64 + direction * step_size.map_or(span / 100., |step| step as f64)).clamp(min, max) as f32;
        let next = match &cx.event {
            InputEvent::SetNumericValue(v) if v.is_finite() => Some(v.clamp(min, max) as f32),
            InputEvent::SetValue(v) => v
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(min, max) as f32),
            InputEvent::Increment => Some(step(1.)),
            InputEvent::Decrement => Some(step(-1.)),
            InputEvent::KeyDown { key, .. } => match key {
                Key::ArrowLeft | Key::ArrowDown => Some(step(-1.)),
                Key::ArrowRight | Key::ArrowUp => Some(step(1.)),
                Key::Home => Some(min as f32),
                Key::End => Some(max as f32),
                _ => None,
            },
            InputEvent::PointerDown {
                x,
                y,
                button: PointerButton::Primary,
                ..
            } => {
                let next = at_position(*x, *y);
                if next.is_none() {
                    return;
                }
                dragging = true;
                cx.focus();
                cx.capture_pointer();
                next
            }
            InputEvent::PointerMove { x, y } if dragging => at_position(*x, *y),
            InputEvent::PointerUp {
                button: crate::input::PointerButton::Primary,
                ..
            }
            | InputEvent::PointerCancel
            | InputEvent::Blur => {
                dragging = false;
                cx.release_pointer();
                None
            }
            _ => None,
        };
        if let Some(next) = next {
            if next.is_finite() {
                let next = step_size.map_or(next, |step| ((next - min as f32) / step).round() * step + min as f32).clamp(min as f32, max as f32);
                value.set(next);
            }
            cx.prevent_default();
            cx.stop_propagation();
        }
    });
}
fn update_button_labels(ui: &Ui, node: NodeId) {
    let scene = ui.scene.borrow();
    let mut current = Some(node);
    while let Some(id) = current {
        let is_button = ui
            .semantics
            .borrow()
            .get(id)
            .is_some_and(|n| n.role == Role::Button && !n.explicit_label);
        if is_button {
            let mut parts = Vec::new();
            let mut pending = vec![id];
            while let Some(child) = pending.pop() {
                match scene.kind(child) {
                    NodeKind::Text { text, .. } => parts.push(text.as_ref()),
                    NodeKind::RichText { text } => parts.push(text.text()),
                    _ => {}
                }
                pending.extend(scene.children(child).iter().rev().copied());
            }
            let label = parts.join(" ");
            ui.semantics
                .borrow_mut()
                .update(id, |node| node.label = label);
        }
        current = scene.parent(id);
    }
}
/// Reference to a Ui-owned component tree. Drop does not unmount; call `unmount`.
#[derive(Clone)]
pub struct ViewHandle {
    ui: WeakUi,
    root: NodeId,
    registry: Registry,
}
impl ViewHandle {
    pub fn node(&self) -> NodeId {
        self.root
    }
    pub fn is_mounted(&self) -> bool {
        self.ui
            .upgrade()
            .is_some_and(|ui| ui.scene.borrow().contains(self.root))
    }
    pub fn unmount(&self) {
        if let Some(mut ui) = self.ui.upgrade() {
            ui.remove(self.root);
        }
    }
    pub fn find(&self, id: &str) -> Option<NodeId> {
        let ui = self.ui.upgrade()?;
        let registry = self.registry.borrow();
        registry
            .get(id)?
            .iter()
            .copied()
            .find(|node| ui.scene.borrow().contains(*node))
    }
}
impl Ui {
    /// Mount a declarative tree under the document root. The document owns its lifetime.
    pub fn mount(&mut self, view: View) -> ViewHandle {
        let registry = Registry::default();
        let environment = Environment {
            services: ServiceScope::new(),
            typography: self.signal(Typography {
                display: Default::default(),
                color: self.theme.text,
                size: self.theme.font_size,
                wrap: false,
                font: Default::default(),
            }),
            registry: registry.clone(),
            portal: None,
            menu: None,
            measure: self.scene.borrow().text_measure(),
        };
        let runtime = self.runtime.clone();
        let mut ui = self.shared();
        // Initial model normalization can notify observers that remove the new
        // subtree. Attach the whole tree and its ownership before they run.
        let mut transaction = MountTransaction::new(&ui, self.root());
        let initialization = ui.begin_mount_initialization();
        let root = runtime.batch(|| {
            let _scope = initialization.enter();
            let root = runtime.untracked(|| mount(&mut ui, self.root(), view, environment));
            initialization.set_root(root);
            root
        });
        transaction.commit();
        ViewHandle {
            ui: self.downgrade(),
            root,
            registry,
        }
    }
    /// Replace root content explicitly. Ordinary property changes do not call this.
    pub fn render(&mut self, view: View) -> ViewHandle {
        let children = self.scene.borrow().children(self.root()).to_vec();
        let mounted = self.mount(view);
        for child in children {
            self.remove(child);
        }
        mounted
    }
}
/// Common imports for component application code.
pub mod prelude {
    pub use super::{
        CompiledMorphAnimation, CompiledMorphEffect, CompiledMorphTiming, compiled_morph, Context, ScrollbarStyle, Slot, VariableHeights, View, button, checkbox, column, component, div, image,
        image_signal, keyed, measured_rows, measured_virtual_list, menu, menu_item, modal, overlay,
        popover, progress, provide, provide_with, row, scroll, scroll_x, slider, submenu, submenu_with_anchor, switch,
        text, text_area, text_input, text_signal, variable_virtual_list, virtual_list,
    };
    pub use super::{DisplayText, RichTextView, TextSpan, display_text_signal, rich_text, rich_text_signal, text_span};
    pub use super::{animated_image, animated_image_controlled, async_image, canvas};
    #[cfg(target_os = "macos")]
    pub use super::{native_surface, native_surface_signal};
    pub use super::{svg, svg_signal};
    pub use crate::affine::Affine;
    pub use crate::canvas::{
        Brush, Canvas, FillRule, GradientStop, LineCap, LineJoin, Path, PathBuilder, Point, Stroke,
    };
    pub use crate::cursor::Cursor;
    pub use crate::decoration::{Background, BorderStyle, Corners};
    pub use crate::motion::{
        AnimationGroup, Completion, DragMotion, Easing, Interpolate, Keyframe, Keyframes,
        MotionAxis, MotionColor, MotionError, MotionInspector, MotionPoint, MotionPolicy,
        MotionStates, MotionValue, Playback, Presence, Repeat, ScrollProgress, SharedLayoutScope,
        Spring, Timeline, Transition, Vec2,
    };
    pub use crate::rich_text::Decoration;
    pub use crate::style::{ObjectFit, Styled, Styles, rgb, rgba};
    pub use crate::svg::SvgData;
    pub use crate::text_layout::{FontFamily, FontFeatures, TextAlign, TextOptions, TextOverflow};
}

#[cfg(test)]
mod scroll_mount_tests {
    use super::*;

    #[test]
    fn scroll_mount_waits_for_complete_geometry_without_clamping_or_publishing_nan() {
        for horizontal in [false, true] {
            let mut ui = Ui::new(600., 400.);
            let offset = ui.signal(37.);
            let progress = ui.signal(0.25);
            let view = if horizontal {
                scroll_x(offset.clone())
            } else {
                scroll(offset.clone())
            };
            let view = ui.mount(
                view.size(200., 100.)
                    .scroll_progress(progress.clone())
                    .scroll_fades_with_duration(8., || std::time::Duration::ZERO)
                    .child(div().size(500., 500.)),
            );
            let root = view.node();
            let (viewport, content, overlay, track, thumb) = {
                let scene = ui.scene.borrow();
                let viewport = scene.children(root)[0];
                let overlay = scene.children(root)[1];
                let track = scene.children(overlay)[0];
                (
                    viewport,
                    scene.children(viewport)[0],
                    overlay,
                    track,
                    scene.children(track)[0],
                )
            };
            let size = ui.observe_content_size_deferred(root);
            let extent = ui.observe_content_size_deferred(content);
            assert_eq!(
                ui.scene.borrow().layout_revision(),
                0,
                "mount must not prepare an incomplete tree"
            );
            assert_eq!((offset.get(), progress.get()), (37., 0.25));
            for (size_value, extent_value) in [
                ((200., f32::NAN), (500., 500.)),
                ((f32::NAN, 100.), (500., 500.)),
                ((200., 100.), (f32::NAN, 500.)),
                ((200., 100.), (500., f32::NAN)),
            ] {
                ui.runtime.batch(|| {
                    size.set(size_value);
                    extent.set(extent_value);
                });
                // Bypass Ui's usual frame preparation to exercise pre-layout consumers.
                ui.input.dispatch_to(
                    &ui.scene,
                    root,
                    InputEvent::Scroll {
                        x: 10.,
                        y: 10.,
                        delta_x: 40.,
                        delta_y: 40.,
                    },
                );
                ui.input
                    .dispatch_to(&ui.scene, track, InputEvent::Increment);
                assert_eq!((offset.get(), progress.get()), (37., 0.25));
                assert!(ui.semantics.borrow().get(track).is_none());
                let scene = ui.scene.borrow();
                assert_eq!(scene.layout_revision(), 0);
                for node in [viewport, content, overlay, track, thumb] {
                    let style = scene.style(node);
                    assert!(
                        [style.width, style.height, style.min_width, style.min_height]
                            .into_iter()
                            .flatten()
                            .all(f32::is_finite)
                    );
                    assert!(style.fade_edges.into_iter().all(f32::is_finite));
                    let transform = scene.transform(node);
                    assert!(transform.x.is_finite() && transform.y.is_finite());
                }
            }
            ui.prepare_frame();
            let limit = if horizontal { 300. } else { 400. };
            assert_eq!(offset.get(), 37.);
            assert_eq!(progress.get(), 37. / limit);
            assert_eq!(
                ui.semantics.borrow().get(track).unwrap().max,
                Some(limit as f64)
            );
            offset.set(10000.);
            ui.prepare_frame();
            assert_eq!(offset.get(), limit);
            assert_eq!(progress.get(), 1.);
        }
    }
}

#[cfg(test)]
mod vector_mount_tests {
    use super::*;
    use std::{cell::Cell, sync::Arc};

    #[test]
    fn svg_and_canvas_wait_for_complete_mount_geometry_and_retain_reactive_sources() {
        let mut ui = Ui::new(400., 300.);
        let source = ui.signal(Arc::new(crate::svg::SvgData::new(&b"<svg/>"[..]).unwrap()));
        let color = ui.signal(crate::scene::Color(255, 0, 0, 255));
        let width = ui.signal(80.);
        let svg_calls = Rc::new(Cell::new(0));
        let canvas_calls = Rc::new(Cell::new(0));
        let svg_read = source.clone();
        let svg_count = svg_calls.clone();
        let canvas_read = color.clone();
        let canvas_count = canvas_calls.clone();
        let width_read = width.clone();
        let view = ui.mount(
            column()
                .w(140.)
                .id("flow")
                .child(
                    svg_signal("Icon", move || {
                        svg_count.set(svg_count.get() + 1);
                        svg_read.get()
                    })
                    .id("svg")
                    .size(40., 20.),
                )
                .child(
                    canvas(move |(width, height)| {
                        assert!(width.is_finite() && height.is_finite());
                        canvas_count.set(canvas_count.get() + 1);
                        let mut canvas = crate::canvas::Canvas::new();
                        canvas.fill(
                            crate::canvas::Path::rectangle(crate::scene::Rect::new(
                                0., 0., width, height,
                            )),
                            canvas_read.get(),
                        );
                        canvas
                    })
                    .id("canvas")
                    .h(30.)
                    .reactive_style(move || Styles::new().w(width_read.get())),
                )
                .child(div().id("tail").h(30.)),
        );
        assert_eq!(
            ui.scene.borrow().layout_revision(),
            0,
            "vector mounting must not measure partial ancestors"
        );
        assert_eq!((svg_calls.get(), canvas_calls.get()), (0, 0));
        let svg = view.find("svg").unwrap();
        let canvas = view.find("canvas").unwrap();
        ui.observe_content_size_deferred(svg).set((40., f32::NAN));
        ui.observe_content_size_deferred(canvas)
            .set((f32::NAN, 30.));
        assert_eq!((svg_calls.get(), canvas_calls.get()), (0, 0));
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        assert_eq!((svg_calls.get(), canvas_calls.get()), (1, 1));
        let (svg_child, canvas_child) = {
            let scene = ui.scene.borrow();
            assert_eq!(scene.layout_bounds(view.node()).height, 80.);
            assert_eq!(scene.layout_bounds(view.find("tail").unwrap()).y, 50.);
            (scene.children(svg)[0], scene.children(canvas)[0])
        };
        source.set(Arc::new(
            crate::svg::SvgData::new(&b"<svg width=\"2\"/>"[..]).unwrap(),
        ));
        color.set(crate::scene::Color(0, 255, 0, 255));
        ui.prepare_frame();
        assert_eq!(ui.scene.borrow_mut().flush().layout_nodes, 0);
        assert_eq!((svg_calls.get(), canvas_calls.get()), (2, 2));
        width.set(120.);
        ui.prepare_frame();
        assert_eq!(ui.scene.borrow().layout_bounds(canvas_child).width, 120.);
        assert_eq!(ui.scene.borrow().children(svg)[0], svg_child);
        assert_eq!(ui.scene.borrow().children(canvas)[0], canvas_child);
        let calls = (svg_calls.get(), canvas_calls.get());
        view.unmount();
        source.set(Arc::new(crate::svg::SvgData::new(&b"<svg/>"[..]).unwrap()));
        color.set(crate::scene::Color(0, 0, 255, 255));
        width.set(100.);
        ui.prepare_frame();
        assert_eq!((svg_calls.get(), canvas_calls.get()), calls);
    }
}

#[cfg(test)]
mod image_fit_tests {
    use super::fitted_image_size;
    use crate::style::ObjectFit;

    #[test]
    fn extreme_aspect_scaling_remains_finite_and_zero_content_has_no_bitmap() {
        for mode in [
            ObjectFit::Fill,
            ObjectFit::Contain,
            ObjectFit::Cover,
            ObjectFit::None,
            ObjectFit::ScaleDown,
        ] {
            let (width, height) = fitted_image_size(f32::MAX, 1., 1., u32::MAX as f32, mode);
            assert!(width.is_finite() && height.is_finite());
            assert!(width >= 0. && height >= 0.);
            assert_eq!(fitted_image_size(0., 100., 20., 10., mode), (0., 0.));
            assert_eq!(fitted_image_size(100., 0., 20., 10., mode), (0., 0.));
        }
    }
}

#[cfg(test)]
mod editor_mount_tests {
    use super::*;
    use crate::input::Key;
    use crate::scene::Color;

    #[test]
    fn editor_focus_preserves_borders_caret_and_selection() {
        for (multiline, border) in [false, true]
            .into_iter()
            .flat_map(|multiline| [None, Some(0.), Some(2.)].map(move |border| (multiline, border)))
        {
            let mut ui = Ui::new(400., 100.);
            let value = ui.signal("address".into());
            let mut editor = if multiline {
                text_area("Editor", value)
            } else {
                text_input("Editor", value)
            };
            if let Some(border) = border {
                editor = editor.border(border).border_color(Color(70, 80, 90, 255));
            }
            let view = ui.mount(editor);
            ui.prepare_frame();
            let borders = |ui: &Ui| {
                ui.scene
                    .borrow()
                    .paint_items()
                    .filter_map(|item| match item.kind {
                        NodeKind::Quad(quad) | NodeKind::Panel { quad, .. }
                            if quad.border_width > 0. =>
                        {
                            Some((quad.border_width, quad.border_color))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            let before = borders(&ui);
            ui.input.focus(&ui.scene, Some(view.node()));
            ui.prepare_frame();
            assert_eq!(
                borders(&ui),
                before,
                "focus must not invent or replace a field border"
            );
            let editor = ui.focused_editor().unwrap();
            assert_eq!(editor.node, view.node());
            assert!(
                ui.scene
                    .borrow()
                    .paint_items()
                    .any(|item| item.effects.opacity > 0.
                        && matches!(item.kind, NodeKind::Rect(color) if *color == ui.theme.text)),
                "focused editor must retain its caret"
            );
            editor.select_all();
            ui.prepare_frame();
            assert_eq!(editor.copy(), "address");
            assert!(ui.scene.borrow().paint_items().any(|item|
                item.effects.opacity > 0. && matches!(item.kind, NodeKind::Quad(quad) if quad.fill == ui.theme.selection)),
                "focused editor must paint its selection");
            ui.input.focus(&ui.scene, None);
            ui.prepare_frame();
            assert_eq!(
                borders(&ui),
                before,
                "blur must restore the same field border"
            );
        }
    }

    #[test]
    fn explicit_editor_focus_border_and_default_button_focus_remain_available() {
        for editor in [true, false] {
            let mut ui = Ui::new(400., 100.);
            let view = if editor {
                text_input("Editor", ui.signal("address".into()))
                    .focus(|s| s.border(1.).border_color(ui.theme.accent))
            } else {
                button().child(text("Button"))
            };
            let mounted = ui.mount(view);
            ui.prepare_frame();
            ui.input.focus(&ui.scene, Some(mounted.node()));
            ui.prepare_frame();
            assert!(ui.scene.borrow().paint_items().any(
                |item| matches!(item.kind, NodeKind::Quad(quad) | NodeKind::Panel { quad, .. }
                    if quad.border_width == 1. && quad.border_color == ui.theme.accent)
            ));
        }
    }

    #[test]
    fn focus_border_policy_preserves_control_borders_and_keyboard_activation() {
        for kind in 0..5 {
            let mut ui = Ui::new(400., 100.);
            ui.theme.focus_borders = false;
            let activations = std::rc::Rc::new(std::cell::Cell::new(0));
            let clicked = activations.clone();
            let view = match kind {
                0 | 1 => button().child(text("Button")).on_click(move || {
                    clicked.set(clicked.get() + 1);
                }),
                2 => text_input("Editor", ui.signal("address".into())),
                3 => checkbox("Checkbox", ui.signal(false)),
                _ => slider("Slider", ui.signal(0.5), 0.0..=1.0),
            };
            let base_color = Color(30, 40, 50, 255);
            let focus_color = Color(60, 70, 80, 255);
            let view = if kind == 1 || kind == 2 {
                view.border(2.).border_color(base_color)
                    .focus(|s| s.border(4.).border_color(focus_color))
            } else {
                view
            };
            let mounted = ui.mount(view);
            ui.prepare_frame();
            ui.input.focus(&ui.scene, Some(mounted.node()));
            ui.prepare_frame();
            assert_eq!(ui.input.focused(), Some(mounted.node()));
            let expected_border = if kind == 1 || kind == 2 { 2. } else { 0. };
            if let NodeKind::Quad(quad) | NodeKind::Panel { quad, .. } = ui.scene.borrow().kind(mounted.node()) {
                assert_eq!(quad.border_width, expected_border);
                if expected_border > 0. {
                    assert_eq!(quad.border_color, base_color);
                }
            } else {
                assert_eq!(expected_border, 0.);
            }
            if kind == 0 || kind == 1 {
                ui.dispatch(InputEvent::KeyDown {
                    key: Key::Enter,
                    modifiers: Default::default(),
                    repeat: false,
                });
                ui.dispatch(InputEvent::KeyUp {
                    key: Key::Enter,
                    modifiers: Default::default(),
                });
                assert_eq!(activations.get(), 1);
            }
        }
    }

    #[test]
    fn canonical_writeback_can_unmount_a_fully_registered_declarative_tree() {
        let mut ui = Ui::new(400., 300.);
        let model = ui.signal("a\r\nb".to_owned());
        let observed = model.clone();
        let weak = ui.downgrade();
        let watcher = ui.runtime.effect(move || {
            if observed.get() == "ab" {
                let mut ui = weak.upgrade().unwrap();
                let children = ui.scene.borrow().children(ui.root()).to_vec();
                for child in children {
                    ui.remove(child);
                }
            }
        });
        let view = ui.mount(
            column()
                .child(text_input("Editor", model.clone()))
                .child(text("After")),
        );
        assert!(!ui.scene.borrow().contains(view.node()));
        assert_eq!(ui.runtime.effect_count(), 1);
        model.set("next\nvalue".to_owned());
        ui.prepare_frame();
        assert_eq!(model.get(), "next\nvalue");
        assert!(ui.scene.borrow().children(ui.root()).is_empty());
        drop(watcher);
        assert_eq!(ui.runtime.effect_count(), 0);
    }
}

#[cfg(test)]
mod control_alignment_tests {
    use super::*;

    #[test]
    fn fixed_controls_center_content_and_preserve_explicit_padding_and_alignment() {
        let mut ui = Ui::new(300., 200.);
        let root = ui.mount(
            column().children([
                button()
                    .id("square")
                    .size(32., 32.)
                    .child(div().id("icon").size(14., 14.))
                    .child(text("Accessible label").absolute().size(0., 0.).opacity(0.)),
                button()
                    .id("row")
                    .h(28.)
                    .px(6.)
                    .child(text("1").id("number").text_size(10.).line_height(14.))
                    .child(text("Option").text_size(12.).line_height(16.8)),
                button()
                    .id("explicit")
                    .size(32., 32.)
                    .px(4.)
                    .py(3.)
                    .justify_start()
                    .child(div().id("explicit-icon").size(14., 14.)),
                button().id("auto").child(text("Auto")),
            ]),
        );
        ui.prepare_frame();
        let scene = ui.scene.borrow();
        let bounds = |name: &str| scene.bounds(root.find(name).unwrap());
        let square = bounds("square");
        let icon = bounds("icon");
        assert_eq!((icon.x - square.x, icon.y - square.y), (9., 9.));
        let row = bounds("row");
        let number = bounds("number");
        assert_eq!((number.x - row.x, number.y - row.y), (6., 7.));
        let explicit = bounds("explicit");
        let icon = bounds("explicit-icon");
        assert_eq!((icon.x - explicit.x, icon.y - explicit.y), (4., 9.));
        let explicit_style = scene.style(root.find("explicit").unwrap());
        assert_eq!(explicit_style.padding_edges.unwrap().top, 3.);
        assert_eq!(scene.style(root.find("auto").unwrap()).padding_edges.unwrap().top, 8.);
    }
}

/// Application bindings applied to both live and generated native view factories.
/// The factory applies the source styles to the replacement returned by `create`.
pub trait CompiledHost {
    /// Namespace repeated compiled masters by their permanent application role.
    fn identity(&self, id: &str) -> String { id.into() }
    fn create(&self, _id: &str, fallback: impl FnOnce() -> View) -> View { fallback() }
    fn decorate(&self, _id: &str, view: View) -> View { view }
}
impl CompiledHost for () {}
#[cfg(feature = "codegen")]
#[path = "compose_codegen.rs"]
mod codegen;
#[cfg(feature = "codegen")]
pub use codegen::CompiledSource;

impl View {
    /// Replace an imported component's contents without changing its source styling.
    pub fn without_children(mut self) -> Self { self.children.clear(); self }
}

fn mount_scrollbar(
    ui: &mut Ui,
    environment: &Environment,
    owner: NodeId,
    offset: Signal<f32>,
    size: Signal<(f32, f32)>,
    extent: Signal<f32>,
    horizontal: bool,
    style: ScrollbarStyle,
) {
    let mut cx = Context::new(ui.runtime.clone(), environment.clone());
    crate::compose_scrollbar::mount_with_style(
        ui, &mut cx, owner, offset, size, extent, horizontal, style,
    );
    cx.finish(ui, owner);
}

#[cfg(test)]
#[path = "compose_react_host_tests.rs"]
mod react_host_tests;
