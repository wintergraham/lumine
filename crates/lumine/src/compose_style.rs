//! Styling lowered onto retained nodes; state changes preserve mounted children.
use crate::{
    input::{EventPhase, InputEvent, Key, PointerButton},
    reactive::Signal,
    scene::{Color, NodeId, NodeKind, QuadStyle},
    style::Styles,
    widgets::Ui,
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Typography {
    pub display: crate::text_layout::TextOptions,
    pub color: Color,
    pub size: f32,
    pub wrap: bool,
    pub font: crate::text_layout::FontStyle,
}
#[derive(Default)]
pub(crate) struct Variants {
    pub hover: Option<Styles>,
    pub active: Option<Styles>,
    pub focus: Option<Styles>,
    pub disabled: Option<Styles>,
}
#[derive(Clone, Copy, PartialEq)]
enum ActivationKey {
    Space,
    Enter,
}
#[derive(Clone, Copy, Default, PartialEq)]
struct Interaction {
    hover: bool,
    pointer_active: bool,
    key_active: Option<ActivationKey>,
    focus: bool,
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn mount_style(
    ui: &mut Ui,
    node: NodeId,
    base: Styles,
    inherited: Signal<Typography>,
    dynamic: Option<Box<dyn FnMut() -> Styles>>,
    variants: Variants,
    disabled: Option<Box<dyn FnMut() -> bool>>,
    interactive: bool,
) -> Signal<Typography> {
    mount_style_with_intrinsic(
        ui,
        node,
        base,
        inherited,
        dynamic,
        variants,
        disabled,
        interactive,
        None,
        None,
        None,
        None,
    )
}

/// Resolved styles, typography, intrinsic size and content layout target.
type AppliedInputs = (
    Styles,
    Typography,
    Option<(f32, f32)>,
    Option<NodeId>,
    crate::affine::Affine,
);
#[allow(clippy::too_many_arguments)]
pub(crate) fn mount_style_with_intrinsic(
    ui: &mut Ui,
    node: NodeId,
    base: Styles,
    inherited: Signal<Typography>,
    mut dynamic: Option<Box<dyn FnMut() -> Styles>>,
    mut variants: Variants,
    mut disabled: Option<Box<dyn FnMut() -> bool>>,
    interactive: bool,
    intrinsic: Option<Signal<(f32, f32)>>,
    layout_target: Option<Signal<Option<NodeId>>>,
    object_fit: Option<Signal<crate::style::ObjectFit>>,
    projection: Option<Signal<crate::affine::Affine>>,
) -> Signal<Typography> {
    // Preserve the base/hover border and all non-border focus feedback.
    // Focus itself still drives keyboard input, semantics and editor carets.
    if !ui.theme.focus_borders && let Some(focus) = &mut variants.focus {
        focus.border_width = None;
        focus.border_color = None;
        focus.border_edges = None;
        focus.border_style = None;
    }
    let typography = ui.signal(inherited.with_untracked(Clone::clone));
    let output = typography.clone();
    let interaction = ui.signal(Interaction::default());
    if interactive
        || variants.hover.is_some()
        || variants.active.is_some()
        || variants.focus.is_some()
    {
        let interaction = interaction.clone();
        ui.on_event(node, interactive, move |event| {
            if event.phase == EventPhase::Capture {
                return;
            }
            if event.default_prevented()
                && matches!(
                    event.event,
                    InputEvent::PointerDown { .. } | InputEvent::KeyDown { .. }
                )
            {
                return;
            }
            interaction.update(|state| match event.event {
                InputEvent::PointerEnter => state.hover = true,
                InputEvent::PointerLeave => {
                    state.hover = false;
                    state.pointer_active = false;
                }
                InputEvent::PointerDown {
                    button: PointerButton::Primary,
                    ..
                } => {
                    state.pointer_active = true;
                    if event.target == node {
                        event.capture_pointer();
                    }
                }
                InputEvent::PointerUp {
                    button: PointerButton::Primary,
                    ..
                }
                | InputEvent::PointerCancel => state.pointer_active = false,
                InputEvent::Focus if event.target == node => state.focus = true,
                InputEvent::Blur if event.target == node => {
                    state.focus = false;
                    state.pointer_active = false;
                    state.key_active = None;
                }
                InputEvent::KeyDown {
                    key: Key::Space,
                    repeat: false,
                    ..
                } if event.target == node => state.key_active = Some(ActivationKey::Space),
                InputEvent::KeyDown {
                    key: Key::Enter,
                    repeat: false,
                    ..
                } if event.target == node => state.key_active = Some(ActivationKey::Enter),
                InputEvent::KeyUp {
                    key: Key::Space, ..
                } if event.target == node && state.key_active == Some(ActivationKey::Space) => {
                    state.key_active = None
                }
                InputEvent::KeyUp {
                    key: Key::Enter, ..
                } if event.target == node && state.key_active == Some(ActivationKey::Enter) => {
                    state.key_active = None
                }
                _ => {}
            });
        });
    }
    let weak = ui.downgrade();
    let original_style = ui.scene.borrow().style(node);
    let original_effects = ui.scene.borrow().effects(node);
    let original_transform = ui.scene.borrow().transform(node);
    let original_paint = ui.scene.borrow().paint_transform(node);
    let original_origin = ui.scene.borrow().paint_transform_origin(node);
    let projection_bounds = projection.as_ref().map(|_| ui.observe_layout_bounds(node));
    let original_isolated = ui.scene.borrow().is_isolated(node);
    let original_kind = ui.scene.borrow().kind(node).clone();
    let mut previous_disabled = None;
    // The inputs last applied to the scene. Animations re-run this effect
    // every frame; unchanged results skip rebuilding and rewriting the node.
    let mut applied: Option<AppliedInputs> = None;
    ui.bind(node, move || {
        let Some(mut ui) = weak.upgrade() else { return };
        if !ui.scene.borrow().contains(node) { return; }
        let mut resolved = base.clone();
        if let Some(dynamic) = &mut dynamic { resolved.merge(&dynamic()); }

        let state = interaction.get();
        let is_disabled = disabled.as_mut().is_some_and(|read| read());
        for (enabled, variant) in [(!is_disabled && state.hover, &variants.hover), (!is_disabled && (state.pointer_active || state.key_active.is_some()), &variants.active), (!is_disabled && state.focus, &variants.focus), (is_disabled, &variants.disabled)] {
            if enabled && let Some(variant) = variant { resolved.merge(variant); }
        }
        if disabled.is_some() && previous_disabled != Some(is_disabled) {
            previous_disabled = Some(is_disabled);
            ui.runtime.clone().untracked(|| ui.set_disabled(node, is_disabled));
        }
        if let Some(output) = &object_fit { output.set(resolved.object_fit.unwrap_or_default()); }
        // Only observe inherited typography when a field is actually inherited.
        let projection = projection.as_ref().map_or(crate::affine::Affine::IDENTITY, |value| value.get());
        let parent = if resolved.text_overflow.is_none() || resolved.line_clamp.is_none() || resolved.text_color.is_none() || resolved.text_size.is_none() || resolved.text_wrap.is_none() || resolved.font_family.is_none() || resolved.font_features.is_none() || resolved.text_align.is_none() || resolved.font_fallbacks.is_none() || resolved.font_weight.is_none() || resolved.italic.is_none() || resolved.line_height.is_none() || resolved.letter_spacing.is_none() {
            inherited.get()
        } else {
            inherited.with_untracked(Clone::clone)
        };
        let current = Typography { display: crate::text_layout::TextOptions {overflow: resolved.text_overflow.unwrap_or(parent.display.overflow),line_clamp:resolved.line_clamp.unwrap_or(parent.display.line_clamp)}, color: resolved.text_color.unwrap_or(parent.color), size: resolved.text_size.unwrap_or(parent.size), wrap: resolved.text_wrap.unwrap_or(parent.wrap), font: crate::text_layout::FontStyle { family: resolved.font_family.clone().unwrap_or(parent.font.family), features: resolved.font_features.clone().unwrap_or(parent.font.features), align: resolved.text_align.unwrap_or(parent.font.align), fallbacks: resolved.font_fallbacks.clone().unwrap_or(parent.font.fallbacks), weight: resolved.font_weight.unwrap_or(parent.font.weight), italic: resolved.italic.unwrap_or(parent.font.italic), line_height: resolved.line_height.unwrap_or(parent.font.line_height), letter_spacing: resolved.letter_spacing.unwrap_or(parent.font.letter_spacing) } };
        output.set(current.clone());
        // Read every remaining dependency before any early return so the
        // effect keeps its subscriptions.
        let intrinsic_size = intrinsic.as_ref().map(|size| size.get());
        let content_layout = layout_target.as_ref().and_then(|target| target.get());
        let projection = if let Some(bounds) = &projection_bounds {
            let observed = bounds.get();
            let inherited_inverse = ui.ancestor_layout_compensation(node);
            let (paint, origin) = paint_transform(&resolved, original_paint, original_origin);
            let legacy = resolved.transform.unwrap_or(original_transform);
            let mut scene = ui.scene.borrow_mut();
            if !scene.contains(node) { return; }
            let bounds = observed.unwrap_or_else(|| scene.layout_bounds(node));
            let pivot = projection.point(bounds.width * origin[0], bounds.height * origin[1]);
            let compensation = crate::affine::Affine::translation(bounds.x, bounds.y).then(inherited_inverse)
                .then(crate::affine::Affine::translation(-bounds.x, -bounds.y));
            let desired = projection.then(paint.around(pivot.0, pivot.1))
                .then(crate::affine::Affine::translation(legacy.x, legacy.y)).then(compensation);
            scene.set_paint_transform_origin(node, paint, origin);
            scene.set_transform(node, legacy);
            if projection == crate::affine::Affine::IDENTITY && inherited_inverse == crate::affine::Affine::IDENTITY {
                crate::affine::Affine::IDENTITY
            } else { scene.projection_for_paint(node, desired) }
        } else { projection };
        let inputs = (resolved, current, intrinsic_size, content_layout, projection);
        if applied.as_ref() == Some(&inputs) {
            return;
        }
        // Animations mostly change paint effects or the transform, which need
        // neither layout nor a new paint kind: update just those.
        if let Some(previous) = &mut applied
            && previous.1 == inputs.1
            && previous.2 == inputs.2
            && previous.3 == inputs.3
        {
            let before = (previous.0.opacity, previous.0.blur, previous.0.edge_fade, previous.0.transform, previous.0.scale, previous.0.rotation, previous.0.transform_origin);
            let next = &inputs.0;
            (previous.0.opacity, previous.0.blur, previous.0.edge_fade, previous.0.transform, previous.0.scale, previous.0.rotation, previous.0.transform_origin) =
                (next.opacity, next.blur, next.edge_fade, next.transform, next.scale, next.rotation, next.transform_origin);
            if previous.0 == *next {
                let mut effects = original_effects;
                if let Some(value) = next.opacity { effects.opacity = value; }
                if let Some(value) = next.blur { effects.blur_radius = value; }
                if let Some(value) = next.edge_fade { effects.edge_fade = value; }
                let mut scene = ui.scene.borrow_mut();
                scene.set_effects(node, effects);
                scene.set_transform(node, next.transform.unwrap_or(original_transform));
                let (paint, origin) = paint_transform(next, original_paint, original_origin);
                scene.set_paint_transform_origin(node, paint, origin);
                scene.set_projection_transform(node, inputs.4);
                previous.4 = inputs.4;
                drop(scene);
                ui.publish_paint_transform(node);
                return;
            }
            (previous.0.opacity, previous.0.blur, previous.0.edge_fade, previous.0.transform, previous.0.scale, previous.0.rotation, previous.0.transform_origin) = before;
        }
        let (resolved, current, _, _, projection) = &*applied.insert(inputs);
        let mut style = original_style.clone();
        if let Some(resolved_options) = &resolved.layout_options {
            if let Some(original) = &style.layout_options {
                let mut options = **original;
                options.merge(**resolved_options);
                if options != **original { style.layout_options = Some(std::sync::Arc::new(options)); }
            } else {
                style.layout_options = Some(resolved_options.clone());
            }
        }
        macro_rules! field { ($($field:ident),* $(,)?) => { $(if let Some(value) = resolved.$field { style.$field = value; })* }; }
        macro_rules! dimension { ($($field:ident),* $(,)?) => { $(if let Some(value) = resolved.$field { style.$field = Some(value); })* }; }
        dimension!(min_width,max_width,min_height,max_height);
        if resolved.width.is_some() || resolved.width_percent.is_some() {
            style.width = resolved.width;
            style.width_percent = resolved.width_percent;
        }
        if resolved.height.is_some() || resolved.height_percent.is_some() {
            style.height = resolved.height;
            style.height_percent = resolved.height_percent;
        }
        field!(absolute,gap,clip,fade_edges,flex_grow,flex_shrink,align,justify,text_wrap,margin);
        if matches!(original_kind, NodeKind::Text { .. } | NodeKind::RichText { .. }) { style.text_wrap = current.wrap; style.text_options = current.display; }
        if let Some(padding) = resolved.padding { style.padding_edges = Some(padding); }
        if let Some((width, height)) = intrinsic_size {
            let padding = style.padding_edges.unwrap_or(crate::scene::Insets::all(style.padding));
            if resolved.width.is_none() && resolved.width_percent.is_none() { style.width = Some(width + padding.left.max(0.) + padding.right.max(0.)); }
            if resolved.height.is_none() && resolved.height_percent.is_none() { style.height = Some(height + padding.top.max(0.) + padding.bottom.max(0.)); }
        }

        let mut content_options = crate::layout::LayoutOptions::default();
        if layout_target.is_some() {
            style.gap = 0.; style.align = crate::scene::Align::Stretch; style.justify = crate::scene::Justify::Start;
            if let Some(options) = &mut style.layout_options {
                let options = std::sync::Arc::make_mut(options);
                if options.display != Some(crate::layout::Display::None) { content_options.display = options.display.take(); }
                content_options.wrap = options.wrap.take();
                content_options.reverse = options.reverse.take();
                content_options.align_content = options.align_content.take();
                content_options.gap_x = options.gap_x.take(); content_options.gap_y = options.gap_y.take();
                content_options.columns = options.columns.take(); content_options.rows = options.rows.take();
            }
        }
        let mut effects = original_effects;
        if let Some(value) = resolved.opacity { effects.opacity = value; }
        if let Some(value) = resolved.blur { effects.blur_radius = value; }
        if let Some(value) = resolved.edge_fade { effects.edge_fade = value; }
        let mut scene = ui.scene.borrow_mut();
        scene.set_style(node, style);
        if let Some(content) = content_layout {
            let mut content_style = scene.style(content);
            content_style.gap = resolved.gap.unwrap_or(0.);
            content_style.align = resolved.align.unwrap_or(crate::scene::Align::Stretch);
            content_style.justify = resolved.justify.unwrap_or(crate::scene::Justify::Start);
            content_style.layout_options = (!content_options.is_empty()).then(|| std::sync::Arc::new(content_options));
            scene.set_style(content, content_style);
            if let Some(layout) = resolved.layout { scene.set_kind(content, NodeKind::Container(layout)); }
        }

        scene.set_cursor(node, resolved.cursor);
        scene.set_effects(node, effects);
        scene.set_transform(node, resolved.transform.unwrap_or(original_transform));
        let (paint, origin) = paint_transform(resolved, original_paint, original_origin);
        scene.set_paint_transform_origin(node, paint, origin);
        scene.set_projection_transform(node, *projection);
        scene.set_isolated(node, resolved.isolated.unwrap_or(original_isolated));
        let kind = match &original_kind {
            NodeKind::Container(layout) | NodeKind::Panel { layout, .. } => {
                let mut quad = match &original_kind { NodeKind::Panel { quad, .. } => quad.clone(), _ => QuadStyle::default() };
                if let Some(value) = resolved.background { quad.fill = value; }
                if let Some(value) = resolved.radius { quad.radius = value; }
                if let Some(value) = resolved.border_color { quad.border_color = value; }
                if let Some(value) = resolved.border_width { quad.border_width = value; }
                if let Some(value) = resolved.shadow { quad.shadow = value; }
                if quad.decoration.is_some() || resolved.paint_background.is_some() || resolved.corners.is_some() || resolved.border_edges.is_some() || resolved.border_style.is_some() || resolved.shadows.is_some() {
                    let mut detail = quad.decoration.as_deref().cloned().unwrap_or_default();
                    if let Some(value) = &resolved.paint_background { detail.background = value.clone(); }
                    if let Some(value) = resolved.corners { detail.corners = value; }
                    if let Some(value) = resolved.border_edges { detail.border_widths = value; }
                    if let Some(value) = resolved.border_style { detail.border_style = value; }
                    if let Some(value) = &resolved.shadows { detail.shadows = value.clone(); }
                    quad.decoration = (detail != crate::decoration::Decoration::default()).then(|| std::sync::Arc::new(detail));
                }
                if quad == QuadStyle::default() { NodeKind::Container(if layout_target.is_some() { *layout } else { resolved.layout.unwrap_or(*layout) }) }
                else { NodeKind::Panel { layout: if layout_target.is_some() { *layout } else { resolved.layout.unwrap_or(*layout) }, quad } }
            }
            NodeKind::Text { .. } => match scene.kind(node) {
                NodeKind::Text { text, .. } => NodeKind::Text { text: text.clone(), color: current.color, font_size: current.size },
                kind => kind.clone(),
            },
            _ => scene.kind(node).clone(),
        };
        if matches!(kind, NodeKind::Text { .. }) { scene.set_font(node, current.font.clone()); }
        scene.set_kind(node, kind);
        drop(scene);
        ui.publish_paint_transform(node);
    });
    typography
}

fn paint_transform(
    styles: &Styles,
    original: crate::affine::Affine,
    original_origin: [f32; 2],
) -> (crate::affine::Affine, [f32; 2]) {
    let changed = styles.scale.is_some() || styles.rotation.is_some();
    let matrix = if changed {
        let [x, y] = styles.scale.unwrap_or([1., 1.]);
        crate::affine::Affine::scale(x, y).then(crate::affine::Affine::rotation(
            styles.rotation.unwrap_or(0.),
        ))
    } else {
        original
    };
    (
        matrix,
        styles
            .transform_origin
            .unwrap_or(if changed { [0.5, 0.5] } else { original_origin }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Layout, Style};
    fn color(value: u8) -> Styles {
        Styles {
            background: Some(Color(value, 0, 0, 255)),
            ..Default::default()
        }
    }
    fn fill(ui: &Ui, node: NodeId) -> Color {
        match ui.scene.borrow().kind(node) {
            NodeKind::Panel { quad, .. } => quad.fill,
            _ => panic!("expected panel"),
        }
    }
    fn inherited(ui: &Ui) -> Signal<Typography> {
        ui.signal(Typography {
            display: Default::default(),
            color: Color(255, 255, 255, 255),
            size: 16.,
            wrap: false,
            font: Default::default(),
        })
    }
    #[test]
    fn reactive_affine_styles_preserve_sparse_precedence_and_skip_layout() {
        use crate::{compose::div, style::Styled};
        let mut ui = Ui::new(400., 300.);
        let factor = ui.signal(1.);
        let angle = ui.signal(0.);
        let (f, a) = (factor.clone(), angle.clone());
        let view = ui.mount(
            div()
                .size(100., 40.)
                .translate(12., 8.)
                .scale(1., 3.)
                .transform_origin(0., 0.5)
                .reactive_style(move || Styles::new().scale(f.get(), 3.).rotate(a.get())),
        );
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        let node = view.node();
        ui.runtime.batch(|| {
            factor.set(2.);
            angle.set(std::f32::consts::FRAC_PI_2);
        });
        ui.prepare_frame();
        assert_eq!(ui.scene.borrow_mut().flush().layout_nodes, 0);
        let scene = ui.scene.borrow();
        assert_eq!(scene.layout_bounds(node).width, 100.);
        assert_eq!(
            scene.transform(node),
            crate::scene::Transform { x: 12., y: 8. }
        );
        assert_eq!(scene.paint_transform_origin(node), [0., 0.5]);
        let (x, y) = scene.local_to_world(node, 0., 20.);
        assert!((x - 12.).abs() < 0.001 && (y - 28.).abs() < 0.001);
        let (x, y) = scene.local_to_world(node, 10., 20.);
        assert!((x - 12.).abs() < 0.001 && (y - 48.).abs() < 0.001);
    }
    #[test]
    fn interaction_variants_restore_and_disabled_wins_without_layout() {
        let mut ui = Ui::new(100., 100.);
        let node = ui.container(
            ui.root(),
            Layout::Column,
            Style {
                width: Some(50.),
                height: Some(50.),
                ..Default::default()
            },
        );
        let disabled = ui.signal(false);
        let read_disabled = disabled.clone();
        let inherited = inherited(&ui);
        mount_style(
            &mut ui,
            node,
            color(1),
            inherited,
            None,
            Variants {
                hover: Some(color(2)),
                active: Some(color(3)),
                focus: Some(Styles {
                    border_width: Some(2.),
                    ..Default::default()
                }),
                disabled: Some(color(4)),
            },
            Some(Box::new(move || read_disabled.get())),
            true,
        );
        ui.scene.borrow_mut().flush();
        ui.dispatch(InputEvent::PointerMove { x: 10., y: 10. });
        assert_eq!(fill(&ui, node), Color(2, 0, 0, 255));
        ui.dispatch(InputEvent::PointerDown {
            x: 10.,
            y: 10.,
            button: PointerButton::Primary,
        });
        assert_eq!(fill(&ui, node), Color(3, 0, 0, 255));
        assert_eq!(ui.input.focused(), Some(node));
        assert!(
            matches!(ui.scene.borrow().kind(node), NodeKind::Panel { quad, .. } if quad.border_width == 2.)
        );
        assert_eq!(ui.scene.borrow_mut().flush().layout_nodes, 0);
        disabled.set(true);
        assert_eq!(fill(&ui, node), Color(4, 0, 0, 255));
        assert_eq!(ui.input.focused(), None);
        assert!(
            matches!(ui.scene.borrow().kind(node), NodeKind::Panel { quad, .. } if quad.border_width == 0.)
        );
        assert!(ui.semantics.borrow().get(node).unwrap().disabled);
        disabled.set(false);
        assert_eq!(fill(&ui, node), Color(2, 0, 0, 255));
        ui.dispatch(InputEvent::PointerLeave);
        assert_eq!(fill(&ui, node), Color(1, 0, 0, 255));
    }
    #[test]
    fn typography_inherits_and_explicit_overrides_are_stable() {
        let mut ui = Ui::new(100., 100.);
        let parent = inherited(&ui);
        let text = ui.label(ui.root(), "retained", Style::default());
        let output = mount_style(
            &mut ui,
            text,
            Styles::default(),
            parent.clone(),
            None,
            Variants::default(),
            None,
            false,
        );
        let explicit = ui.label(ui.root(), "fixed", Style::default());
        let fixed = Typography {
            display: Default::default(),
            color: Color(12, 34, 56, 255),
            size: 20.,
            wrap: false,
            font: Default::default(),
        };
        let stable = mount_style(
            &mut ui,
            explicit,
            Styles {
                text_color: Some(fixed.color),
                text_size: Some(fixed.size),
                ..Default::default()
            },
            parent.clone(),
            None,
            Variants::default(),
            None,
            false,
        );
        ui.scene.borrow_mut().flush();
        parent.set(Typography {
            display: Default::default(),
            color: Color(1, 2, 3, 255),
            size: 16.,
            wrap: false,
            font: Default::default(),
        });
        assert_eq!(output.get().color, Color(1, 2, 3, 255));
        assert_eq!(stable.get(), fixed);
        assert_eq!(ui.scene.borrow_mut().flush().layout_nodes, 0);
        parent.set(Typography {
            display: Default::default(),
            color: Color(1, 2, 3, 255),
            size: 24.,
            wrap: false,
            font: Default::default(),
        });
        assert_eq!(output.get().size, 24.);
        assert_eq!(stable.get(), fixed);
        assert!(ui.scene.borrow_mut().flush().layout_nodes > 0);
    }
    #[test]
    fn removing_dynamic_properties_restores_base_and_inheritance() {
        let mut ui = Ui::new(100., 100.);
        let node = ui.container(ui.root(), Layout::Row, Style::default());
        let toggled = ui.signal(true);
        let read = toggled.clone();
        let parent = inherited(&ui);
        let output = mount_style(
            &mut ui,
            node,
            color(1),
            parent.clone(),
            Some(Box::new(move || {
                if read.get() {
                    Styles {
                        background: Some(Color(9, 0, 0, 255)),
                        text_size: Some(40.),
                        opacity: Some(0.5),
                        ..Default::default()
                    }
                } else {
                    Styles::default()
                }
            })),
            Variants::default(),
            None,
            false,
        );
        assert_eq!(output.get().size, 40.);
        assert_eq!(fill(&ui, node), Color(9, 0, 0, 255));
        toggled.set(false);
        assert_eq!(output.get().size, 16.);
        assert_eq!(fill(&ui, node), Color(1, 0, 0, 255));
        assert_eq!(ui.scene.borrow().effects(node).opacity, 1.);
        parent.set(Typography {
            display: Default::default(),
            size: 20.,
            ..parent.get()
        });
        assert_eq!(output.get().size, 20.);
    }
    #[test]
    fn styled_noninteractive_child_preserves_button_activation() {
        use crate::{
            compose::{button, text},
            style::Styled,
        };
        let mut ui = Ui::new(120., 80.);
        let clicks = ui.signal(0);
        let write = clicks.clone();
        ui.mount(
            button()
                .size(100., 60.)
                .child(text("Child").hover(|s| s.text_color(Color(255, 0, 0, 255))))
                .on_click(move || {
                    write.update(|count| *count += 1);
                }),
        );
        ui.dispatch(InputEvent::PointerMove { x: 15., y: 15. });
        ui.dispatch(InputEvent::PointerDown {
            x: 15.,
            y: 15.,
            button: PointerButton::Primary,
        });
        ui.dispatch(InputEvent::PointerUp {
            x: 15.,
            y: 15.,
            button: PointerButton::Primary,
        });
        assert_eq!(clicks.get(), 1);
    }
    #[test]
    fn nested_button_activation_stops_at_nearest_clickable_owner() {
        use crate::{
            compose::{button, text},
            style::Styled,
        };
        let mut ui = Ui::new(140., 100.);
        let outer = ui.signal(0);
        let inner = ui.signal(0);
        let outer_write = outer.clone();
        let inner_write = inner.clone();
        let mounted = ui.mount(
            button()
                .size(120., 80.)
                .p(0.)
                .on_click(move || {
                    outer_write.update(|n| *n += 1);
                })
                .child(
                    button()
                        .id("inner")
                        .size(80., 40.)
                        .on_click(move || {
                            inner_write.update(|n| *n += 1);
                        })
                        .child(text("Inner").hover(|s| s.opacity(0.8))),
                ),
        );
        ui.scene.borrow_mut().prepare_layout();
        let bounds = ui.scene.borrow().bounds(mounted.find("inner").unwrap());
        let x = bounds.x + 15.;
        let y = bounds.y + 15.;
        ui.dispatch(InputEvent::PointerMove { x, y });
        ui.dispatch(InputEvent::PointerDown {
            x,
            y,
            button: PointerButton::Primary,
        });
        ui.dispatch(InputEvent::PointerUp {
            x,
            y,
            button: PointerButton::Primary,
        });
        assert_eq!(inner.get(), 1);
        assert_eq!(outer.get(), 0);
    }
    #[test]
    fn unchanged_label_is_not_reshaped_beside_an_animating_sibling() {
        use crate::{
            compose::{column, div, row, text},
            style::Styled,
        };
        use std::{cell::Cell, rc::Rc};
        let mut ui = Ui::new(600., 400.);
        let calls = Rc::new(Cell::new(0_u32));
        let counter = calls.clone();
        ui.scene
            .borrow_mut()
            .set_text_measurer(move |text: &str, _: f32, _: Option<f32>| {
                if text == "MATRIX" {
                    counter.set(counter.get() + 1);
                }
                (text.len() as f32 * 10., 20.)
            });
        let side = ui.signal(10_f32);
        let read = side.clone();
        // A caption and a pulsing cell in a growing card, as in a loader tile.
        let stage = |label: &str, cell: crate::compose::View| {
            column()
                .grow()
                .min_w(0.)
                .h(150.)
                .p(16.)
                .gap(10.)
                .child(text(label))
                .child(
                    column()
                        .w_full()
                        .grow()
                        .items_center()
                        .justify_center()
                        .child(cell),
                )
        };
        ui.mount(
            column().w_full().gap(20.).child(text("Motion")).child(
                row()
                    .w_full()
                    .gap(12.)
                    .child(stage("MATRIX", div().size(30., 30.)))
                    .child(stage(
                        "WAVE",
                        div().reactive_style(move || Styles::new().size(read.get(), read.get())),
                    )),
            ),
        );
        ui.scene.borrow_mut().flush();
        let settled = calls.get();
        for step in 0..8 {
            side.set(10. + step as f32);
            ui.scene.borrow_mut().flush();
        }
        assert_eq!(
            calls.get(),
            settled,
            "MATRIX was reshaped {} times",
            calls.get() - settled
        );
    }
    #[test]
    fn boxed_text_reactive_wrapping_reflows_without_remounting() {
        use crate::{compose::text, style::Styled};
        let mut ui = Ui::new(300., 200.);
        ui.scene
            .borrow_mut()
            .set_text_measurer(|text: &str, _: f32, limit: Option<f32>| {
                let natural = text.len() as f32 * 10.;
                let width = limit.unwrap_or(natural).min(natural).max(1.);
                (width, (natural / width).ceil() * 20.)
            });
        let wrap = ui.signal(false);
        let read = wrap.clone();
        let view = ui.mount(
            text("abcdefghijabcdefghij")
                .id("box")
                .w(70.)
                .p(10.)
                .bg(Color(1, 2, 3, 255))
                .text_wrap(false)
                .reactive_style(move || Styles::new().text_wrap(read.get())),
        );
        ui.scene.borrow_mut().flush();
        let outer = view.node();
        let inner = ui.scene.borrow().children(outer)[0];
        assert_eq!(ui.scene.borrow().bounds(outer).height, 40.);
        wrap.set(true);
        let report = ui.scene.borrow_mut().flush();
        assert!(report.layout_nodes > 0);
        assert_eq!(ui.scene.borrow().bounds(outer).height, 100.);
        assert_eq!(ui.scene.borrow().bounds(inner).width, 50.);
        assert_eq!(ui.scene.borrow().children(outer), &[inner]);
        assert_eq!(view.find("box"), Some(outer));
        wrap.set(false);
        ui.scene.borrow_mut().flush();
        assert_eq!(ui.scene.borrow().bounds(outer).height, 40.);
        assert_eq!(ui.scene.borrow().children(outer), &[inner]);
    }
    #[test]
    fn font_properties_inherit_override_and_update_without_remount() {
        use crate::{
            compose::{column, text},
            style::Styled,
            text_layout::FontFamily,
        };
        let mut ui = Ui::new(300., 200.);
        let bold = ui.signal(false);
        let read = bold.clone();
        let view = ui.mount(
            column()
                .font_family(FontFamily::Serif)
                .italic(true)
                .reactive_style(move || {
                    Styles::new().font_weight(if read.get() { 700 } else { 400 })
                })
                .child(text("inherited").id("a"))
                .child(
                    text("override")
                        .id("b")
                        .font_family(FontFamily::Monospace)
                        .font_weight(500)
                        .italic(false),
                ),
        );
        let a = view.find("a").unwrap();
        let b = view.find("b").unwrap();
        ui.scene.borrow_mut().flush();
        assert_eq!(ui.scene.borrow().font(a).family, FontFamily::Serif);
        assert!(ui.scene.borrow().font(a).italic);
        assert_eq!(ui.scene.borrow().font(b).weight, 500);
        assert!(!ui.scene.borrow().font(b).italic);
        bold.set(true);
        assert_eq!(view.find("a"), Some(a));
        assert_eq!(ui.scene.borrow().font(a).weight, 700);
        assert_eq!(ui.scene.borrow().font(b).weight, 500);
        assert!(ui.scene.borrow_mut().flush().layout_nodes > 0);
        bold.set(true);
        assert!(ui.scene.borrow_mut().flush().is_idle());
    }
}
