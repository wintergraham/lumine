//! Production GPU playback of the shared Rust scrollbar and measured Pen masters.
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
use lumine::{
    compose::{TaskRunner, prelude::*},
    frame::{Frame, FrameClock},
    input::{InputEvent, PointerButton},
    scene::*,
    semantics::Role,
    task::LocalExecutor,
    widgets::Ui,
};
use lumine_gpu::GpuRenderer;

#[test]
fn scrollbar_design_gpu_playback_and_pen_master_parity() {
    let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../artifacts/scrollbar");
    std::fs::create_dir_all(&output).unwrap();
    for width in [240., 400.] {
        for scale in [1., 1.5, 2.] {
            for horizontal in [false, true] {
                let (w, h) = if horizontal {
                    (width, 80.)
                } else {
                    (width, 240.)
                };
                let length = if horizontal { w } else { h };
                // Measured SUmOc/t9RkFW and F28tr/cbY0i from Components · Scrollbar.
                let (thumb_length, start) = if horizontal { (120., 40.) } else { (84., 28.) };
                let extent = (length - 4.) * length / thumb_length;
                let value = (start - 2.) / (length - 4. - thumb_length) * (extent - length);
                let mut gpu = GpuRenderer::new((w * scale) as u32, (h * scale) as u32).unwrap();
                gpu.set_scale_factor(scale);
                gpu.set_background(rgb(0x121211));
                let mut ui = Ui::new(w, h);
                let offset = ui.signal(0.);
                let executor = Rc::new(RefCell::new(LocalExecutor::new()));
                let frames = FrameClock::new();
                let scroller = if horizontal {
                    scroll_x(offset.clone())
                        .size(w, h)
                        .child(div().size(extent, h))
                } else {
                    scroll(offset.clone())
                        .size(w, h)
                        .child(div().size(w, extent))
                };
                let view = ui.mount(provide(
                    TaskRunner::from_executor(executor.clone()),
                    provide(frames.clone(), scroller),
                ));
                ui.prepare_frame();
                let track = ui
                    .semantics
                    .borrow()
                    .iter()
                    .find(|(_, n)| n.role == Role::ScrollBar)
                    .unwrap()
                    .0;
                let thumb = ui.scene.borrow().children(track)[0];
                // Keep the test clock ahead of readback/setup work between samples.
                let mut time = Instant::now() + Duration::from_secs(10);
                frames.deliver(Frame {
                    index: 0,
                    time,
                    interval: Duration::from_millis(16),
                });
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
                let mut capture = |ui: &mut Ui, stage: &str| {
                    let report = ui.scene.borrow_mut().flush();
                    gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
                    let pixels = gpu.readback().unwrap();
                    let axis = if horizontal { "horizontal" } else { "vertical" };
                    image::save_buffer(
                        output.join(format!("{axis}-{width}-{scale}-{stage}.png")),
                        &pixels,
                        (w * scale) as u32,
                        (h * scale) as u32,
                        image::ColorType::Rgba8,
                    )
                    .unwrap();
                    pixels
                };
                let rest = capture(&mut ui, "rest");
                offset.set(value);
                advance(&mut ui, 45);
                capture(&mut ui, "fade-in");
                advance(&mut ui, 55);
                let scrolling = capture(&mut ui, "scrolling");
                let b = ui.scene.borrow().bounds(thumb);
                let expected = if horizontal {
                    Rect::new(start, h - 7., thumb_length, 4.)
                } else {
                    Rect::new(w - 7., start, 4., thumb_length)
                };
                for (actual, expected) in [
                    (b.x, expected.x),
                    (b.y, expected.y),
                    (b.width, expected.width),
                    (b.height, expected.height),
                ] {
                    assert!((actual - expected).abs() < 0.001);
                }
                let mut reference = Scene::new(w, h);
                let pill = reference.append(
                    reference.root(),
                    NodeKind::Quad(QuadStyle {
                        fill: Color(255, 255, 255, 51),
                        radius: 2.,
                        ..Default::default()
                    }),
                    Style {
                        width: Some(expected.width),
                        height: Some(expected.height),
                        ..Default::default()
                    },
                );
                reference.set_transform(
                    pill,
                    Transform {
                        x: expected.x,
                        y: expected.y,
                    },
                );
                let mut reference_gpu =
                    GpuRenderer::new((w * scale) as u32, (h * scale) as u32).unwrap();
                reference_gpu.set_scale_factor(scale);
                reference_gpu.set_background(rgb(0x121211));
                let report = reference.flush();
                reference_gpu.render(&reference, &report.damage).unwrap();
                assert_eq!(
                    scrolling,
                    reference_gpu.readback().unwrap(),
                    "Pen master pixels"
                );
                advance(&mut ui, 690);
                assert_eq!(capture(&mut ui, "idle-hold"), scrolling);
                advance(&mut ui, 110);
                let fading = capture(&mut ui, "fade-out");
                assert_ne!(fading, scrolling);
                assert_ne!(fading, rest);
                advance(&mut ui, 110);
                assert_eq!(capture(&mut ui, "hidden"), rest);
                assert!(!frames.wants_frame());
                let (x, y) = if horizontal {
                    (start + 10., h - 6.)
                } else {
                    (w - 6., start + 10.)
                };
                ui.dispatch(InputEvent::PointerMove { x, y });
                advance(&mut ui, 70);
                capture(&mut ui, "hover-growing");
                advance(&mut ui, 80);
                let hover = capture(&mut ui, "hover");
                assert_ne!(hover, scrolling);
                ui.dispatch(InputEvent::PointerDown {
                    x,
                    y,
                    button: PointerButton::Primary,
                });
                advance(&mut ui, 1100);
                assert_eq!(ui.input.captured(), Some(track));
                let drag = capture(&mut ui, "drag-held");
                assert_ne!(drag, hover);
                ui.dispatch(InputEvent::PointerMove {
                    x: w + 100.,
                    y: h + 100.,
                });
                advance(&mut ui, 100);
                capture(&mut ui, "drag-endpoint");
                ui.dispatch(InputEvent::PointerUp {
                    x: w + 100.,
                    y: h + 100.,
                    button: PointerButton::Primary,
                });
                advance(&mut ui, 1100);
                assert_eq!(capture(&mut ui, "released"), rest);
                view.unmount();
                executor.borrow_mut().tick();
                assert!(!frames.wants_frame());
            }
        }
    }
}
