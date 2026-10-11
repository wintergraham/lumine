//! Shared retained overlay controls for ordinary and virtual viewports.
use crate::{
    compose::Context,
    input::{EventPhase, InputEvent, Key, PointerButton},
    motion::{Easing, Keyframe, Keyframes, MotionValue, Playback, Transition},
    reactive::Signal,
    scene::{Color, Layout, NodeId, NodeKind, QuadStyle, Style, Transform},
    semantics::{Role, ScrollAxis, SemanticNode},
    widgets::Ui,
};

pub(crate) fn mount(
    ui: &mut Ui,
    context: &mut Context,
    owner: NodeId,
    offset: Signal<f32>,
    size: Signal<(f32, f32)>,
    extent: Signal<f32>,
    horizontal: bool,
) {
    let overlay = ui.container(owner, Layout::Overlay, Style::default());
    let track = ui.container(overlay, Layout::Overlay, Style::default());
    // Components · Scrollbar: a trackless pill inside a 12 px overlay hit area.
    let thumb = ui.scene.borrow_mut().append(
        track,
        NodeKind::Quad(QuadStyle::default()),
        Style::default(),
    );
    let alpha = MotionValue::new(context, 0.);
    let thickness = MotionValue::new(context, 4.);
    let hovered = ui.signal(false);
    let dragging = ui.signal(false);
    let main = move |size: (f32, f32)| if horizontal { size.0 } else { size.1 };
    let event_size = size.clone();
    let event_extent = extent.clone();
    let event_offset = offset.clone();
    let scene = ui.scene.clone();
    let focused = ui.signal(false);
    let write_focus = focused.clone();
    let write_hover = hovered.clone();
    let write_drag = dragging.clone();
    let mut grab = None;
    let mut pointer_focus = false;
    ui.on_event(track, true, move |cx| {
        if cx.phase == EventPhase::Capture {
            return;
        }
        if matches!(cx.event, InputEvent::Focus) {
            write_focus.set(!pointer_focus);
        }
        if matches!(cx.event, InputEvent::Blur) {
            pointer_focus = false;
            write_focus.set(false);
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
        match cx.event {
            InputEvent::PointerEnter => {
                write_hover.set(true);
            }
            InputEvent::PointerLeave => {
                write_hover.set(false);
            }
            _ => {}
        }
        if matches!(cx.event, InputEvent::KeyDown { .. }) {
            pointer_focus = false;
            write_focus.set(true);
        }
        let length = main(event_size.get()) as f64;
        let total = event_extent.get() as f64;
        let limit = (total - length).max(0.);
        let available = (length - 4.).max(0.);
        let thumb_length = (available * length / total.max(1.)).max(32.).min(available);
        let travel = (available - thumb_length).max(0.);
        let position = match cx.event {
            InputEvent::PointerDown { x, y, .. } | InputEvent::PointerMove { x, y } => scene
                .borrow()
                .world_to_local(track, x, y)
                .map(|(x, y)| if horizontal { x as f64 } else { y as f64 }),
            _ => None,
        };
        if position.is_some_and(|position| !position.is_finite())
            || (position.is_none()
                && matches!(
                    cx.event,
                    InputEvent::PointerDown { .. } | InputEvent::PointerMove { .. }
                ))
        {
            return;
        }
        let before = event_offset.get() as f64;
        let next = match &cx.event {
            InputEvent::PointerDown {
                button: PointerButton::Primary,
                ..
            } if limit > 0. => {
                pointer_focus = true;
                write_focus.set(false);
                cx.focus();
                let pointer = position.unwrap();
                let thumb_start = 2. + before / limit * travel;
                if pointer >= thumb_start && pointer <= thumb_start + thumb_length {
                    grab = Some(pointer - thumb_start);
                    write_drag.set(true);
                    cx.capture_pointer();
                    None
                } else {
                    Some(
                        before
                            + if pointer < thumb_start {
                                -length
                            } else {
                                length
                            },
                    )
                }
            }
            InputEvent::PointerMove { .. } if grab.is_some() => Some(if travel > 0. {
                (position.unwrap() - 2. - grab.unwrap()) / travel * limit
            } else {
                0.
            }),
            InputEvent::PointerUp {
                button: PointerButton::Primary,
                ..
            }
            | InputEvent::PointerCancel
            | InputEvent::Blur => {
                grab = None;
                write_drag.set(false);
                cx.release_pointer();
                None
            }
            InputEvent::KeyDown { key, .. } => match key {
                Key::Home => Some(0.),
                Key::End => Some(limit),
                Key::PageUp => Some(before - length),
                Key::PageDown => Some(before + length),
                Key::ArrowLeft | Key::ArrowUp => Some(before - 40.),
                Key::ArrowRight | Key::ArrowDown => Some(before + 40.),
                _ => None,
            },
            InputEvent::Increment => Some(before + 40.),
            InputEvent::Decrement => Some(before - 40.),
            InputEvent::SetNumericValue(value) if value.is_finite() => Some(*value),
            InputEvent::SetValue(value) => value.parse::<f64>().ok().filter(|v| v.is_finite()),
            _ => None,
        };
        if let Some(next) = next {
            event_offset.set(next.clamp(0., limit) as f32);
            cx.prevent_default();
            cx.stop_propagation();
        } else if matches!(
            cx.event,
            InputEvent::PointerDown {
                button: PointerButton::Primary,
                ..
            }
        ) {
            cx.prevent_default();
            cx.stop_propagation();
        }
    });
    let read_offset = offset.clone();
    let read_hover = hovered.clone();
    let read_drag = dragging.clone();
    let read_focus = focused.clone();
    let paint_alpha = alpha.clone();
    let paint_thickness = thickness.clone();
    let mut previous = (offset.with_untracked(|v| *v), false, false, false);
    ui.bind(overlay, move || {
        let current = (
            read_offset.get(),
            read_hover.get(),
            read_drag.get(),
            read_focus.get(),
        );
        if current == previous {
            return;
        }
        let engaged = current.1 || current.2 || current.3;
        previous = current;
        paint_thickness.animate_to(
            if current.1 || current.2 { 6. } else { 4. },
            Transition::tween(std::time::Duration::from_millis(140), Easing::EaseOut),
        );
        if engaged {
            paint_alpha.animate_to(
                if current.2 {
                    128.
                } else if current.1 {
                    89.
                } else {
                    51.
                },
                Transition::tween(std::time::Duration::from_millis(90), Easing::Linear),
            );
        } else {
            // A finite timeline holds until 800 ms idle, then fades for 200 ms.
            // Retargeting cancels the old timeline; resting scrollbars request no frames.
            paint_alpha.animate_keyframes(
                Keyframes::new([
                    Keyframe::new(0., paint_alpha.signal().with_untracked(|v| *v)),
                    Keyframe::new(0.09, 51.),
                    Keyframe::new(0.8, 51.),
                    Keyframe::new(1., 0.),
                ])
                .unwrap(),
                std::time::Duration::from_millis(1000),
                Playback::default(),
            );
        }
    });
    let weak = ui.downgrade();
    ui.bind(overlay, move || {
        let Some(mut ui) = weak.upgrade() else {
            return;
        };
        let (width, height) = size.get();
        let length = main((width, height));
        let total = extent.get();
        let limit = (total - length).max(0.);
        let visible = limit > 0. && length > 0. && width > 0. && height > 0.;
        if ui
            .input
            .options(track)
            .is_none_or(|options| options.disabled == visible)
        {
            ui.set_disabled(track, !visible);
        }
        let fixed = |width, height| Style {
            width: Some(width),
            height: Some(height),
            ..Default::default()
        };
        let mut scene = ui.scene.borrow_mut();
        // Zero area lets underlying controls receive hits outside the track.
        scene.set_style(
            overlay,
            Style {
                margin: crate::scene::Insets {
                    right: width,
                    ..Default::default()
                },
                ..fixed(0., 0.)
            },
        );
        let hit_width = if horizontal { height } else { width }.min(12.);
        let track_size = if !visible {
            (0., 0.)
        } else if horizontal {
            (length, hit_width)
        } else {
            (hit_width, length)
        };
        scene.set_style(track, fixed(track_size.0, track_size.1));
        scene.set_transform(
            track,
            if horizontal {
                Transform {
                    x: 0.,
                    y: height - hit_width,
                }
            } else {
                Transform {
                    x: width - hit_width,
                    y: 0.,
                }
            },
        );
        let thumb_length = if visible {
            (((length - 4.).max(0.) as f64 * length as f64 / total as f64)
                .max(32.)
                .min((length - 4.).max(0.) as f64)) as f32
        } else {
            0.
        };
        let pill_width = thickness.get().min((hit_width - 3.).max(0.));
        let thumb_size = if horizontal {
            (thumb_length, pill_width)
        } else {
            (pill_width, thumb_length)
        };
        scene.set_style(thumb, fixed(thumb_size.0, thumb_size.1));
        scene.set_kind(
            thumb,
            NodeKind::Quad(QuadStyle {
                fill: Color(255, 255, 255, alpha.get().round() as u8),
                radius: pill_width / 2.,
                ..Default::default()
            }),
        );
        let value = offset.get().clamp(0., limit);
        let position = if limit > 0. {
            (value as f64 / limit as f64 * (length - 4. - thumb_length).max(0.) as f64) as f32
        } else {
            0.
        };
        scene.set_transform(
            thumb,
            if horizontal {
                Transform {
                    x: 2. + position,
                    y: hit_width - 3. - pill_width,
                }
            } else {
                Transform {
                    x: hit_width - 3. - pill_width,
                    y: 2. + position,
                }
            },
        );
        drop(scene);
        if visible {
            let mut semantic = SemanticNode::new(
                Role::ScrollBar,
                if horizontal {
                    "Horizontal scrollbar"
                } else {
                    "Vertical scrollbar"
                },
            );
            semantic.scroll_axis = Some(if horizontal {
                ScrollAxis::Horizontal
            } else {
                ScrollAxis::Vertical
            });
            semantic.min = Some(0.);
            semantic.max = Some(limit as f64);
            semantic.numeric_value = Some(value as f64);
            ui.semantics.borrow_mut().set(track, semantic);
        } else {
            ui.semantics.borrow_mut().remove(track);
        }
    });
}
