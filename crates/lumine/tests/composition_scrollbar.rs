use lumine::compose::TaskRunner;
use lumine::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers, PointerButton},
    scene::NodeId,
    semantics::Role,
    widgets::Ui,
};
fn bar(ui: &Ui) -> NodeId {
    ui.semantics
        .borrow()
        .iter()
        .find(|(_, n)| n.role == Role::ScrollBar)
        .unwrap()
        .0
}
fn down(ui: &mut Ui, x: f32, y: f32) {
    ui.dispatch(InputEvent::PointerDown {
        x,
        y,
        button: PointerButton::Primary,
    });
}
fn up(ui: &mut Ui, x: f32, y: f32) {
    ui.dispatch(InputEvent::PointerUp {
        x,
        y,
        button: PointerButton::Primary,
    });
}
#[test]
fn scrollbar_track_paging_thumb_capture_resize_and_cancel() {
    let mut ui = Ui::new(500., 400.);
    let offset = ui.signal(0.);
    let height = ui.signal(120.);
    let read = height.clone();
    ui.mount(
        scroll(offset.clone())
            .w(200.)
            .p(10.)
            .reactive_style(move || Styles::new().h(read.get()))
            .scrollbar(true)
            .child(div().h(400.)),
    );
    ui.prepare_frame();
    let bar = bar(&ui);
    let bounds = ui.scene.borrow().bounds(bar);
    assert_eq!(
        (bounds.x, bounds.y, bounds.width, bounds.height),
        (178., 10., 12., 100.)
    );
    down(&mut ui, 186., 90.);
    up(&mut ui, 186., 90.);
    assert_eq!(offset.get(), 100.);
    assert_eq!(ui.input.focused(), Some(bar));
    ui.dispatch(InputEvent::KeyDown {
        key: Key::End,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    assert_eq!(offset.get(), 300.);
    offset.set(100.);
    down(&mut ui, 186., 40.);
    assert_eq!(ui.input.captured(), Some(bar));
    height.set(220.);
    ui.prepare_frame();
    ui.dispatch(InputEvent::PointerMove { x: 400., y: 390. });
    assert_eq!(offset.get(), 200.);
    ui.dispatch(InputEvent::PointerCancel);
    assert_eq!(ui.input.captured(), None);
    ui.dispatch(InputEvent::PointerMove { x: 186., y: 20. });
    assert_eq!(offset.get(), 200.);
}
#[test]
fn horizontal_scrollbar_keyboard_numeric_and_translation_only() {
    let mut ui = Ui::new(500., 300.);
    let offset = ui.signal(0.);
    let view = ui.mount(
        scroll_x(offset.clone())
            .w(200.)
            .h(80.)
            .scrollbar(true)
            .child(div().w(800.)),
    );
    ui.prepare_frame();
    let bar = bar(&ui);
    ui.scene.borrow_mut().flush();
    assert!(ui.input.focus(&ui.scene, Some(bar)));
    for (key, expected) in [
        (Key::PageDown, 200.),
        (Key::ArrowRight, 240.),
        (Key::End, 600.),
        (Key::PageUp, 400.),
        (Key::Home, 0.),
    ] {
        ui.dispatch(InputEvent::KeyDown {
            key,
            modifiers: Modifiers::default(),
            repeat: false,
        });
        assert_eq!(offset.get(), expected);
        ui.prepare_frame();
        assert_eq!(ui.scene.borrow_mut().flush().layout_nodes, 0);
    }
    ui.input
        .dispatch_to(&ui.scene, bar, InputEvent::SetNumericValue(125.));
    assert_eq!(offset.get(), 125.);
    assert_eq!(
        ui.semantics.borrow().get(bar).unwrap().numeric_value,
        Some(125.)
    );
    view.unmount();
    assert!(!ui.input.has_listeners(bar));
}
#[test]
fn hidden_scrollbar_releases_focus_and_allows_underlying_button() {
    let mut ui = Ui::new(500., 300.);
    let offset = ui.signal(0.);
    let tall = ui.signal(true);
    let read = tall.clone();
    let calls = ui.signal(0);
    let click = calls.clone();
    let view = ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(100.)
            .scrollbar(true)
            .child(
                button()
                    .w(200.)
                    .id("button")
                    .reactive_style(move || Styles::new().h(if read.get() { 400. } else { 100. }))
                    .on_click(move || {
                        click.set(click.get() + 1);
                    }),
            ),
    );
    ui.prepare_frame();
    let bar = bar(&ui);
    down(&mut ui, 50., 50.);
    up(&mut ui, 50., 50.);
    assert_eq!(calls.get(), 1);
    ui.input.focus(&ui.scene, Some(bar));
    tall.set(false);
    ui.prepare_frame();
    assert!(ui.semantics.borrow().get(bar).is_none());
    assert_ne!(ui.input.focused(), Some(bar));
    down(&mut ui, 196., 50.);
    up(&mut ui, 196., 50.);
    assert_eq!(calls.get(), 2);
    tall.set(true);
    ui.prepare_frame();
    assert_eq!(self::bar(&ui), bar);
    view.unmount();
    offset.set(1000.);
    assert_eq!(offset.get(), 1000.);
}
#[test]
fn virtual_scrollbar_tracks_extent_and_leaf_validation_cleans_mount() {
    let mut ui = Ui::new(500., 300.);
    let offset = ui.signal(0.);
    let count = ui.signal(100);
    let read = count.clone();
    let view = ui.mount(
        virtual_list(
            offset.clone(),
            20.,
            1,
            move || read.get(),
            |n| n,
            |_, n, _| text(n.to_string()),
        )
        .w(200.)
        .h(100.)
        .scrollbar(true),
    );
    ui.prepare_frame();
    let bar = bar(&ui);
    assert_eq!(ui.semantics.borrow().get(bar).unwrap().max, Some(1900.));
    ui.input
        .dispatch_to(&ui.scene, bar, InputEvent::SetValue("500".into()));
    assert_eq!(offset.get(), 500.);
    count.set(2);
    ui.prepare_frame();
    assert_eq!(offset.get(), 0.);
    assert!(ui.semantics.borrow().get(bar).is_none());
    let before = ui.scene.borrow().len();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || ui.mount(column().child(text("bad").scrollbar(true)))
        ))
        .is_err()
    );
    assert_eq!(ui.scene.borrow().len(), before);
    assert!(view.is_mounted());
}

#[test]
fn thumb_drag_survives_unrelated_button_release() {
    let mut ui = Ui::new(500., 400.);
    let offset = ui.signal(0.);
    ui.mount(
        scroll(offset.clone())
            .size(200., 120.)
            .scrollbar(true)
            .child(div().h(600.)),
    );
    ui.prepare_frame();
    let track = bar(&ui);
    down(&mut ui, 196., 10.);
    assert_eq!(ui.input.captured(), Some(track));
    ui.dispatch(InputEvent::PointerDown {
        x: 196.,
        y: 10.,
        button: PointerButton::Secondary,
    });
    ui.dispatch(InputEvent::PointerUp {
        x: 196.,
        y: 10.,
        button: PointerButton::Secondary,
    });
    assert_eq!(ui.input.captured(), Some(track));
    ui.dispatch(InputEvent::PointerMove { x: 196., y: 70. });
    assert!(offset.get() > 0.);
    up(&mut ui, 196., 70.);
    assert_eq!(ui.input.captured(), None);
    let settled = offset.get();
    ui.dispatch(InputEvent::PointerMove { x: 196., y: 100. });
    assert_eq!(offset.get(), settled);
}

#[test]
fn scrollbars_are_default_for_both_axes_and_virtual_lists_with_explicit_opt_out() {
    for horizontal in [false, true] {
        let mut ui = Ui::new(500., 300.);
        let offset = ui.signal(0.);
        let view = if horizontal {
            scroll_x(offset).size(200., 100.).child(div().w(600.))
        } else {
            scroll(offset).size(200., 100.).child(div().h(600.))
        };
        ui.mount(view);
        ui.prepare_frame();
        assert!(ui.semantics.borrow().get(bar(&ui)).is_some());
    }
    let mut ui = Ui::new(500., 300.);
    let offset = ui.signal(0.);
    ui.mount(
        virtual_list(offset, 20., 1, || 100, |n| n, |_, n, _| text(n.to_string())).size(200., 100.),
    );
    ui.prepare_frame();
    assert!(ui.semantics.borrow().get(bar(&ui)).is_some());

    let mut ui = Ui::new(500., 300.);
    let offset = ui.signal(0.);
    ui.mount(
        scroll(offset)
            .size(200., 100.)
            .scrollbar(false)
            .child(div().h(600.)),
    );
    ui.prepare_frame();
    assert!(
        !ui.semantics
            .borrow()
            .iter()
            .any(|(_, n)| n.role == Role::ScrollBar)
    );
}

#[test]
fn overlay_pill_keeps_content_width_and_grows_inward_during_hover_and_capture() {
    use lumine::scene::{Color, NodeKind};
    let mut ui = Ui::new(500., 300.);
    let offset = ui.signal(0.);
    let view = ui.mount(
        scroll(offset)
            .size(200., 100.)
            .child(div().id("content").w_full().h(600.)),
    );
    ui.prepare_frame();
    let track = bar(&ui);
    let thumb = ui.scene.borrow().children(track)[0];
    let pill = |ui: &Ui| {
        let scene = ui.scene.borrow();
        let NodeKind::Quad(quad) = scene.kind(thumb) else {
            panic!("pill")
        };
        (scene.bounds(thumb), quad.fill, quad.radius)
    };
    let (rest, color, radius) = pill(&ui);
    assert_eq!(
        ui.scene
            .borrow()
            .bounds(view.find("content").unwrap())
            .width,
        200.
    );
    assert_eq!(
        (rest.x, rest.y, rest.width, rest.height),
        (193., 2., 4., 32.)
    );
    assert_eq!(color, Color(255, 255, 255, 0));
    assert_eq!(radius, 2.);
    ui.dispatch(InputEvent::PointerMove { x: 194., y: 10. });
    ui.prepare_frame();
    let (hover, color, radius) = pill(&ui);
    assert_eq!((hover.x, hover.width), (191., 6.));
    assert_eq!(hover.x + hover.width, rest.x + rest.width);
    assert_eq!(color, Color(255, 255, 255, 89));
    assert_eq!(radius, 3.);
    down(&mut ui, 194., 10.);
    ui.dispatch(InputEvent::PointerMove { x: 400., y: 50. });
    ui.prepare_frame();
    assert_eq!(pill(&ui).1, Color(255, 255, 255, 128));
    assert_eq!(ui.input.captured(), Some(track));
    up(&mut ui, 400., 50.);
    ui.prepare_frame();
    assert_eq!(pill(&ui).1.3, 0);
}

#[test]
fn idle_fade_hover_growth_and_drag_hold_follow_the_display_clock() {
    use std::{
        cell::RefCell,
        rc::Rc,
        time::{Duration, Instant},
    };
    use lumine::{
        compose::TaskRunner,
        frame::{Frame, FrameClock},
        scene::NodeKind,
        task::LocalExecutor,
    };
    let executor = Rc::new(RefCell::new(LocalExecutor::new()));
    let frames = FrameClock::new();
    let mut ui = Ui::new(500., 300.);
    let offset = ui.signal(0.);
    let view = ui.mount(provide(
        TaskRunner::from_executor(executor.clone()),
        provide(
            frames.clone(),
            scroll(offset.clone()).size(200., 100.).child(div().h(600.)),
        ),
    ));
    ui.prepare_frame();
    let track = bar(&ui);
    let thumb = ui.scene.borrow().children(track)[0];
    let sample = |ui: &Ui| {
        let scene = ui.scene.borrow();
        let NodeKind::Quad(q) = scene.kind(thumb) else {
            panic!("pill")
        };
        (q.fill.3, scene.bounds(thumb).width)
    };
    let mut time = Instant::now();
    let mut index = 0;
    let mut advance = |ui: &mut Ui, millis: u64| {
        executor.borrow_mut().tick();
        time += Duration::from_millis(millis);
        index += 1;
        frames.deliver(Frame {
            index,
            time,
            interval: Duration::from_millis(16),
        });
        executor.borrow_mut().tick();
        ui.prepare_frame();
    };
    assert_eq!(sample(&ui), (0, 4.));
    offset.set(100.);
    advance(&mut ui, 45);
    assert!((24..=27).contains(&sample(&ui).0));
    advance(&mut ui, 50);
    assert_eq!(sample(&ui), (51, 4.));
    advance(&mut ui, 695);
    assert_eq!(sample(&ui).0, 51);
    advance(&mut ui, 110);
    assert!((24..=27).contains(&sample(&ui).0));
    advance(&mut ui, 110);
    assert_eq!(sample(&ui).0, 0);
    assert!(!frames.wants_frame());
    ui.dispatch(InputEvent::PointerMove { x: 194., y: 35. });
    advance(&mut ui, 70);
    assert!(sample(&ui).1 > 4. && sample(&ui).1 < 6.);
    advance(&mut ui, 80);
    assert_eq!(sample(&ui), (89, 6.));
    down(&mut ui, 194., 35.);
    advance(&mut ui, 100);
    assert_eq!(sample(&ui), (128, 6.));
    ui.dispatch(InputEvent::PointerMove { x: 400., y: 50. });
    advance(&mut ui, 1100);
    assert_eq!(sample(&ui), (128, 6.));
    assert_eq!(ui.input.captured(), Some(track));
    up(&mut ui, 400., 50.);
    advance(&mut ui, 1100);
    assert_eq!(sample(&ui), (0, 4.));
    view.unmount();
    executor.borrow_mut().tick();
    assert!(!frames.wants_frame());
}

#[test]
fn classic_scrollbar_persists_without_idle_frames_and_reuses_input() {
    use lumine::{
        frame::{Frame, FrameClock},
        scene::{Color, NodeKind},
        task::LocalExecutor,
    };
    use std::{
        cell::RefCell,
        rc::Rc,
        time::{Duration, Instant},
    };
    for horizontal in [false, true] {
        let executor = Rc::new(RefCell::new(LocalExecutor::new()));
        let frames = FrameClock::new();
        let mut ui = Ui::new(500., 300.);
        let offset = ui.signal(0.);
        let overflowing = ui.signal(true);
        let read = overflowing.clone();
        let view = if horizontal {
            scroll_x(offset.clone())
        } else {
            scroll(offset.clone())
        };
        let view = ui.mount(provide(
            TaskRunner::from_executor(executor.clone()),
            provide(
                frames.clone(),
                view.size(200., 100.)
                    .scrollbar_style(ScrollbarStyle::Classic)
                    .child(div().id("content").reactive_style(move || {
                        let length = if read.get() { 800. } else { 50. };
                        if horizontal {
                            Styles::new().w(length).h(50.)
                        } else {
                            Styles::new().w_full().h(length)
                        }
                    })),
            ),
        ));
        ui.prepare_frame();
        let track = bar(&ui);
        let thumb = ui.scene.borrow().children(track)[0];
        let sample = |ui: &Ui| {
            let scene = ui.scene.borrow();
            let NodeKind::Panel {
                quad: track_paint, ..
            } = scene.kind(track)
            else {
                panic!("classic track")
            };
            let NodeKind::Quad(paint) = scene.kind(thumb) else {
                panic!("classic thumb")
            };
            (
                track_paint.fill,
                paint.fill,
                paint.border_color,
                scene.bounds(track),
                scene.bounds(thumb),
            )
        };
        let (track_color, thumb_color, edge_color, track_bounds, thumb_bounds) = sample(&ui);
        assert_eq!(track_color, Color(44, 44, 44, 255));
        assert_eq!(thumb_color, Color(107, 107, 107, 255));
        assert_eq!(edge_color, Color(81, 81, 81, 255));
        assert_eq!(
            if horizontal {
                track_bounds.height
            } else {
                track_bounds.width
            },
            15.
        );
        assert_eq!(
            if horizontal {
                thumb_bounds.height
            } else {
                thumb_bounds.width
            },
            8.
        );
        if !horizontal {
            assert_eq!(
                ui.scene
                    .borrow()
                    .bounds(view.find("content").unwrap())
                    .width,
                200.
            );
        }
        offset.set(100.);
        ui.prepare_frame();
        executor.borrow_mut().tick();
        assert!(!frames.wants_frame());
        frames.deliver(Frame {
            index: 1,
            time: Instant::now() + Duration::from_secs(2),
            interval: Duration::from_millis(16),
        });
        executor.borrow_mut().tick();
        ui.prepare_frame();
        assert_eq!(sample(&ui).1, thumb_color);
        assert!(!frames.wants_frame());
        let (_, _, _, _, thumb_bounds) = sample(&ui);
        down(&mut ui, thumb_bounds.x + 4., thumb_bounds.y + 4.);
        assert_eq!(ui.input.captured(), Some(track));
        ui.dispatch(InputEvent::PointerMove { x: 400., y: 250. });
        assert!(offset.get() > 100.);
        ui.dispatch(InputEvent::PointerCancel);
        ui.input
            .dispatch_to(&ui.scene, track, InputEvent::SetNumericValue(f64::MAX));
        assert_eq!(offset.get(), if horizontal { 600. } else { 700. });
        overflowing.set(false);
        ui.prepare_frame();
        assert_eq!(offset.get(), 0.);
        assert!(ui.semantics.borrow().get(track).is_none());
        assert_eq!(ui.scene.borrow().bounds(track).width, 0.);
        view.unmount();
        executor.borrow_mut().tick();
        assert!(!frames.wants_frame());
    }
}

#[test]
fn scrollbar_style_survives_component_refinement_and_virtual_list_mount() {
    for variable in [false, true] {
        let mut ui = Ui::new(500., 300.);
        let offset = ui.signal(0.);
        ui.mount(
            component(move |cx| {
                if variable {
                    variable_virtual_list(
                        offset,
                        VariableHeights::new(&cx.runtime(), 100, 20.),
                        1,
                        |n| n,
                        |_, n, _| text(n.to_string()),
                    )
                } else {
                    virtual_list(offset, 20., 1, || 100, |n| n, |_, n, _| text(n.to_string()))
                }
            })
            .size(200., 100.)
            .scrollbar_style(ScrollbarStyle::Classic),
        );
        ui.prepare_frame();
        assert_eq!(ui.scene.borrow().bounds(bar(&ui)).width, 15.);
    }
    for view_style in [ScrollbarStyle::Hidden, ScrollbarStyle::Classic] {
        let mut ui = Ui::new(500., 300.);
        let offset = ui.signal(0.);
        ui.mount(
            scroll(offset)
                .size(200., 100.)
                .scrollbar_style(view_style)
                .scrollbar(false)
                .child(div().h(600.)),
        );
        ui.prepare_frame();
        assert!(
            !ui.semantics
                .borrow()
                .iter()
                .any(|(_, node)| node.role == Role::ScrollBar)
        );
    }
}
