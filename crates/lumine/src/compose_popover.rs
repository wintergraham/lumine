//! Placement for a retained popover portal; ownership and focus live in compose_modal.
use crate::{
    scene::{NodeId, Rect, Transform},
    widgets::Ui,
};

/// Position the private placement host, preserving the public panel's styles.
/// Registrations belong to the logical owner so disposal removes subscriptions.
pub(crate) fn mount(
    ui: &mut Ui,
    owner: NodeId,
    anchor: NodeId,
    overlay: NodeId,
    position: NodeId,
    panel: NodeId,
) {
    mount_placement(ui, owner, anchor, overlay, position, panel, false);
}

/// A submenu starts beside its trigger, flipping left when the right side lacks room.
pub(crate) fn mount_submenu(
    ui: &mut Ui,
    owner: NodeId,
    anchor: NodeId,
    overlay: NodeId,
    position: NodeId,
    panel: NodeId,
) {
    mount_placement(ui, owner, anchor, overlay, position, panel, true);
}

fn mount_placement(
    ui: &mut Ui,
    owner: NodeId,
    anchor: NodeId,
    overlay: NodeId,
    position: NodeId,
    panel: NodeId,
    sideways: bool,
) {
    let anchor_bounds = ui.observe_bounds(anchor);
    let viewport = ui.observe_bounds(overlay);
    let panel_bounds = ui.observe_bounds(panel);
    let position_bounds = ui.observe_bounds(position);
    let weak = ui.downgrade();
    ui.bind(owner, move || {
        let anchor = anchor_bounds.get();
        let viewport = viewport.get();
        let panel = panel_bounds.get();
        let current = position_bounds.get();
        let desired = if sideways {
            submenu_placement(anchor, viewport, panel.width, panel.height)
        } else {
            placement(anchor, viewport, panel.width, panel.height)
        };
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let mut scene = ui.scene.borrow_mut();
        if !scene.contains(position) {
            return;
        }
        let previous = scene.transform(position);
        let dx = desired.0 - current.x;
        let dy = desired.1 - current.y;
        // Avoid introducing an allocation feedback cycle through float rounding.
        if dx.abs() > 0.01 || dy.abs() > 0.01 {
            scene.set_transform(
                position,
                Transform {
                    x: previous.x + dx,
                    y: previous.y + dy,
                },
            );
        }
    });
}

fn placement(anchor: Rect, viewport: Rect, width: f32, height: f32) -> (f32, f32) {
    let right = viewport.x + viewport.width;
    let bottom = viewport.y + viewport.height;
    let below = anchor.y + anchor.height + 4.;
    let above = anchor.y - height - 4.;
    let y = if below + height > bottom && above >= viewport.y {
        above
    } else {
        below
    };
    (
        anchor.x.clamp(viewport.x, (right - width).max(viewport.x)),
        y.clamp(viewport.y, (bottom - height).max(viewport.y)),
    )
}

fn submenu_placement(anchor: Rect, viewport: Rect, width: f32, height: f32) -> (f32, f32) {
    let right_edge = viewport.x + viewport.width;
    let right = anchor.x + anchor.width + 4.;
    let left = anchor.x - width - 4.;
    let right_room = right_edge - right;
    let left_room = anchor.x - 4. - viewport.x;
    let x = if right_room >= width || right_room >= left_room {
        right
    } else {
        left
    };
    (
        x.clamp(viewport.x, (right_edge - width).max(viewport.x)),
        anchor.y.clamp(
            viewport.y,
            (viewport.y + viewport.height - height).max(viewport.y),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn submenu_placement_flips_and_clamps_with_nonzero_viewport_origin() {
        let viewport = Rect::new(10., 20., 300., 200.);
        assert_eq!(
            submenu_placement(Rect::new(30., 50., 80., 20.), viewport, 100., 60.),
            (114., 50.)
        );
        assert_eq!(
            submenu_placement(Rect::new(230., 190., 60., 20.), viewport, 100., 60.),
            (126., 160.)
        );
        // Both sides are too small; choose the more spacious side then clamp.
        assert_eq!(
            submenu_placement(Rect::new(140., 10., 40., 20.), viewport, 180., 60.),
            (130., 20.)
        );
        assert_eq!(
            submenu_placement(Rect::new(30., 50., 80., 20.), viewport, 400., 300.),
            (10., 20.)
        );
    }
}
