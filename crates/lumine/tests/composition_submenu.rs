use std::{cell::Cell, rc::Rc};
use lumine::{
    compose::prelude::*,
    input::{InputEvent, PointerButton},
    widgets::Ui,
};

fn gesture(ui: &Ui, x: f32, y: f32) {
    ui.input.dispatch(
        &ui.scene,
        InputEvent::PointerDown {
            x,
            y,
            button: PointerButton::Primary,
        },
    );
    ui.input.dispatch(
        &ui.scene,
        InputEvent::PointerUp {
            x,
            y,
            button: PointerButton::Primary,
        },
    );
}

#[test]
fn outside_cascade_click_closes_every_level_without_background_activation() {
    let mut ui = Ui::new(900., 600.);
    let root = ui.signal(true);
    let child = ui.signal(false);
    let calls = Rc::new(Cell::new(0));
    let write = calls.clone();
    let mounted = ui.mount(
        column().children([
            menu("Actions", root.clone(), button().child(text("Actions")))
                .child(submenu("More", child.clone()).child(menu_item("Child"))),
            button()
                .id("background")
                .w(800.)
                .h(400.)
                .on_click(move || write.set(write.get() + 1)),
        ]),
    );
    ui.prepare_frame();
    child.set(true);
    ui.prepare_frame();
    let background = ui
        .scene
        .borrow()
        .bounds(mounted.find("background").unwrap());
    ui.runtime.batch(|| {
        gesture(
            &ui,
            background.x + background.width - 10.,
            background.y + background.height - 10.,
        )
    });
    assert!(!root.get());
    assert!(!child.get());
    assert_eq!(calls.get(), 0);
    assert!(ui.input.focus_scope().is_none());
}

#[test]
fn batched_parent_pointer_handoff_preserves_one_activation_and_full_width_trigger() {
    let mut ui = Ui::new(900., 600.);
    let root = ui.signal(true);
    let child = ui.signal(false);
    let calls = Rc::new(Cell::new(0));
    let write = calls.clone();
    let mounted = ui.mount(
        menu("Actions", root.clone(), button().child(text("Actions")))
            .w(240.)
            .p(8.)
            .child(
                submenu("More", child.clone())
                    .trigger_id("more")
                    .child(menu_item("Child")),
            )
            .child(
                menu_item("Parent")
                    .id("parent")
                    .on_click(move || write.set(write.get() + 1)),
            ),
    );
    ui.prepare_frame();
    let more = ui.scene.borrow().bounds(mounted.find("more").unwrap());
    let parent = ui.scene.borrow().bounds(mounted.find("parent").unwrap());
    assert!((more.width - parent.width).abs() < 0.01);
    assert!(more.width > 200.);
    child.set(true);
    ui.prepare_frame();
    ui.runtime.batch(|| {
        gesture(
            &ui,
            parent.x + parent.width - 5.,
            parent.y + parent.height / 2.,
        )
    });
    assert_eq!(calls.get(), 1);
    assert!(!root.get());
    assert!(!child.get());
    assert!(ui.input.focus_scope().is_none());
}
