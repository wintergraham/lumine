use super::*;
use crate::{
    input::{InputEvent, PointerButton},
    scene::{Color, Insets, Rect},
    widgets::Ui,
};
use std::{cell::Cell, rc::Rc};

#[test]
fn layout_observers_exclude_paint_transforms_and_semantics_stay_reactive() {
    let mut ui = Ui::new(120., 80.);
    let layout = ui.signal(None::<Rect>);
    let world = ui.signal(Rect::default());
    let label = ui.signal("Before".to_owned());
    let read = label.clone();
    let root = ui.mount(
        button()
            .size(40., 24.)
            .translate(12., 8.)
            .observe_layout_bounds(layout.clone())
            .observe_bounds(world.clone())
            .accessibility(move |node| node.label = read.get()),
    );
    ui.prepare_frame();
    assert_eq!(layout.get().unwrap().x, 0.);
    assert_eq!(world.get().x, 12.);
    label.set("After".into());
    ui.prepare_frame();
    assert_eq!(
        ui.semantics.borrow().get(root.node()).unwrap().label,
        "After"
    );
}

#[test]
fn pointer_transparency_updates_without_remounting() {
    let mut ui = Ui::new(120., 80.);
    let enabled = ui.signal(true);
    let read = enabled.clone();
    let clicks = Rc::new(Cell::new(0));
    let hit = clicks.clone();
    let root = ui.mount(
        overlay()
            .size(120., 80.)
            .child(
                button()
                    .id("under")
                    .size(120., 80.)
                    .on_click(move || hit.set(hit.get() + 1)),
            )
            .child(
                button()
                    .id("over")
                    .absolute()
                    .size(120., 80.)
                    .reactive_style(move || Styles::new().pointer_events(read.get())),
            ),
    );
    ui.prepare_frame();
    let over = root.find("over").unwrap();
    let click = |ui: &mut Ui| {
        ui.dispatch(InputEvent::PointerDown {
            x: 20.,
            y: 20.,
            button: PointerButton::Primary,
        });
        ui.dispatch(InputEvent::PointerUp {
            x: 20.,
            y: 20.,
            button: PointerButton::Primary,
        });
    };
    click(&mut ui);
    assert_eq!(clicks.get(), 0);
    enabled.set(false);
    ui.prepare_frame();
    click(&mut ui);
    assert_eq!(root.find("over"), Some(over));
    assert_eq!(clicks.get(), 1);
}

#[test]
fn reveal_clip_changes_paint_and_hits_without_changing_layout() {
    let mut ui = Ui::new(120., 80.);
    let root = ui.mount(
        div()
            .id("reveal")
            .size(80., 60.)
            .bg(Color(255, 0, 0, 255))
            .clip_insets(Some(Insets {
                bottom: 30.,
                ..Default::default()
            })),
    );
    ui.prepare_frame();
    let node = root.find("reveal").unwrap();
    let scene = ui.scene.borrow();
    assert_eq!(scene.layout_bounds(node).height, 60.);
    assert!(scene.hit_test_all(20., 20.).contains(&node));
    assert!(!scene.hit_test_all(20., 40.).contains(&node));
}
