use std::{cell::Cell, rc::Rc};
use lumine::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers, PointerButton},
    scene::NodeKind,
    semantics::Role,
    text_layout::FontFamily,
    widgets::Ui,
};

#[test]
fn checkbox_input_model_semantics_and_owned_cleanup() {
    let mut ui = Ui::new(500., 200.);
    let checked = ui.signal(false);
    let calls = Rc::new(Cell::new(0));
    let callback_calls = calls.clone();
    let callback_value = checked.clone();
    let mounted = ui.mount(
        checkbox("Enable notifications", checked.clone()).on_click(move || {
            callback_calls.set(callback_calls.get() + 1);
            assert_eq!(callback_value.get(), callback_calls.get() % 2 == 1);
        }),
    );
    let root = mounted.node();
    ui.scene.borrow_mut().flush();
    assert_eq!(
        ui.semantics.borrow().get(root).unwrap().role,
        Role::CheckBox
    );
    assert_eq!(
        ui.semantics.borrow().get(root).unwrap().label,
        "Enable notifications"
    );
    assert!(ui.input.focus(&ui.scene, Some(root)));
    for key in [Key::Space, Key::Enter] {
        ui.dispatch(InputEvent::KeyDown {
            key: key.clone(),
            modifiers: Modifiers::default(),
            repeat: false,
        });
        ui.dispatch(InputEvent::KeyUp {
            key,
            modifiers: Modifiers::default(),
        });
    }
    assert_eq!(calls.get(), 2);
    let label = ui.scene.borrow().children(root)[1];
    let bounds = ui.scene.borrow().bounds(label);
    ui.dispatch(InputEvent::PointerDown {
        x: bounds.x + 2.,
        y: bounds.y + 2.,
        button: PointerButton::Primary,
    });
    ui.dispatch(InputEvent::PointerUp {
        x: bounds.x + 2.,
        y: bounds.y + 2.,
        button: PointerButton::Primary,
    });
    assert_eq!(calls.get(), 3);
    assert!(checked.get());
    checked.set(false);
    assert_eq!(
        ui.semantics.borrow().get(root).unwrap().checked,
        Some(false)
    );
    let indicator = ui.scene.borrow().children(root)[0];
    assert!(
        matches!(ui.scene.borrow().kind(indicator), NodeKind::Text { text, .. } if text.as_ref() == "☐")
    );
    mounted.unmount();
    assert!(!ui.input.has_listeners(root));
    assert!(ui.semantics.borrow().get(root).is_none());
    checked.set(true);
    assert!(!ui.scene.borrow().contains(indicator));
}

#[test]
fn checkbox_inherits_typography_and_disabled_state_styles() {
    let mut ui = Ui::new(500., 200.);
    let checked = ui.signal(false);
    let disabled = ui.signal(false);
    let read_disabled = disabled.clone();
    let mounted = ui.mount(
        column()
            .font_family(FontFamily::Monospace)
            .text_size(22.)
            .child(
                checkbox("Settings", checked.clone())
                    .id("settings")
                    .focus(|style| style.bg(rgb(0x123456)))
                    .disabled_when(move || read_disabled.get())
                    .disabled_style(|style| style.opacity(0.3)),
            ),
    );
    let root = mounted.find("settings").unwrap();
    for child in ui.scene.borrow().children(root) {
        assert_eq!(ui.scene.borrow().font(*child).family, FontFamily::Monospace);
        assert!(
            matches!(ui.scene.borrow().kind(*child), NodeKind::Text { font_size, .. } if *font_size == 22.)
        );
    }
    assert!(ui.input.focus(&ui.scene, Some(root)));
    assert!(
        matches!(ui.scene.borrow().kind(root), NodeKind::Panel { quad, .. } if quad.fill == rgb(0x123456))
    );
    disabled.set(true);
    assert_eq!(ui.input.focused(), None);
    assert!(ui.semantics.borrow().get(root).unwrap().disabled);
    assert_eq!(ui.scene.borrow().effects(root).opacity, 0.3);
    ui.input.dispatch_to(&ui.scene, root, InputEvent::Activate);
    assert!(!checked.get());
    checked.set(true);
    assert_eq!(ui.semantics.borrow().get(root).unwrap().checked, Some(true));
    disabled.set(false);
    assert!(ui.input.focus(&ui.scene, Some(root)));
    ui.input.dispatch_to(&ui.scene, root, InputEvent::Activate);
    assert!(!checked.get());
}

#[test]
fn checkbox_rejects_extra_children_without_leaking_partial_mounts() {
    let mut ui = Ui::new(500., 200.);
    let checked = ui.signal(false);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ui.mount(checkbox("label", checked).child(text("extra")));
    }));
    assert!(result.is_err());
    assert!(ui.scene.borrow().children(ui.root()).is_empty());
}

#[test]
fn checkbox_indicator_expands_for_large_inherited_typography() {
    let mut ui = Ui::new(900., 300.);
    let checked = ui.signal(true);
    let mounted = ui.mount(column().text_size(64.).child(checkbox("Large", checked)));
    ui.scene.borrow_mut().flush();
    let scene = ui.scene.borrow();
    let control = scene.children(mounted.node())[0];
    let indicator = scene.children(control)[0];
    let label = scene.children(control)[1];
    assert!(scene.bounds(indicator).width > 24.);
    assert!(scene.bounds(label).x >= scene.bounds(indicator).x + scene.bounds(indicator).width);
}
