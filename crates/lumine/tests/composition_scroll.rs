use std::{cell::Cell, rc::Rc};
use lumine::{compose::prelude::*, input::InputEvent, semantics::Role, widgets::Ui};
fn wheel(ui: &mut Ui, x: f32, y: f32, delta_y: f32) {
    ui.dispatch(InputEvent::Scroll {
        x,
        y,
        delta_x: 0.,
        delta_y,
    });
}
#[test]
fn ordinary_scroll_measures_dynamic_children_and_reclamps_on_resize() {
    let mut ui = Ui::new(600., 400.);
    let offset = ui.signal(0.);
    let expanded = ui.signal(false);
    let read_expanded = expanded.clone();
    let height = ui.signal(120.);
    let read_height = height.clone();
    let view = ui.mount(
        scroll(offset.clone())
            .w(220.)
            .p(10.)
            .reactive_style(move || Styles::new().h(read_height.get()))
            .child(div().h(150.))
            .child(switch(
                move || read_expanded.get(),
                |expanded, _| {
                    if expanded {
                        div().h(250.)
                    } else {
                        div().h(50.)
                    }
                },
            )),
    );
    ui.prepare_frame();
    offset.set(10000.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 100.);
    expanded.set(true);
    ui.prepare_frame();
    offset.set(10000.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 300.);
    height.set(220.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 200.);
    expanded.set(false);
    ui.prepare_frame();
    assert_eq!(offset.get(), 0.);
    assert_eq!(
        ui.semantics.borrow().get(view.node()).unwrap().role,
        Role::ScrollView
    );
}
#[test]
fn nested_scroll_consumes_motion_until_edge_then_bubbles() {
    let mut ui = Ui::new(500., 400.);
    let outer = ui.signal(0.);
    let inner = ui.signal(0.);
    ui.mount(
        scroll(outer.clone())
            .w(300.)
            .h(160.)
            .child(
                scroll(inner.clone())
                    .w(200.)
                    .h(100.)
                    .child(div().w(200.).h(300.)),
            )
            .child(div().h(400.)),
    );
    ui.prepare_frame();
    wheel(&mut ui, 20., 20., 100.);
    assert_eq!((inner.get(), outer.get()), (100., 0.));
    wheel(&mut ui, 20., 20., 100.);
    assert_eq!((inner.get(), outer.get()), (200., 0.));
    wheel(&mut ui, 20., 20., 40.);
    assert_eq!((inner.get(), outer.get()), (200., 40.));
    wheel(&mut ui, 20., 20., -50.);
    assert_eq!((inner.get(), outer.get()), (150., 40.));
}
#[test]
fn offset_updates_are_compositor_only_and_children_retain_local_state() {
    let mut ui = Ui::new(500., 300.);
    let offset = ui.signal(0.);
    let mounts = Rc::new(Cell::new(0));
    let observed = mounts.clone();
    let view = ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(100.)
            .child(component(move |cx| {
                observed.set(observed.get() + 1);
                let count = cx.state(42);
                column()
                    .h(500.)
                    .child(text_signal(move || count.get().to_string()).id("local"))
            })),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let child = view.find("local").unwrap();
    for n in [25., 100., 350., 0.] {
        offset.set(n);
        ui.prepare_frame();
        let frame = ui.scene.borrow_mut().flush();
        assert_eq!(frame.layout_nodes, 0);
        assert_eq!(view.find("local"), Some(child));
    }
    assert_eq!(mounts.get(), 1);
}
#[test]
fn disabled_scroll_blocks_input_and_unmount_releases_observers() {
    let mut ui = Ui::new(500., 300.);
    let offset = ui.signal(0.);
    let view = ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(100.)
            .disabled(true)
            .child(div().h(500.)),
    );
    ui.prepare_frame();
    assert!(ui.semantics.borrow().get(view.node()).unwrap().disabled);
    wheel(&mut ui, 20., 20., 50.);
    assert_eq!(offset.get(), 0.);
    offset.set(f32::INFINITY);
    ui.prepare_frame();
    assert_eq!(offset.get(), 400.);
    offset.set(f32::NAN);
    assert_eq!(offset.get(), 0.);
    view.unmount();
    assert!(!ui.input.has_listeners(view.node()));
    assert!(ui.semantics.borrow().get(view.node()).is_none());
    offset.set(900.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 900.);
}

#[test]
fn scroll_padding_preserves_badge_paint_and_hits_without_changing_content_or_range() {
    use lumine::scene::Color;
    for horizontal in [false, true] {
        for padded in [false, true] {
            let mut ui = Ui::new(500., 300.);
            let offset = ui.signal(0.);
            let redistributed = ui.signal(false);
            let read = redistributed.clone();
            let calls = ui.signal(0);
            let clicked = calls.clone();
            let view = if horizontal {
                scroll_x(offset.clone())
            } else {
                scroll(offset.clone())
            };
            let view = ui.mount(
                view.size(200., 100.)
                    .scrollbar(false)
                    .reactive_style(move || {
                        if !padded {
                            Styles::new()
                        } else if read.get() {
                            Styles::new().pl(14.).pr(6.).pt(6.).pb(14.)
                        } else {
                            Styles::new().p(10.)
                        }
                    })
                    .child(
                        div()
                            .id("attachment")
                            .size(
                                50.,
                                if horizontal {
                                    if padded { 80. } else { 100. }
                                } else {
                                    50.
                                },
                            )
                            .child(
                                button()
                                    .id("badge")
                                    .absolute()
                                    .left(-4.5)
                                    .bottom(-7.5)
                                    .size(20., 20.)
                                    .bg(Color(255, 0, 0, 255))
                                    .on_click(move || clicked.set(clicked.get() + 1)),
                            ),
                    )
                    .child(div().size(400., 400.)),
            );
            ui.prepare_frame();
            let badge = view.find("badge").unwrap();
            let attachment = view.find("attachment").unwrap();
            let assert_paint = |ui: &Ui| {
                let scene = ui.scene.borrow();
                let bounds = scene.bounds(badge);
                let visible = scene.visible_bounds(badge).unwrap();
                if padded {
                    assert_eq!(visible, bounds);
                } else {
                    assert!((visible.x - bounds.x - 4.5).abs() < 0.001);
                    if horizontal {
                        assert!((bounds.height - visible.height - 7.5).abs() < 0.001);
                    }
                }
                bounds
            };
            let bounds = assert_paint(&ui);
            if padded {
                let x = bounds.x + 1.;
                let y = bounds.y + bounds.height - 1.;
                ui.dispatch(InputEvent::PointerDown {
                    x,
                    y,
                    button: lumine::input::PointerButton::Primary,
                });
                ui.dispatch(InputEvent::PointerUp {
                    x,
                    y,
                    button: lumine::input::PointerButton::Primary,
                });
                assert_eq!(calls.get(), 1);
            }
            let initial = ui.scene.borrow().layout_bounds(attachment);
            assert_eq!(initial.x, if padded { 10. } else { 0. });
            assert_eq!(initial.y, if padded { 10. } else { 0. });
            offset.set(f32::INFINITY);
            assert_eq!(
                offset.get(),
                if horizontal {
                    450. - if padded { 180. } else { 200. }
                } else {
                    450. - if padded { 80. } else { 100. }
                }
            );
            offset.set(0.);
            redistributed.set(true);
            ui.prepare_frame();
            assert_paint(&ui);
            if padded {
                let bounds = ui.scene.borrow().layout_bounds(attachment);
                assert_eq!((bounds.x, bounds.y), (14., 6.));
            }
        }
    }
}
