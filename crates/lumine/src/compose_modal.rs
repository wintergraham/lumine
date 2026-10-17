//! Owned overlay portals shared by modal and anchored component views.
use crate::{
    input::{EventPhase, InputEvent, Key, NodeInput, PointerButton},
    reactive::Signal,
    scene::{Effects, Layout, NodeId, Style, Transform},
    semantics::{Role, SemanticNode},
    widgets::{Ui, WeakUi},
};

pub(crate) struct Presentation {
    alpha: crate::motion::MotionValue,
    scale: crate::motion::MotionValue,
    policy: Box<dyn FnMut() -> crate::compose::PopupMotion>,
    open: bool,
}
impl Presentation {
    pub(crate) fn new(
        cx: &mut crate::compose::Context,
        policy: Box<dyn FnMut() -> crate::compose::PopupMotion>,
    ) -> Self {
        Self {
            alpha: crate::motion::MotionValue::new(cx, 0.),
            scale: crate::motion::MotionValue::new(cx, 1.),
            policy,
            open: false,
        }
    }
}

pub(crate) struct Portal {
    pub overlay: NodeId,
    pub position: NodeId,
    pub visible: Signal<bool>,
}
struct Owner {
    ui: WeakUi,
    overlay: NodeId,
}
fn unwind(ui: &Ui, overlay: NodeId) {
    ui.input.remove_focus_scope(&ui.scene, overlay);
}
impl Drop for Owner {
    fn drop(&mut self) {
        let Some(mut ui) = self.ui.upgrade() else {
            return;
        };
        unwind(&ui, self.overlay);
        ui.remove(self.overlay);
    }
}
pub(crate) fn create(ui: &mut Ui, owner: NodeId, parent: NodeId) -> Portal {
    let overlay = ui.container(
        parent,
        Layout::Overlay,
        Style {
            absolute: true,
            ..Default::default()
        },
    );
    ui.retain(
        owner,
        Owner {
            ui: ui.downgrade(),
            overlay,
        },
    );
    let position = ui.container(overlay, Layout::Overlay, Default::default());
    Portal {
        overlay,
        position,
        visible: ui.signal(false),
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn activate(
    ui: &mut Ui,
    owner: NodeId,
    portal: &Portal,
    panel: NodeId,
    label: String,
    open: Signal<bool>,
    parent_visible: Option<Signal<bool>>,
    dismiss_backdrop: bool,
    role: Role,
    logical_parent: Option<NodeId>,
    return_focus: Option<NodeId>,
    focusable: bool,
    mut presentation: Option<Presentation>,
) {
    let overlay = portal.overlay;
    let mut semantics = SemanticNode::new(role, label);
    semantics.modal = role == Role::Dialog;
    semantics.logical_parent = logical_parent;
    semantics.disabled = ui
        .semantics
        .borrow()
        .get(panel)
        .is_some_and(|node| node.disabled);
    ui.semantics.borrow_mut().set(panel, semantics);
    let mut options = ui.input.options(panel).unwrap_or_default();
    options.focusable = focusable;
    ui.input.set_options(panel, options);
    // Blank panel space is a hit boundary, separate from the backdrop.
    ui.on_event(panel, true, |_| {});
    let close = open.clone();
    let input = ui.input.clone();
    ui.on_event(overlay, false, move |cx| match cx.event {
        InputEvent::FocusScopeClosed if cx.phase == EventPhase::Target => {
            close.set(false);
        }
        InputEvent::KeyDown {
            key: Key::Escape, ..
        } if cx.phase != EventPhase::Capture
            && !cx.default_prevented()
            && input.focus_scope() == Some(overlay) =>
        {
            close.set(false);
            cx.prevent_default();
            cx.stop_propagation();
        }
        InputEvent::PointerDown {
            button: PointerButton::Primary,
            ..
        } if cx.phase == EventPhase::Target
            && !cx.default_prevented()
            && input.focus_scope() == Some(overlay) =>
        {
            if dismiss_backdrop {
                close.set(false);
            }
            cx.prevent_default();
            cx.stop_propagation();
        }
        _ => {}
    });
    let root_bounds = ui.observe_bounds(ui.root());
    let scene = ui.scene.clone();
    ui.bind(owner, move || {
        let bounds = root_bounds.get();
        let mut scene = scene.borrow_mut();
        scene.set_style(
            overlay,
            Style {
                absolute: true,
                width: Some(bounds.width),
                height: Some(bounds.height),
                ..Default::default()
            },
        );
    });
    // Portals escape visual ancestry, but retain the logical owner's disability.
    let ancestors = {
        let scene = ui.scene.borrow();
        std::iter::once(owner)
            .chain(scene.ancestors(owner))
            .collect::<Vec<_>>()
    };
    let disabled = ancestors
        .into_iter()
        .map(|node| ui.input.observe_disabled(node, &ui.runtime))
        .collect::<Vec<_>>();
    let weak = ui.downgrade();
    let visible = portal.visible.clone();
    // Portals escape visual ancestry, including retained invisible pages.
    let owner_bounds = ui.observe_layout_bounds(owner);
    ui.bind(owner, move || {
        let blocked = disabled.iter().any(|disabled| disabled.get());
        let requested = open.get();
        let owner_visible = owner_bounds.get().is_some();
        if blocked && requested {
            open.set(false);
        }
        let desired = requested
            && !blocked
            && owner_visible
            && parent_visible.as_ref().is_none_or(|parent| parent.get());
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if !ui.scene.borrow().contains(overlay) {
            return;
        }
        let (alpha, scale, origin) = if let Some(presentation) = &mut presentation {
            let policy = (presentation.policy)();
            if !owner_visible {
                // A hidden owner's motion clock may be paused indefinitely.
                presentation.alpha.set(0.);
                presentation.open = false;
            }
            let reactivated = desired
                && ui
                    .input
                    .options(overlay)
                    .is_some_and(|options| options.disabled);
            if presentation.open != desired || reactivated {
                presentation.open = desired;
                if desired {
                    presentation.alpha.set(0.);
                    presentation.scale.set(policy.enter_scale);
                    presentation.alpha.animate_to(1., policy.enter);
                    presentation.scale.animate_to(1., policy.enter);
                } else {
                    presentation.alpha.animate_to(0., policy.exit);
                }
            }
            (
                presentation.alpha.signal().get(),
                presentation.scale.signal().get(),
                policy.origin,
            )
        } else {
            (if desired { 1. } else { 0. }, 1., [0.5, 0.5])
        };
        ui.scene.borrow_mut().set_effects(
            overlay,
            Effects {
                // Opacity-zero portals cannot acquire a focus scope. Preserve
                // logical entry immediately; this rounds to transparent paint.
                opacity: if desired && presentation.is_some() {
                    alpha.max(f32::EPSILON)
                } else {
                    alpha
                },
                ..Default::default()
            },
        );
        if presentation.is_some() {
            ui.scene.borrow_mut().set_paint_transform_origin(
                panel,
                crate::affine::Affine::scale(scale, scale),
                origin,
            );
        }
        ui.input.set_options(
            overlay,
            NodeInput {
                disabled: !desired,
                ..Default::default()
            },
        );
        ui.input.set_pointer_transparent(overlay, !desired);
        if desired {
            let already_active = {
                let scene = ui.scene.borrow();
                ui.input.focus_scope().is_some_and(|active| {
                    active == overlay
                        || (scene.contains(active)
                            && scene.ancestors(active).any(|node| node == overlay))
                })
            };
            if !already_active {
                // Raise only when opening, preserving paint order within the scope.
                let parent = ui.scene.borrow().parent(overlay).unwrap();
                let mut children = ui.scene.borrow().children(parent).to_vec();
                children.retain(|node| *node != overlay);
                children.push(overlay);
                ui.scene.borrow_mut().reorder_children(parent, &children);
                if let Some(trigger) = return_focus {
                    ui.input.focus(&ui.scene, Some(trigger));
                }
                if !ui.input.push_focus_scope(&ui.scene, overlay) {
                    open.set(false);
                    visible.set(false);
                    return;
                }
                if !ui.scene.borrow().contains(panel) || !ui.scene.borrow().contains(overlay) {
                    open.set(false);
                    visible.set(false);
                    return;
                }
                // Prefer a usable child control; the panel remains the empty-dialog fallback.
                let candidates = {
                    let scene = ui.scene.borrow();
                    let mut result = Vec::new();
                    let mut pending = scene
                        .children(panel)
                        .iter()
                        .rev()
                        .copied()
                        .collect::<Vec<_>>();
                    while let Some(node) = pending.pop() {
                        result.push(node);
                        pending.extend(scene.children(node).iter().rev().copied());
                    }
                    result
                };
                for candidate in candidates {
                    if (role != Role::Menu
                        || ui
                            .semantics
                            .borrow()
                            .get(candidate)
                            .is_some_and(|node| node.role == Role::MenuItem))
                        && ui
                            .input
                            .options(candidate)
                            .is_some_and(|options| options.focusable)
                        && ui.input.focus(&ui.scene, Some(candidate))
                    {
                        break;
                    }
                }
            }
            visible.set(ui.scene.borrow().contains(overlay));
        } else {
            visible.set(false);
            unwind(&ui, overlay);
        }
    });
}
/// Synchronously close presentation during pointer handoff, even inside a batch.
pub(crate) fn deactivate(ui: &Ui, overlay: NodeId) {
    if ui.scene.borrow().contains(overlay) {
        let mut scene = ui.scene.borrow_mut();
        let mut effects = scene.effects(overlay);
        effects.opacity = 0.;
        scene.set_effects(overlay, effects);
        drop(scene);
        let mut options = ui.input.options(overlay).unwrap_or_default();
        options.disabled = true;
        ui.input.set_options(overlay, options);
    }
    ui.input.remove_focus_scope(&ui.scene, overlay);
}
pub(crate) fn center(ui: &mut Ui, owner: NodeId, position: NodeId) {
    let viewport = ui.observe_bounds(ui.root());
    let panel = ui.observe_bounds(position);
    let scene = ui.scene.clone();
    ui.bind(owner, move || {
        let viewport = viewport.get();
        let panel = panel.get();
        let mut scene = scene.borrow_mut();
        let prior = scene.transform(position);
        let x = viewport.x + ((viewport.width - panel.width) / 2.).max(0.);
        let y = viewport.y + ((viewport.height - panel.height) / 2.).max(0.);
        if (x - panel.x).abs() > 0.01 || (y - panel.y).abs() > 0.01 {
            scene.set_transform(
                position,
                Transform {
                    x: prior.x + x - panel.x,
                    y: prior.y + y - panel.y,
                },
            );
        }
    });
}
