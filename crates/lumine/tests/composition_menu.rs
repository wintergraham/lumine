use lumine::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers, PointerButton},
    semantics::Role,
    widgets::Ui,
};
fn key(ui: &mut Ui, key: Key) {
    ui.dispatch(InputEvent::KeyDown {
        key,
        modifiers: Modifiers::default(),
        repeat: false,
    });
}
#[test]
fn menu_keyboard_wrap_skip_typeahead_and_close_before_action() {
    let mut ui = Ui::new(500., 400.);
    let open = ui.signal(false);
    let calls = ui.signal(0);
    let action = calls.clone();
    let state = open.clone();
    let view = ui.mount(
        menu("Actions", open.clone(), button().id("anchor").child("Menu"))
            .p(0.)
            .children([
                menu_item("Alpha").id("alpha"),
                menu_item("Disabled").id("disabled").disabled(true),
                menu_item("Beta").id("beta").on_click(move || {
                    assert!(!state.get());
                    action.set(action.get() + 1);
                }),
                menu_item("Bravo").id("bravo"),
            ]),
    );
    ui.prepare_frame();
    ui.input.focus(&ui.scene, view.find("anchor"));
    open.set(true);
    ui.prepare_frame();
    assert_eq!(ui.input.focused(), view.find("alpha"));
    key(&mut ui, Key::ArrowUp);
    assert_eq!(ui.input.focused(), view.find("bravo"));
    key(&mut ui, Key::ArrowDown);
    assert_eq!(ui.input.focused(), view.find("alpha"));
    key(&mut ui, Key::ArrowDown);
    assert_eq!(ui.input.focused(), view.find("beta"));
    key(&mut ui, Key::Character("b".into()));
    assert_eq!(ui.input.focused(), view.find("bravo"));
    key(&mut ui, Key::Character("b".into()));
    assert_eq!(ui.input.focused(), view.find("beta"));
    key(&mut ui, Key::Enter);
    ui.dispatch(InputEvent::KeyUp {
        key: Key::Enter,
        modifiers: Modifiers::default(),
    });
    assert_eq!(calls.get(), 1);
    assert!(!open.get());
    assert_eq!(ui.input.focused(), view.find("anchor"));
    let anchor = view.find("anchor").unwrap();
    assert_eq!(
        ui.semantics.borrow().get(anchor).unwrap().expanded,
        Some(false)
    );
}
#[test]
fn menu_tab_closes_and_leaves_in_each_direction() {
    let mut ui = Ui::new(500., 400.);
    let open = ui.signal(false);
    let view = ui.mount(
        column()
            .child(button().id("before"))
            .child(
                menu("Menu", open.clone(), button().id("anchor"))
                    .child(menu_item("Item").id("item")),
            )
            .child(button().id("after")),
    );
    ui.prepare_frame();
    ui.input.focus(&ui.scene, view.find("anchor"));
    open.set(true);
    key(&mut ui, Key::Tab);
    assert!(!open.get());
    assert_eq!(ui.input.focused(), view.find("after"));
    ui.input.focus(&ui.scene, view.find("anchor"));
    open.set(true);
    ui.dispatch(InputEvent::KeyDown {
        key: Key::Tab,
        modifiers: Modifiers {
            shift: true,
            ..Default::default()
        },
        repeat: false,
    });
    assert!(!open.get());
    assert_eq!(ui.input.focused(), view.find("before"));
}
#[test]
fn keyed_component_menu_items_retain_identity_and_recover_after_focused_removal() {
    let mut ui = Ui::new(500., 400.);
    let open = ui.signal(true);
    let items = ui.signal(vec![1, 2, 3]);
    let read = items.clone();
    let view = ui.mount(menu("Items", open, button()).child(keyed(
        move || read.get(),
        |n, _| component(move |_| menu_item(format!("Item {n}")).id(n.to_string())),
    )));
    ui.prepare_frame();
    key(&mut ui, Key::ArrowDown);
    let second = view.find("2").unwrap();
    assert_eq!(ui.input.focused(), Some(second));
    items.set(vec![3, 2, 1]);
    assert_eq!(view.find("2"), Some(second));
    key(&mut ui, Key::ArrowDown);
    assert_eq!(ui.input.focused(), view.find("1"));
    items.set(vec![3, 2]);
    assert_eq!(ui.input.focused(), None);
    key(&mut ui, Key::Home);
    assert_eq!(ui.input.focused(), view.find("3"));
    assert_eq!(
        ui.semantics.borrow().get(second).unwrap().role,
        Role::MenuItem
    );
}
#[test]
fn nested_menu_activation_closes_entire_chain_before_callback() {
    let mut ui = Ui::new(500., 400.);
    let outer = ui.signal(false);
    let inner = ui.signal(false);
    let first = outer.clone();
    let second = inner.clone();
    let calls = ui.signal(0);
    let count = calls.clone();
    let view = ui.mount(menu("Outer", outer.clone(), button().id("anchor")).child(
        menu("Inner", inner.clone(), button().id("subanchor")).child(
            menu_item("Action").id("action").on_click(move || {
                assert!(!first.get() && !second.get());
                count.set(count.get() + 1);
            }),
        ),
    ));
    ui.prepare_frame();
    ui.input.focus(&ui.scene, view.find("anchor"));
    outer.set(true);
    inner.set(true);
    ui.prepare_frame();
    ui.input.dispatch_to(
        &ui.scene,
        view.find("action").unwrap(),
        InputEvent::Activate,
    );
    assert_eq!(calls.get(), 1);
    assert!(ui.input.focus_scope().is_none());
    assert_eq!(ui.input.focused(), view.find("anchor"));
}
#[test]
fn disabled_menu_items_reject_pointer_and_invalid_leaf_cleanup() {
    let mut ui = Ui::new(500., 400.);
    let open = ui.signal(true);
    let calls = ui.signal(0);
    let count = calls.clone();
    let view = ui.mount(
        menu("Menu", open.clone(), button().w(50.).h(20.))
            .p(0.)
            .child(
                menu_item("Disabled")
                    .w(160.)
                    .h(30.)
                    .p(0.)
                    .disabled(true)
                    .id("disabled")
                    .on_click(move || {
                        count.set(count.get() + 1);
                    }),
            )
            .child(menu_item("Enabled").id("enabled")),
    );
    ui.prepare_frame();
    let bounds = ui.scene.borrow().bounds(view.find("disabled").unwrap());
    for event in [
        InputEvent::PointerDown {
            x: bounds.x + 10.,
            y: bounds.y + 10.,
            button: PointerButton::Primary,
        },
        InputEvent::PointerUp {
            x: bounds.x + 10.,
            y: bounds.y + 10.,
            button: PointerButton::Primary,
        },
    ] {
        ui.dispatch(event);
    }
    assert_eq!(calls.get(), 0);
    assert!(open.get());
    assert_eq!(ui.input.focused(), view.find("enabled"));
    view.unmount();
    let before = ui.scene.borrow().len();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || ui.mount(menu_item("Invalid"))
        ))
        .is_err()
    );
    assert_eq!(ui.scene.borrow().len(), before);
}

#[test]
fn repeated_typeahead_cycles_even_when_a_label_matches_doubled_letters() {
    let mut ui = Ui::new(400., 300.);
    let view = ui.mount(
        menu("Names", ui.signal(true), button().child("Names"))
            .child(menu_item("Aaron").id("aaron"))
            .child(menu_item("Abby").id("abby"))
            .child(menu_item("Alpha").id("alpha")),
    );
    ui.prepare_frame();
    for expected in ["abby", "alpha", "aaron"] {
        ui.dispatch(InputEvent::KeyDown {
            key: Key::Character("a".into()),
            modifiers: Modifiers::default(),
            repeat: false,
        });
        assert_eq!(ui.input.focused(), view.find(expected));
    }
}
