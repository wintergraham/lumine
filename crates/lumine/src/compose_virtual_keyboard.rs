//! Bounded keyboard navigation over the currently mounted virtual-list rows.
use crate::{
    input::{EventPhase, InputEvent, Key},
    reactive::Signal,
    scene::{Layout, NodeId, NodeKind, QuadStyle},
    semantics::{Role, SemanticNode},
    widgets::Ui,
};
use std::{cell::RefCell, rc::Rc};
#[derive(Default)]
struct Rows {
    length: usize,
    visible: Vec<(usize, NodeId)>,
}
pub(crate) struct Navigation {
    pub request: Signal<(u64, usize)>,
    rows: Rc<RefCell<Rows>>,
}
enum Geometry {
    Fixed(f32),
    Variable(crate::compose::VariableHeights),
}
impl Navigation {
    pub fn new(
        ui: &mut Ui,
        root: NodeId,
        offset: Signal<f32>,
        size: Signal<(f32, f32)>,
        row_height: f32,
    ) -> Self {
        Self::with_geometry(ui, root, offset, size, Geometry::Fixed(row_height))
    }
    pub fn new_variable(
        ui: &mut Ui,
        root: NodeId,
        offset: Signal<f32>,
        size: Signal<(f32, f32)>,
        heights: crate::compose::VariableHeights,
    ) -> Self {
        Self::with_geometry(ui, root, offset, size, Geometry::Variable(heights))
    }
    fn with_geometry(
        ui: &mut Ui,
        root: NodeId,
        offset: Signal<f32>,
        size: Signal<(f32, f32)>,
        geometry: Geometry,
    ) -> Self {
        let request = ui.signal((0_u64, 0_usize));
        let write = request.clone();
        let rows = Rc::new(RefCell::new(Rows::default()));
        let state = rows.clone();
        ui.on_event(root, true, move |cx| {
            if cx.phase == EventPhase::Capture || cx.default_prevented() {
                return;
            }
            let InputEvent::KeyDown { key, .. } = &cx.event else {
                return;
            };
            let rows = state.borrow();
            let current = rows
                .visible
                .iter()
                .find(|(_, node)| *node == cx.target)
                .map(|(index, _)| *index);
            // Interactive descendants keep their own editing/control key behavior.
            if cx.target != root && current.is_none() {
                return;
            }
            if rows.length == 0 {
                return;
            }
            let visible_start = match &geometry {
                Geometry::Fixed(row_height) => ((offset.get() as f64 / *row_height as f64).floor()
                    as usize)
                    .min(rows.length - 1),
                Geometry::Variable(heights) => heights
                    .row_at(offset.get())
                    .unwrap_or(0)
                    .min(rows.length - 1),
            };
            let base = current.unwrap_or(visible_start);
            let (previous_page, next_page) = match &geometry {
                Geometry::Fixed(row_height) => {
                    let page = ((size.get().1 as f64 / *row_height as f64).floor() as usize).max(1);
                    (base.saturating_sub(page), base.saturating_add(page))
                }
                Geometry::Variable(heights) => {
                    let top = heights.row_offset(base);
                    let viewport = size.get().1.max(0.);
                    let previous = heights.row_at((top - viewport).max(0.)).unwrap_or(0);
                    let next = heights
                        .row_at((top as f64 + viewport as f64).min(f32::MAX as f64) as f32)
                        .unwrap_or(0);
                    (
                        previous.min(base.saturating_sub(1)),
                        next.max(base.saturating_add(1)),
                    )
                }
            };
            let target = match key {
                Key::Home => 0,
                Key::End => rows.length - 1,
                Key::ArrowDown => current.map_or(visible_start, |index| index.saturating_add(1)),
                Key::ArrowUp => current.map_or(visible_start, |index| index.saturating_sub(1)),
                Key::PageDown => next_page,
                Key::PageUp => previous_page,
                _ => return,
            }
            .min(rows.length - 1);
            drop(rows);
            let serial = write.get().0.wrapping_add(1);
            write.set((serial, target));
            cx.prevent_default();
            cx.stop_propagation();
        });
        Self { request, rows }
    }
    pub fn mount_row(&self, ui: &mut Ui, row: NodeId, _row_height: f32) {
        let marker = ui.scene.borrow_mut().append(
            row,
            NodeKind::Rect(ui.theme.accent),
            crate::scene::Style {
                width: Some(2.),
                height_percent: Some(1.),
                absolute: true,
                ..Default::default()
            },
        );
        ui.scene.borrow_mut().set_effects(
            marker,
            crate::scene::Effects {
                opacity: 0.,
                ..Default::default()
            },
        );
        let scene = ui.scene.clone();
        let color = ui.theme.hover;
        ui.on_event(row, true, move |cx| {
            if cx.phase != EventPhase::Target {
                return;
            }
            match cx.event {
                InputEvent::Focus => {
                    scene
                        .borrow_mut()
                        .set_effects(marker, crate::scene::Effects::default());
                    scene.borrow_mut().set_kind(
                        row,
                        NodeKind::Panel {
                            layout: Layout::Overlay,
                            quad: QuadStyle {
                                fill: color,
                                ..Default::default()
                            },
                        },
                    );
                }
                InputEvent::Blur => {
                    scene.borrow_mut().set_effects(
                        marker,
                        crate::scene::Effects {
                            opacity: 0.,
                            ..Default::default()
                        },
                    );
                    scene
                        .borrow_mut()
                        .set_kind(row, NodeKind::Container(Layout::Overlay));
                }
                _ => {}
            }
        });
    }
    pub fn update(&self, ui: &Ui, root: NodeId, length: usize, visible: Vec<(usize, NodeId)>) {
        let mut semantics = ui.semantics.borrow_mut();
        semantics.update(root, |node| node.size_of_set = Some(length));
        for (index, node) in &visible {
            let mut semantic = SemanticNode::new(Role::ListItem, "");
            semantic.position_in_set = Some(index.saturating_add(1));
            semantic.size_of_set = Some(length);
            semantics.set(*node, semantic);
        }
        *self.rows.borrow_mut() = Rows { length, visible };
    }
}
