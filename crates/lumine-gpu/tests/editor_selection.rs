use std::{cell::RefCell, rc::Rc};
use lumine::{
    compose::prelude::*,
    input::{InputEvent, Modifiers, PointerButton},
    text_layout::{FontStyle, TextLayout},
    widgets::Ui,
};
use lumine_gpu::text::ShapedText;

#[test]
fn native_selection_has_rounded_pixels_even_at_a_scrolled_viewport_edge() {
    use lumine::scene::{Color, Insets, NodeKind};
    let mut gpu = lumine_gpu::GpuRenderer::new(128, 40).unwrap();
    for all in [false, true] {
        let mut ui = Ui::new(128., 40.);
        let selection = Color(57, 57, 51, 255);
        let root = ui.mount(
            text_input(
                "Address",
                ui.signal("linear.app/workspace/inbox/more/text/for/scroll".into()),
            )
            .size(128., 40.)
            .p(0.)
            .border(0.)
            .bg(Color(0, 0, 0, 0))
            .text_color(Color(0, 0, 0, 0))
            .text_size(13.)
            .selection_style(
                selection,
                2.,
                Insets {
                    left: 1.,
                    right: 1.,
                    top: 0.,
                    bottom: 0.,
                },
            ),
        );
        ui.prepare_frame();
        ui.input.focus(&ui.scene, Some(root.node()));
        let editor = ui.focused_editor().unwrap();
        if all {
            editor.select_all();
        } else {
            editor.set_selection(0, 0);
            editor.set_selection(2, 8);
        }
        ui.prepare_frame();
        let scene = ui.scene.borrow();
        let bounds = scene
            .paint_items()
            .find(|item| matches!(item.kind, NodeKind::Quad(quad) if quad.fill == selection))
            .unwrap()
            .bounds;
        gpu.render(&scene, &[scene.bounds(scene.root())]).unwrap();
        let pixels = gpu.readback().unwrap();
        let alpha = |x: usize, y: usize| pixels[(y * 128 + x) * 4 + 3];
        let x = bounds.x.floor().max(0.) as usize;
        let y = bounds.y.floor().max(0.) as usize;
        assert!(
            alpha(x, y) < alpha(x + 3, y),
            "selection corner must be rounded: {bounds:?}"
        );
        assert_eq!(alpha(x + 3, y + 3), 255);
        assert!(bounds.x >= 0. && bounds.x + bounds.width <= 128.);
    }
}

#[test]
fn triple_click_selects_full_logical_row_in_mixed_direction_text() {
    let fonts = Rc::new(RefCell::new(cosmic_text::FontSystem::new()));
    for text in ["abc אבג", "אבג abc", "abc אבג xyz דהו", "abc אבג  "] {
        let mut ui = Ui::new(500., 200.);
        let fonts = fonts.clone();
        ui.scene.borrow_mut().set_font_text_shaper(
            move |text: &str, size, width, font: &FontStyle| -> Box<dyn TextLayout> {
                Box::new(ShapedText::with_font(
                    &mut fonts.borrow_mut(),
                    text,
                    size,
                    width,
                    font,
                ))
            },
        );
        let view = ui.mount(
            text_area("Bidi document", ui.signal(format!("{text}\nnext row")))
                .size(440., 120.)
                .p(10.)
                .text_size(16.)
                .line_height(24.),
        );
        ui.prepare_frame();
        ui.input.focus(&ui.scene, Some(view.node()));
        let editor = ui.focused_editor().unwrap();
        editor.editor.borrow_mut().set_selection(0, 0);
        editor.refresh();
        for _ in 0..3 {
            ui.dispatch_with_modifiers(
                InputEvent::PointerDown {
                    x: 40.,
                    y: 22.,
                    button: PointerButton::Primary,
                },
                Modifiers::default(),
            );
            ui.dispatch_with_modifiers(
                InputEvent::PointerUp {
                    x: 40.,
                    y: 22.,
                    button: PointerButton::Primary,
                },
                Modifiers::default(),
            );
        }
        assert_eq!(
            editor.copy(),
            text,
            "complete bidi row including trailing spaces"
        );
        assert_eq!(editor.editor.borrow().selection().anchor, 0);
        assert_eq!(editor.editor.borrow().selection().focus, text.len());
    }
}
