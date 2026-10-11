use std::{cell::Cell, rc::Rc};
use lumine::{compose::prelude::*, scene::Effects, widgets::Ui};

#[test]
fn reactive_nonfinite_effects_normalize_before_equality_and_restore_sparse_base() {
    for (invalid, opacity) in [(f32::NAN, 1.), (f32::INFINITY, 1.), (f32::NEG_INFINITY, 0.)] {
        let mut ui = Ui::new(200., 160.);
        let tick = ui.signal(0);
        let apply = ui.signal(true);
        let read_tick = tick.clone();
        let read_apply = apply.clone();
        let calls = Rc::new(Cell::new(0));
        let count = calls.clone();
        let view = ui.mount(
            div()
                .w(80.)
                .h(60.)
                .opacity(0.4)
                .blur(6.)
                .edge_fade(8.)
                .reactive_style(move || {
                    read_tick.get();
                    count.set(count.get() + 1);
                    if read_apply.get() {
                        Styles::new()
                            .opacity(invalid)
                            .blur(invalid)
                            .edge_fade(invalid)
                    } else {
                        Styles::new()
                    }
                }),
        );
        ui.prepare_frame();
        let expected = Effects {
            opacity,
            blur_radius: 0.,
            edge_fade: 0.,
        };
        assert_eq!(ui.scene.borrow().effects(view.node()), expected);
        ui.scene.borrow_mut().flush();
        let initial_calls = calls.get();
        for value in 1..=3 {
            tick.set(value);
            ui.prepare_frame();
            assert_eq!(calls.get(), initial_calls + value);
            assert_eq!(ui.scene.borrow().effects(view.node()), expected);
            let frame = ui.scene.borrow_mut().flush();
            assert!(
                frame.is_idle(),
                "equal normalized effects should remain idle: {frame:?}"
            );
            assert_eq!(frame.layout_nodes, 0);
            assert_eq!(frame.composite_nodes, 0);
            assert!(frame.damage.is_empty());
        }
        apply.set(false);
        ui.prepare_frame();
        assert_eq!(
            ui.scene.borrow().effects(view.node()),
            Effects {
                opacity: 0.4,
                blur_radius: 6.,
                edge_fade: 8.
            }
        );
        let frame = ui.scene.borrow_mut().flush();
        assert_eq!(frame.layout_nodes, 0);
        assert!(!frame.damage.is_empty());
        apply.set(true);
        ui.prepare_frame();
        assert_eq!(ui.scene.borrow().effects(view.node()), expected);
        ui.scene.borrow_mut().flush();
        assert!(ui.scene.borrow_mut().flush().is_idle());
    }
}
