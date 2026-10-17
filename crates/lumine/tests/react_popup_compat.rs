use lumine::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers, PointerButton},
    widgets::Ui,
};
use std::{cell::Cell, rc::Rc};

#[test]
fn custom_submenu_trigger_retains_identity_and_keyboard_handoff() {
    let mut ui = Ui::new(500., 400.);
    let outer = ui.signal(true);
    let inner = ui.signal(false);
    let root = ui.mount(
        menu("Outer", outer.clone(), button().id("anchor"))
            .p(0.)
            .child(
                submenu_with_anchor(
                    "Move to group",
                    inner.clone(),
                    menu_item("Move to group")
                        .id("custom-trigger")
                        .h(36.)
                        .child(text("Custom paint").pointer_events(false)),
                )
                .p(0.)
                .child(menu_item("Destination").id("destination")),
            ),
    );
    ui.prepare_frame();
    let trigger = root.find("custom-trigger").unwrap();
    assert_eq!(ui.input.focused(), Some(trigger));
    ui.dispatch(InputEvent::KeyDown {
        key: Key::ArrowRight,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    ui.prepare_frame();
    assert!(inner.get());
    assert_eq!(ui.input.focused(), root.find("destination"));
    ui.dispatch(InputEvent::KeyDown {
        key: Key::ArrowLeft,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    assert!(!inner.get());
    assert!(outer.get());
    assert_eq!(root.find("custom-trigger"), Some(trigger));
    assert_eq!(ui.input.focused(), Some(trigger));
}

#[test]
fn custom_slider_thumb_travel_matches_pointer_mapping_with_and_without_chrome() {
    for (thumb_width, chrome) in [(16., false), (0., false), (24., true)] {
        let mut ui = Ui::new(400., 100.);
        let value = ui.signal(0.);
        let root = ui.mount(
            slider("Custom thumb", value.clone(), 0. ..=100.)
                .size(240., 60.)
                .p(20.)
                .slider_chrome(chrome)
                .slider_step(0.5)
                .slider_thumb_width(thumb_width),
        );
        ui.prepare_frame();
        let children = ui.scene.borrow().children(root.node()).to_vec();
        let rail = ui.scene.borrow().bounds(children[0]);
        assert!((rail.width - (200. - thumb_width)).abs() < 0.001);
        assert!((rail.x - (20. + thumb_width / 2.)).abs() < 0.001);
        let x = rail.x + rail.width / 4.;
        for event in [
            InputEvent::PointerDown {
                x,
                y: 30.,
                button: PointerButton::Primary,
            },
            InputEvent::PointerUp {
                x,
                y: 30.,
                button: PointerButton::Primary,
            },
        ] {
            ui.dispatch(event);
        }
        assert!((value.get() - 25.).abs() < 0.001);
        ui.prepare_frame();
        let thumb = ui.scene.borrow().bounds(children[2]);
        assert!((thumb.width - thumb_width).abs() < 0.001);
        assert!((thumb.x + thumb.width / 2. - x).abs() < 0.001);
    }
}

#[test]
fn portal_motion_releases_input_during_exit_and_restarts_on_reopen() {
    use lumine::{
        compose::{PopupMotion, TaskRunner},
        frame::{Frame, FrameClock},
        motion::{Easing, Transition},
        task::LocalExecutor,
    };
    use std::{
        cell::RefCell,
        rc::Rc,
        time::{Duration, Instant},
    };
    let mut ui = Ui::new(500., 400.);
    let open = ui.signal(false);
    let calls = ui.signal(0);
    let hit = calls.clone();
    let executor = Rc::new(RefCell::new(LocalExecutor::new()));
    let frames = FrameClock::new();
    let view = ui.mount(provide(
        TaskRunner::from_executor(executor.clone()),
        provide(
            frames.clone(),
            overlay()
                .child(
                    button()
                        .id("behind")
                        .absolute()
                        .left(0.)
                        .top(30.)
                        .size(240., 80.)
                        .on_click(move || {
                            hit.update(|n| *n += 1);
                        }),
                )
                .child(
                    menu(
                        "Animated menu",
                        open.clone(),
                        button().id("anchor").size(100., 30.),
                    )
                    .p(0.)
                    .popup_motion(|| PopupMotion {
                        origin: [0., 1.],
                        enter: Transition::tween(Duration::from_millis(200), Easing::Linear),
                        exit: Transition::tween(Duration::from_millis(100), Easing::Linear),
                        enter_scale: 0.97,
                    })
                    .child(menu_item("Item").h(30.)),
                ),
        ),
    ));
    ui.prepare_frame();
    let start = Instant::now();
    let frame = |index, millis| {
        executor.borrow_mut().tick();
        frames.deliver(Frame {
            index,
            time: start + Duration::from_millis(millis),
            interval: Duration::from_millis(10),
        });
        executor.borrow_mut().tick();
    };
    open.set(true);
    ui.prepare_frame();
    let overlay = ui
        .input
        .focus_scope()
        .expect("opening must own focus immediately");
    frame(1, 0);
    frame(2, 50);
    ui.prepare_frame();
    let alpha = ui.scene.borrow().effects(overlay).opacity;
    assert!(
        alpha > 0. && alpha < 1.,
        "entry must render intermediate opacity"
    );
    open.set(false);
    ui.prepare_frame();
    assert!(
        ui.input.focus_scope().is_none(),
        "exit must release focus immediately"
    );
    assert!(
        ui.scene.borrow().effects(overlay).opacity > 0.,
        "paint must remain during exit"
    );
    for event in [
        InputEvent::PointerDown {
            x: 50.,
            y: 50.,
            button: PointerButton::Primary,
        },
        InputEvent::PointerUp {
            x: 50.,
            y: 50.,
            button: PointerButton::Primary,
        },
    ] {
        ui.dispatch(event);
    }
    assert_eq!(
        calls.get(),
        1,
        "fading portal blocked the underlying control"
    );
    open.set(true);
    ui.prepare_frame();
    assert_eq!(ui.input.focus_scope(), Some(overlay));
    assert!(
        ui.scene.borrow().effects(overlay).opacity <= f32::EPSILON,
        "reopening must replay entry"
    );
    frame(3, 60);
    frame(4, 280);
    ui.prepare_frame();
    assert_eq!(ui.scene.borrow().effects(overlay).opacity, 1.);
    open.set(false);
    frame(5, 290);
    frame(6, 410);
    ui.prepare_frame();
    assert_eq!(ui.scene.borrow().effects(overlay).opacity, 0.);
    assert!(view.find("behind").is_some());
}

#[test]
fn external_anchor_places_above_and_reactivates_repeated_trigger_clicks() {
    let mut ui = Ui::new(400., 300.);
    let open = ui.signal(false);
    let anchor = ui.signal(lumine::scene::Rect::default());
    let right = ui.signal(false);
    let show = open.clone();
    let pressed = Rc::new(Cell::new(0));
    let count = pressed.clone();
    let root = ui.mount(
        overlay()
            .w_full()
            .h_full()
            .child(
                button()
                    .id("trigger")
                    .absolute()
                    .left(100.)
                    .top(200.)
                    .w(80.)
                    .h(30.)
                    .observe_bounds(anchor.clone())
                    .on_click(move || {
                        count.set(count.get() + 1);
                        show.set(true);
                    }),
            )
            .child(
                menu("Imported menu", open.clone(), div().size(0., 0.))
                    .anchor_above_aligned(anchor, right.clone(), 6.)
                    .w(100.)
                    .p(0.)
                    .child(menu_item("Item").h(40.)),
            ),
    );
    let trigger = root.find("trigger").unwrap();
    for _ in 0..2 {
        ui.dispatch(InputEvent::PointerDown {
            x: 120.,
            y: 215.,
            button: PointerButton::Primary,
        });
        ui.dispatch(InputEvent::PointerUp {
            x: 120.,
            y: 215.,
            button: PointerButton::Primary,
        });
        ui.prepare_frame();
        assert!(open.get());
        assert!(ui.input.focus_scope().is_some());
    }
    assert_eq!(pressed.get(), 2);
    let panel = ui
        .semantics
        .borrow()
        .iter()
        .find_map(|(id, s)| (s.label == "Imported menu").then_some(id))
        .unwrap();
    let bounds = ui.scene.borrow().bounds(panel);
    let trigger_bounds = ui.scene.borrow().bounds(trigger);
    assert!((bounds.y + bounds.height + 6. - trigger_bounds.y).abs() < 0.1);
    assert!((bounds.x - trigger_bounds.x).abs() < 0.1);
    right.set(true);
    ui.prepare_frame();
    let bounds = ui.scene.borrow().bounds(panel);
    assert!((bounds.x + bounds.width - trigger_bounds.x - trigger_bounds.width).abs() < 0.1);
}

#[test]
fn external_anchor_places_below_with_right_alignment() {
    let mut ui = Ui::new(400., 300.);
    let open = ui.signal(true);
    let anchor = ui.signal(lumine::scene::Rect::new(100., 50., 80., 30.));
    let right = ui.signal(true);
    ui.mount(
        overlay().w_full().h_full().child(
            menu("Below select", open, div().size(0., 0.))
                .anchor_below_aligned(anchor, right, 6.)
                .w(100.)
                .p(0.)
                .child(menu_item("Option").h(40.)),
        ),
    );
    for _ in 0..4 {
        ui.prepare_frame();
    }
    let panel = ui
        .semantics
        .borrow()
        .iter()
        .find_map(|(id, semantics)| (semantics.label == "Below select").then_some(id))
        .unwrap();
    let bounds = ui.scene.borrow().bounds(panel);
    assert!((bounds.x - 80.).abs() < 0.01, "{bounds:?}");
    assert!((bounds.y - 86.).abs() < 0.01, "{bounds:?}");
}

#[test]
fn mount_callbacks_observe_the_retained_root_after_input_setup() {
    use std::cell::RefCell;
    let mut ui = Ui::new(300., 100.);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let inner = seen.clone();
    let outer = seen.clone();
    let input = ui.input.clone();
    let root = ui.mount(
        provide(
            (),
            component(move |_| {
                button().id("mounted").on_mount(move |node| {
                    assert!(input.options(node).is_some_and(|options| options.focusable));
                    inner.borrow_mut().push(node);
                })
            }),
        )
        .on_mount(move |node| outer.borrow_mut().push(node)),
    );
    assert_eq!(&*seen.borrow(), &[root.node(), root.node()]);
    assert_eq!(root.find("mounted"), Some(root.node()));
    ui.prepare_frame();
    assert_eq!(
        seen.borrow().len(),
        2,
        "mount callbacks cannot repeat during layout"
    );
}

#[test]
fn invisible_owner_releases_a_paused_portal_and_can_reopen() {
    use lumine::{
        compose::{PopupMotion, TaskRunner},
        frame::FrameClock,
        motion::{Easing, MotionPolicy, Transition},
        task::LocalExecutor,
    };
    use std::{cell::RefCell, time::Duration};
    let mut ui = Ui::new(500., 400.);
    let hidden = ui.signal(false);
    let visible = hidden.clone();
    let active = ui.signal(true);
    let open = ui.signal(true);
    let root = ui.mount(provide(
        TaskRunner::from_executor(Rc::new(RefCell::new(LocalExecutor::new()))),
        provide(
            FrameClock::new(),
            provide(
                MotionPolicy {
                    active: active.clone(),
                    reduced: ui.signal(false),
                },
                overlay()
                    .w_full()
                    .h_full()
                    .reactive_style(move || {
                        Styles::new().when(visible.get(), |style| style.invisible())
                    })
                    .child(
                        menu("Retained menu", open.clone(), button().size(100., 30.))
                            .id("retained-menu")
                            .popup_motion(|| PopupMotion {
                                enter: Transition::tween(
                                    Duration::from_millis(200),
                                    Easing::Linear,
                                ),
                                exit: Transition::tween(Duration::from_millis(100), Easing::Linear),
                                enter_scale: 0.97,
                                origin: [0., 0.],
                            })
                            .child(menu_item("Item")),
                    ),
            ),
        ),
    ));
    ui.prepare_frame();
    assert!(ui.input.focus_scope().is_some());
    active.set(false);
    hidden.set(true);
    ui.prepare_frame();
    assert!(ui.input.focus_scope().is_none());
    assert!(
        ui.scene
            .borrow()
            .visible_bounds(root.find("retained-menu").unwrap())
            .is_none()
    );
    hidden.set(false);
    active.set(true);
    open.set(true);
    ui.prepare_frame();
    assert!(ui.input.focus_scope().is_some());
}

#[test]
fn document_keyboard_listener_works_without_a_focused_control() {
    use lumine::input::EventPhase;
    use std::cell::RefCell;
    let mut ui = Ui::new(300., 100.);
    let root_id = ui.root();
    let targets = Rc::new(RefCell::new(Vec::new()));
    let recorded = targets.clone();
    let _listener = ui.input.listen(root_id, move |event| {
        if event.phase != EventPhase::Capture && matches!(event.event, InputEvent::KeyDown { .. }) {
            recorded.borrow_mut().push(event.target);
        }
    });
    let button = ui.mount(button().size(100., 30.));
    let key = || InputEvent::KeyDown {
        key: Key::Character("/".into()),
        modifiers: Modifiers::default(),
        repeat: false,
    };
    ui.dispatch(key());
    assert_eq!(&*targets.borrow(), &[root_id]);
    assert!(ui.input.focus(&ui.scene, Some(button.node())));
    ui.dispatch(key());
    assert_eq!(&*targets.borrow(), &[root_id, button.node()]);
}

#[test]
fn menu_radio_and_checkbox_roles_participate_in_keyboard_navigation() {
    use lumine::semantics::Role;
    let mut ui = Ui::new(300., 200.);
    let open = ui.signal(true);
    let clicks = Rc::new(Cell::new(0));
    let hit = clicks.clone();
    let root = ui.mount(
        menu("Choices", open.clone(), button().size(100., 30.))
            .child(
                button()
                    .id("radio")
                    .accessibility(|node| node.role = Role::MenuItemRadio),
            )
            .child(
                button()
                    .id("checkbox")
                    .accessibility(|node| node.role = Role::MenuItemCheckbox)
                    .on_click(move || hit.set(hit.get() + 1)),
            ),
    );
    ui.prepare_frame();
    assert_eq!(ui.input.focused(), root.find("radio"));
    ui.dispatch(InputEvent::KeyDown {
        key: Key::ArrowDown,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    assert_eq!(ui.input.focused(), root.find("checkbox"));
    ui.dispatch(InputEvent::KeyDown {
        key: Key::Enter,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    ui.dispatch(InputEvent::KeyUp {
        key: Key::Enter,
        modifiers: Modifiers::default(),
    });
    assert_eq!(clicks.get(), 1);
    assert!(
        open.get(),
        "a multi-select option owns its dismissal policy"
    );
}
