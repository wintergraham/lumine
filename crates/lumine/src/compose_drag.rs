//! Typed component drag sources, accepted destinations and owned previews.
use crate::{
    actions::Action,
    compose::{View, ViewHandle},
    input::{DragPhase, EventPhase, InputEvent, PointerButton},
    scene::{NodeId, Transform},
    widgets::Ui,
};
impl View {
    /// Native paths grouped at the event-loop dispatch boundary. Legacy per-path
    /// FileDrop events remain available separately through on_event.
    pub fn on_files_drop(
        self,
        mut callback: impl FnMut(&[std::path::PathBuf], &mut crate::input::EventContext) + 'static,
    ) -> Self {
        self.on_event(move |cx| {
            if cx.phase == EventPhase::Capture || cx.default_prevented() {
                return;
            }
            if let InputEvent::FilesDrop { paths, .. } = cx.event.clone() {
                callback(&paths, cx);
                cx.prevent_default();
                cx.stop_propagation();
            }
        })
    }

    /// Begin a primary-button drag after moving four logical pixels. A simple
    /// press/release retains ordinary click behavior. Payloads stay window-local.
    pub fn on_drag<T: 'static>(self, mut payload: impl FnMut() -> T + 'static) -> Self {
        let mut origin = None;
        self.on_event(move |cx| {
            if cx.phase == EventPhase::Capture {
                return;
            }
            match cx.event {
                InputEvent::PointerDown {
                    x,
                    y,
                    button: PointerButton::Primary,
                } if !cx.capture_requested() => {
                    origin = Some((x, y));
                    cx.capture_pointer();
                }
                InputEvent::PointerMove { x, y } => {
                    if let Some((px, py)) = origin.filter(|(px, py)| (x - px).hypot(y - py) > 4.) {
                        origin = None;
                        cx.start_drag_from(payload(), px, py);
                    }
                }
                InputEvent::PointerUp { .. } | InputEvent::PointerCancel | InputEvent::Blur => {
                    origin = None
                }
                _ => {}
            }
        })
    }
    pub fn on_drop<T: 'static>(
        self,
        callback: impl FnMut(&T, &mut crate::input::EventContext) + 'static,
    ) -> Self {
        self.on_drop_when(|_: &T| true, callback)
    }
    /// The predicate is rechecked both during hover and immediately at drop.
    pub fn on_drop_when<T: 'static>(
        self,
        mut accept: impl FnMut(&T) -> bool + 'static,
        mut callback: impl FnMut(&T, &mut crate::input::EventContext) + 'static,
    ) -> Self {
        self.on_event(move |cx| {
            if cx.phase == EventPhase::Capture {
                return;
            }
            let InputEvent::Drag(event) = cx.event.clone() else {
                return;
            };
            let Some(value) = event.payload.downcast_ref::<T>() else {
                return;
            };
            match event.phase {
                DragPhase::Over if accept(value) => cx.accept_drag(),
                DragPhase::Drop if !cx.default_prevented() && accept(value) => {
                    callback(value, cx);
                    cx.prevent_default();
                    cx.stop_propagation();
                }
                _ => {}
            }
        })
    }
    /// Render an ordinary component as the retained pointer-following preview.
    /// Its top-left follows the pointer minus the grab point, so a same-sized
    /// preview stays under the spot where the source was pressed.
    pub fn drag_preview<T: 'static>(self, build: impl FnMut(&T) -> View + 'static) -> Self {
        self.preview(build, None)
    }
    /// A preview placed at a fixed offset from the pointer, independent of the
    /// grab point; suited to badges that do not mirror the source.
    pub fn drag_preview_at_cursor<T: 'static>(
        self,
        offset: (f32, f32),
        build: impl FnMut(&T) -> View + 'static,
    ) -> Self {
        self.preview(build, Some(offset))
    }
    fn preview<T: 'static>(
        mut self,
        mut build: impl FnMut(&T) -> View + 'static,
        cursor_offset: Option<(f32, f32)>,
    ) -> Self {
        self.drag_preview = Some(PreviewBuilder {
            build: Box::new(move |payload| payload.downcast_ref::<T>().map(&mut build)),
            cursor_offset,
        });
        self
    }
    pub fn on_drag_end<T: 'static>(self, mut callback: impl FnMut(&T, bool) + 'static) -> Self {
        self.on_event(move |cx| {
            if cx.phase != EventPhase::Target {
                return;
            }
            if let InputEvent::Drag(event) = &cx.event
                && event.phase == DragPhase::End
                && let Some(value) = event.payload.downcast_ref::<T>()
            {
                callback(value, event.accepted);
            }
        })
    }
}
type BuildPreview = Box<dyn FnMut(&Action) -> Option<View>>;
pub(crate) struct PreviewBuilder {
    build: BuildPreview,
    cursor_offset: Option<(f32, f32)>,
}
struct Preview(Option<ViewHandle>);
impl Drop for Preview {
    fn drop(&mut self) {
        if let Some(view) = self.0.take() {
            view.unmount();
        }
    }
}
pub(crate) fn mount(ui: &mut Ui, source: NodeId, mut builder: PreviewBuilder) {
    let weak = ui.downgrade();
    let mut preview = Preview(None);
    let binding = ui.input.listen(source, move |cx| {
        if cx.phase != EventPhase::Target {
            return;
        }
        let InputEvent::Drag(event) = &cx.event else {
            return;
        };
        let Some(mut ui) = weak.upgrade() else {
            return;
        };
        match event.phase {
            DragPhase::Start => {
                if let Some(old) = preview.0.take() {
                    old.unmount();
                }
                if let Some(view) = (builder.build)(&event.payload) {
                    let view = ui.mount(view);
                    let node = view.node();
                    let mut style = ui.scene.borrow().style(node);
                    style.absolute = true;
                    ui.scene.borrow_mut().set_style(node, style);
                    ui.input.set_drag_preview(Some(node));
                    preview.0 = Some(view);
                }
            }
            DragPhase::End => {
                if let Some(view) = preview.0.take() {
                    view.unmount();
                }
                ui.input.set_drag_preview(None);
            }
            _ => {}
        }
        if let Some(view) = &preview.0 {
            ui.scene.borrow_mut().set_transform(
                view.node(),
                match builder.cursor_offset {
                    Some((x, y)) => Transform {
                        x: event.x + x,
                        y: event.y + y,
                    },
                    None => Transform {
                        x: event.x - event.grab_x,
                        y: event.y - event.grab_y,
                    },
                },
            );
        }
    });
    ui.retain(source, binding);
}
