//! One window-local typed drag session. Native OS drag initiation is separate.
use super::*;
use crate::actions::Action;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragPhase {
    Start,
    Move,
    Over,
    Leave,
    Drop,
    End,
}
#[derive(Clone, Debug, PartialEq)]
pub struct DragEvent {
    pub phase: DragPhase,
    pub payload: Action,
    pub source: NodeId,
    pub x: f32,
    pub y: f32,
    /// Pointer position within the source's bounds when the gesture was pressed.
    /// Subtracting it from `x`/`y` keeps a preview under the original grab point.
    pub grab_x: f32,
    pub grab_y: f32,
    /// True on End only when the accepted destination consumed Drop.
    pub accepted: bool,
}
#[derive(Clone)]
pub(super) struct DragSession {
    source: NodeId,
    payload: Action,
    x: f32,
    y: f32,
    grab: (f32, f32),
    destination: Option<NodeId>,
    preview: Option<NodeId>,
}
impl DragSession {
    fn event(&self, phase: DragPhase, accepted: bool) -> InputEvent {
        InputEvent::Drag(DragEvent {
            phase,
            payload: self.payload.clone(),
            source: self.source,
            x: self.x,
            y: self.y,
            grab_x: self.grab.0,
            grab_y: self.grab.1,
            accepted,
        })
    }
}
impl InputDispatcher {
    pub fn is_dragging(&self) -> bool {
        self.state.borrow().drag.is_some()
    }
    /// Exclude a retained preview subtree from drag hit testing.
    pub fn set_drag_preview(&self, node: Option<NodeId>) {
        if let Some(drag) = &mut self.state.borrow_mut().drag {
            drag.preview = node;
        }
    }
    pub fn cancel_drag(&self, scene: &Rc<RefCell<Scene>>) {
        self.finish_drag(scene, false);
    }
    pub fn validate_drag(&self, scene: &Rc<RefCell<Scene>>) {
        let source = self.state.borrow().drag.as_ref().map(|drag| drag.source);
        if source.is_some_and(|source| !self.enabled(&scene.borrow(), source)) {
            self.cancel_drag(scene);
            return;
        }
        let drag = self.state.borrow().drag.clone();
        if let Some(drag) = drag
            && let Some(destination) = drag.destination
            && !self.enabled(&scene.borrow(), destination)
        {
            if let Some(active) = &mut self.state.borrow_mut().drag {
                active.destination = None;
            }
            if scene.borrow().contains(destination) {
                self.route(scene, destination, drag.event(DragPhase::Leave, false));
            }
        }
    }
    pub(super) fn begin_drag(
        &self,
        scene: &Rc<RefCell<Scene>>,
        source: NodeId,
        payload: Action,
        x: f32,
        y: f32,
        pressed_at: (f32, f32),
    ) {
        if self.is_dragging() || !self.enabled(&scene.borrow(), source) {
            return;
        }
        self.cancel_pending_keys();
        let hovered = self.state.borrow_mut().hovered.take();
        if let Some(hovered) = hovered.filter(|id| scene.borrow().contains(*id)) {
            self.route(scene, hovered, InputEvent::PointerLeave);
        }
        if !self.enabled(&scene.borrow(), source) {
            return;
        }
        let Some(grab) = scene
            .borrow()
            .world_to_local(source, pressed_at.0, pressed_at.1)
        else {
            return;
        };
        let drag = DragSession {
            source,
            payload,
            x,
            y,
            grab,
            destination: None,
            preview: None,
        };
        let pressed = {
            let mut state = self.state.borrow_mut();
            state.drag = Some(drag.clone());
            let pressed = state.pressed.take();
            state.key_pressed = None;
            state.clicks.0 = None;
            pressed
        };
        if let Some(pressed) = pressed.filter(|id| *id != source && scene.borrow().contains(*id)) {
            self.route(scene, pressed, InputEvent::PointerCancel);
        }
        self.route(scene, source, InputEvent::PointerCancel);
        self.route(scene, source, drag.event(DragPhase::Start, false));
        self.update_drag(scene, x, y);
    }
    fn drag_hit(&self, scene: &Scene, x: f32, y: f32, preview: Option<NodeId>) -> Option<NodeId> {
        let mut fallback = None;
        for hit in scene.hit_test_all(x, y) {
            if preview.is_some_and(|preview| {
                hit == preview || scene.ancestors(hit).any(|id| id == preview)
            }) {
                continue;
            }
            let mut node = Some(hit);
            while let Some(id) = node {
                if !self.enabled(scene, id) {
                    break;
                }
                if self.has_listeners(id) {
                    // Match ordinary pointer hit testing: document capture
                    // listeners must not make empty overlays obscure siblings.
                    if id == scene.root() {
                        fallback = Some(id);
                        break;
                    }
                    return Some(id);
                }
                node = scene.parent(id);
            }
        }
        fallback
    }
    fn update_drag(&self, scene: &Rc<RefCell<Scene>>, x: f32, y: f32) {
        self.validate_drag(scene);
        let Some(mut drag) = self.state.borrow().drag.clone() else {
            return;
        };
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        drag.x = x;
        drag.y = y;
        if let Some(active) = &mut self.state.borrow_mut().drag {
            active.x = x;
            active.y = y;
        }
        self.route(scene, drag.source, drag.event(DragPhase::Move, false));
        if !self
            .state
            .borrow()
            .drag
            .as_ref()
            .is_some_and(|active| active.source == drag.source && active.payload == drag.payload)
        {
            return;
        }
        let hit = self.drag_hit(&scene.borrow(), x, y, drag.preview);
        let destination = hit
            .and_then(|hit| {
                self.route(scene, hit, drag.event(DragPhase::Over, false))
                    .drag_accepted
            })
            .filter(|id| self.enabled(&scene.borrow(), *id));
        if drag.destination != destination
            && let Some(old) = drag.destination.filter(|id| scene.borrow().contains(*id))
        {
            self.route(scene, old, drag.event(DragPhase::Leave, false));
        }
        if let Some(active) = &mut self.state.borrow_mut().drag {
            active.destination = destination;
        }
    }
    fn finish_drag(&self, scene: &Rc<RefCell<Scene>>, drop: bool) {
        let Some(drag) = self.state.borrow_mut().drag.take() else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            state.pressed = None;
            state.captured = None;
            state.capture_button = None;
        }
        let accepted = drop
            && drag
                .destination
                .filter(|id| self.enabled(&scene.borrow(), *id))
                .is_some_and(|id| {
                    self.route(scene, id, drag.event(DragPhase::Drop, false))
                        .prevented
                });
        if let Some(id) = drag.destination.filter(|id| scene.borrow().contains(*id)) {
            self.route(scene, id, drag.event(DragPhase::Leave, accepted));
        }
        if scene.borrow().contains(drag.source) {
            self.route(scene, drag.source, drag.event(DragPhase::End, accepted));
        }
        if drop {
            let hovered = self.hit_target(&scene.borrow(), drag.x, drag.y, false);
            self.state.borrow_mut().hovered = hovered;
            if let Some(hovered) = hovered {
                self.route(scene, hovered, InputEvent::PointerEnter);
            }
        }
    }
    pub(super) fn dispatch_drag(&self, scene: &Rc<RefCell<Scene>>, event: &InputEvent) -> bool {
        self.validate_drag(scene);
        if !self.is_dragging() {
            return false;
        }
        match event {
            InputEvent::PointerMove { x, y } => {
                self.update_drag(scene, *x, *y);
                true
            }
            InputEvent::PointerUp {
                x,
                y,
                button: PointerButton::Primary,
            } => {
                self.update_drag(scene, *x, *y);
                self.finish_drag(scene, true);
                true
            }
            InputEvent::PointerCancel
            | InputEvent::Blur
            | InputEvent::KeyDown {
                key: Key::Escape, ..
            } => {
                self.cancel_drag(scene);
                true
            }
            InputEvent::PointerLeave => {
                let drag = self.state.borrow().drag.clone().unwrap();
                if let Some(id) = drag.destination.filter(|id| scene.borrow().contains(*id)) {
                    self.route(scene, id, drag.event(DragPhase::Leave, false));
                }
                if let Some(drag) = &mut self.state.borrow_mut().drag {
                    drag.destination = None;
                }
                true
            }
            _ => false,
        }
    }
}
