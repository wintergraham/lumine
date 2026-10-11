use std::{cell::RefCell, rc::Rc};
use lumine::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers, PointerButton},
    widgets::Ui,
};
#[derive(Clone, Debug, PartialEq)]
struct Card(u32);
fn pointer(ui: &Ui, event: InputEvent) {
    ui.input.dispatch(&ui.scene, event);
    ui.prepare_frame();
}
fn down(ui: &Ui, x: f32, y: f32) {
    pointer(
        ui,
        InputEvent::PointerDown {
            x,
            y,
            button: PointerButton::Primary,
        },
    );
}
fn moved(ui: &Ui, x: f32, y: f32) {
    pointer(ui, InputEvent::PointerMove { x, y });
}
fn up(ui: &Ui, x: f32, y: f32) {
    pointer(
        ui,
        InputEvent::PointerUp {
            x,
            y,
            button: PointerButton::Primary,
        },
    );
}
#[test]
fn typed_drop_preview_and_click_suppression() {
    let mut ui = Ui::new(400., 200.);
    let log = Rc::new(RefCell::new(Vec::new()));
    let dropped = log.clone();
    let ended = log.clone();
    let clicked = log.clone();
    let view = ui.mount(
        row()
            .gap(20.)
            .child(
                button()
                    .child(text("Card"))
                    .id("source")
                    .size(100., 80.)
                    .on_click(move || clicked.borrow_mut().push("click".to_owned()))
                    .on_drag(|| Card(7))
                    .drag_preview(|value: &Card| {
                        div()
                            .id("preview")
                            .size(100., 80.)
                            .child(text(format!("{}", value.0)))
                    })
                    .on_drag_end(move |_: &Card, accepted| {
                        ended.borrow_mut().push(format!("end:{accepted}"))
                    }),
            )
            .child(
                div()
                    .id("destination")
                    .size(100., 80.)
                    .on_drop(move |card: &Card, _| {
                        dropped.borrow_mut().push(format!("drop:{}", card.0))
                    }),
            ),
    );
    ui.prepare_frame();
    let original = ui.scene.borrow().children(ui.root()).len();
    down(&ui, 20., 20.);
    moved(&ui, 22., 20.);
    assert!(!ui.input.is_dragging());
    up(&ui, 22., 20.);
    assert_eq!(&*log.borrow(), &["click"]);
    log.borrow_mut().clear();
    down(&ui, 20., 20.);
    moved(&ui, 35., 20.);
    assert!(ui.input.is_dragging());
    assert_eq!(ui.scene.borrow().children(ui.root()).len(), original + 1);
    moved(&ui, 150., 20.);
    up(&ui, 150., 20.);
    assert_eq!(&*log.borrow(), &["drop:7", "end:true"]);
    assert!(!ui.input.is_dragging());
    assert_eq!(ui.scene.borrow().children(ui.root()).len(), original);
    assert!(view.is_mounted());
}
#[test]
fn rejected_outside_and_escape_drags_end_without_drop() {
    let mut ui = Ui::new(400., 200.);
    let ends = Rc::new(RefCell::new(Vec::new()));
    let log = ends.clone();
    let view = ui.mount(
        row()
            .gap(20.)
            .child(
                div()
                    .id("source")
                    .size(100., 80.)
                    .on_drag(|| Card(7))
                    .on_drag_end(move |_: &Card, accepted| log.borrow_mut().push(accepted)),
            )
            .child(
                div()
                    .size(100., 80.)
                    .on_drop_when(|_: &Card| false, |_: &Card, _| panic!("rejected drop")),
            ),
    );
    ui.prepare_frame();
    down(&ui, 20., 20.);
    moved(&ui, 150., 20.);
    up(&ui, 150., 20.);
    down(&ui, 20., 20.);
    moved(&ui, 300., 150.);
    up(&ui, 300., 150.);
    down(&ui, 20., 20.);
    moved(&ui, 35., 20.);
    pointer(
        &ui,
        InputEvent::KeyDown {
            key: Key::Escape,
            modifiers: Modifiers::default(),
            repeat: false,
        },
    );
    assert_eq!(&*ends.borrow(), &[false, false, false]);
    assert!(!ui.input.is_dragging());
    view.unmount();
}
#[test]
fn removing_source_disposes_preview_and_payload() {
    let mut ui = Ui::new(400., 200.);
    let value = Rc::new(());
    let weak = Rc::downgrade(&value);
    let source = ui.mount(
        div()
            .size(100., 80.)
            .on_drag(move || value.clone())
            .drag_preview(|_: &Rc<()>| div().size(40., 40.)),
    );
    ui.prepare_frame();
    down(&ui, 20., 20.);
    moved(&ui, 35., 20.);
    assert!(ui.input.is_dragging());
    source.unmount();
    ui.prepare_frame();
    assert!(!ui.input.is_dragging());
    assert!(weak.upgrade().is_none());
    assert!(ui.scene.borrow().children(ui.root()).is_empty());
}
#[test]
fn nearest_typed_destination_wins_and_removed_target_cannot_receive_drop() {
    let mut ui = Ui::new(400., 200.);
    let log = Rc::new(RefCell::new(Vec::new()));
    let parent = log.clone();
    let child = log.clone();
    let source = ui.mount(div().size(80., 80.).on_drag(|| Card(1)));
    let target = ui.mount(
        div()
            .absolute()
            .left(120.)
            .size(160., 100.)
            .on_drop(move |_: &Card, _| parent.borrow_mut().push("parent"))
            .child(
                div()
                    .id("inner")
                    .size(80., 80.)
                    .on_drop(move |_: &Card, _| child.borrow_mut().push("child")),
            ),
    );
    ui.prepare_frame();
    down(&ui, 20., 20.);
    moved(&ui, 140., 20.);
    up(&ui, 140., 20.);
    assert_eq!(&*log.borrow(), &["child"]);
    down(&ui, 20., 20.);
    moved(&ui, 140., 20.);
    target.unmount();
    up(&ui, 140., 20.);
    assert_eq!(&*log.borrow(), &["child"]);
    assert!(!ui.input.is_dragging());
    source.unmount();
}
#[test]
fn hidden_source_cancels_and_releases_preview() {
    let mut ui = Ui::new(400., 200.);
    let source = ui.mount(
        div()
            .size(100., 80.)
            .on_drag(|| Card(1))
            .drag_preview(|_: &Card| div().size(20., 20.)),
    );
    ui.prepare_frame();
    down(&ui, 20., 20.);
    moved(&ui, 35., 20.);
    let mut style = ui.scene.borrow().style(source.node());
    style.layout_options = Some(std::sync::Arc::new(lumine::layout::LayoutOptions {
        visible: Some(false),
        ..Default::default()
    }));
    ui.scene.borrow_mut().set_style(source.node(), style);
    ui.prepare_frame();
    assert!(!ui.input.is_dragging());
    assert_eq!(ui.scene.borrow().children(ui.root()).len(), 1);
}
#[test]
fn grouped_external_paths_route_once_to_nearest_handler() {
    let mut ui = Ui::new(300., 200.);
    let received = Rc::new(RefCell::new(Vec::new()));
    let output = received.clone();
    ui.mount(
        div()
            .size(200., 100.)
            .on_files_drop(move |paths, _| output.borrow_mut().push(paths.to_vec())),
    );
    ui.prepare_frame();
    pointer(
        &ui,
        InputEvent::FilesDrop {
            x: 20.,
            y: 20.,
            paths: vec!["/tmp/a".into(), "/tmp/b".into()].into(),
        },
    );
    assert_eq!(
        &*received.borrow(),
        &[vec![
            std::path::PathBuf::from("/tmp/a"),
            std::path::PathBuf::from("/tmp/b")
        ]]
    );
}
#[test]
fn nearest_nested_source_wins_capture() {
    let mut ui = Ui::new(400., 200.);
    let got = Rc::new(RefCell::new(Vec::new()));
    let result = got.clone();
    ui.mount(
        row()
            .gap(20.)
            .child(
                div()
                    .size(100., 80.)
                    .on_drag(|| Card(99))
                    .child(div().size(80., 60.).on_drag(|| Card(1))),
            )
            .child(
                div()
                    .size(100., 80.)
                    .on_drop(move |card: &Card, _| result.borrow_mut().push(card.0)),
            ),
    );
    ui.prepare_frame();
    down(&ui, 20., 20.);
    moved(&ui, 150., 20.);
    up(&ui, 150., 20.);
    assert_eq!(&*got.borrow(), &[1]);
}
#[test]
fn disabled_source_cancellation_releases_preview_and_notifies_end() {
    let mut ui = Ui::new(400., 200.);
    let ends = Rc::new(RefCell::new(Vec::new()));
    let results = ends.clone();
    let source = ui.mount(
        div()
            .size(100., 80.)
            .on_drag(|| Card(1))
            .drag_preview(|_: &Card| div().size(20., 20.))
            .on_drag_end(move |_: &Card, accepted| results.borrow_mut().push(accepted)),
    );
    ui.prepare_frame();
    down(&ui, 20., 20.);
    moved(&ui, 35., 20.);
    ui.set_disabled(source.node(), true);
    ui.prepare_frame();
    assert!(!ui.input.is_dragging());
    assert_eq!(&*ends.borrow(), &[false]);
    assert_eq!(ui.scene.borrow().children(ui.root()).len(), 1);
}
#[test]
fn preview_keeps_grab_point_or_fixed_cursor_offset() {
    for cursor_offset in [None, Some((12., 12.))] {
        let mut ui = Ui::new(400., 300.);
        let source = div().id("source").size(100., 80.).on_drag(|| Card(1));
        let preview = |_: &Card| div().size(100., 80.);
        let source = match cursor_offset {
            None => source.drag_preview(preview),
            Some(offset) => source.drag_preview_at_cursor(offset, preview),
        };
        let view = ui.mount(row().p(30.).child(source));
        ui.prepare_frame();
        let original = ui.scene.borrow().children(ui.root()).len();
        // Press 20,30 inside the source at 30,30, then drag.
        down(&ui, 50., 60.);
        moved(&ui, 100., 90.);
        assert!(ui.input.is_dragging());
        let scene = ui.scene.borrow();
        let children = scene.children(ui.root());
        assert_eq!(children.len(), original + 1);
        let bounds = scene.bounds(children[original]);
        let expected = match cursor_offset {
            None => (80., 60.),
            Some(_) => (112., 102.),
        };
        assert_eq!((bounds.x, bounds.y), expected);
        drop(scene);
        up(&ui, 100., 90.);
        view.unmount();
    }
}
