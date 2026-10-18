use lumine::{compose::prelude::*, widgets::Ui};

#[test]
fn flex_auto_margins_keep_axis_gaps_in_both_directions_and_after_resize() {
    for is_row in [false, true] {
        for reverse in [false, true] {
            for centered_auto in [false, true] {
                let mut ui = Ui::new(240., 240.);
                let mut container = if is_row { row() } else { column() };
                if reverse {
                    container = if is_row {
                        container.flex_row_reverse()
                    } else {
                        container.flex_col_reverse()
                    };
                }
                let children = (0..3).map(|index| {
                    let child = div().id(index.to_string()).size(20., 20.).shrink_0();
                    if centered_auto && index == 1 {
                        if is_row {
                            child.mx_auto()
                        } else {
                            child.my_auto()
                        }
                    } else if !centered_auto && index == 2 {
                        if is_row {
                            child.ml_auto()
                        } else {
                            child.mt_auto()
                        }
                    } else {
                        child
                    }
                });
                let view = ui.mount(
                    container
                        .size(
                            if is_row { 200. } else { 80. },
                            if is_row { 80. } else { 200. },
                        )
                        .items_start()
                        .justify_center()
                        .gap_x(12.)
                        .gap_y(12.)
                        .children(children),
                );
                ui.prepare_frame();
                let ids: Vec<_> = (0..3)
                    .map(|index| view.find(&index.to_string()).unwrap())
                    .collect();
                for main_size in [200., 84., 72., 200.] {
                    let mut style = ui.scene.borrow().style(view.node());
                    if is_row {
                        style.width = Some(main_size);
                    } else {
                        style.height = Some(main_size);
                    }
                    ui.scene.borrow_mut().set_style(view.node(), style);
                    ui.prepare_frame();
                    let expected = if main_size == 200. {
                        match (reverse, centered_auto) {
                            (false, false) => [0., 32., 180.],
                            (true, false) => [180., 148., 116.],
                            (false, true) => [0., 90., 180.],
                            (true, true) => [180., 90., 0.],
                        }
                    } else {
                        // Three 20px items plus two 12px gaps need 84px; overflow stays centered.
                        let start = (main_size - 84.) / 2.;
                        if reverse {
                            [start + 64., start + 32., start]
                        } else {
                            [start, start + 32., start + 64.]
                        }
                    };
                    let scene = ui.scene.borrow();
                    for (index, id) in ids.iter().enumerate() {
                        assert_eq!(view.find(&index.to_string()), Some(*id));
                        let rect = scene.layout_bounds(*id);
                        let actual = if is_row { rect.x } else { rect.y };
                        assert_eq!(
                            actual, expected[index],
                            "row={is_row} reverse={reverse} centered_auto={centered_auto} size={main_size} item={index}: {rect:?}"
                        );
                        assert_eq!((rect.width, rect.height), (20., 20.));
                    }
                }
            }
        }
    }
}
