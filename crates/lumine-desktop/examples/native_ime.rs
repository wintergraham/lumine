//! Native input-method probe; the application never synthesizes IME events.
use std::time::Duration;
use lumine::{
    compose::prelude::*,
    input::{EventPhase, InputEvent},
    scene::{Color, NodeKind},
};
use lumine_desktop::{Application, WindowOptions};

fn observe_native_focus(tick: usize) {
    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;
        let mtm = MainThreadMarker::new().expect("IME observation runs on UI thread");
        let app = NSApplication::sharedApplication(mtm);
        println!("NATIVE_FOCUS {tick} active={}", app.isActive());
        for window in app.windows() {
            println!(
                "NATIVE_WINDOW {tick} title={:?} key={}",
                window.title().to_string(),
                window.isKeyWindow()
            );
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = tick;
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let observe = std::env::args().any(|arg| arg == "--native-observe");
    Application::new()
        .window(WindowOptions {
            title: "lumine native IME".into(),
            width: 640.,
            height: 340.,
            ..Default::default()
        })
        .run(move |cx| {
            let first = cx.ui.signal(String::new());
            let second = cx.ui.signal(String::new());
            let editor = |label: &'static str, model| {
                text_input(label, model)
                    .id(label)
                    .size(560., 44.)
                    .p(10.)
                    .text_size(22.)
                    .bg(Color(38, 51, 72, 255))
                    .focus(|s| s.border(2.).border_color(Color(121, 184, 255, 255)))
                    .on_event(move |event| {
                        if event.phase == EventPhase::Target {
                            match &event.event {
                                InputEvent::ImePreedit { text, cursor } => {
                                    println!("PREEDIT {label} {text:?} {cursor:?}")
                                }
                                InputEvent::ImeCommit(text) => println!("COMMIT {label} {text:?}"),
                                InputEvent::Focus => println!("FOCUS {label}"),
                                InputEvent::Blur => println!("BLUR {label}"),
                                _ => {}
                            }
                        }
                    })
            };
            let handle = cx.render(
                column()
                    .p(24.)
                    .gap(12.)
                    .text_color(Color(231, 236, 246, 255))
                    .child(text("Native input-method composition").h(24.))
                    .child(editor("first", first.clone()))
                    .child(text("Second editor").h(24.))
                    .child(editor("second", second.clone())),
            );
            let scene = cx.ui.scene.clone();
            let input = cx.ui.input.clone();
            let first_node = handle.find("first").unwrap();
            let second_node = handle.find("second").unwrap();
            let close_first = first.clone();
            let close_second = second.clone();
            cx.on_closed(move || {
                println!(
                    "FINAL first={:?} second={:?}",
                    close_first.get(),
                    close_second.get()
                )
            });
            let window = cx.window.clone();
            cx.tasks.spawn(async move {
                let ticks = std::env::var("LUMINE_SECONDS")
                    .ok()
                    .and_then(|s| s.parse::<usize>().ok())
                    .map_or(40, |s| s * 2);
                for tick in 0..ticks {
                    lumine::timer::sleep(Duration::from_millis(500)).await;
                    if observe {
                        observe_native_focus(tick);
                    }
                    println!(
                        "MODEL {tick} first={:?} second={:?} focus={:?}",
                        first.get(),
                        second.get(),
                        input.focused()
                    );
                    let scene = scene.borrow();
                    for (label, node) in [("first", first_node), ("second", second_node)] {
                        let mut nodes = vec![node];
                        while let Some(node) = nodes.pop() {
                            nodes.extend(scene.children(node));
                            let rect = scene.bounds(node);
                            match scene.kind(node) {
                                NodeKind::Text { text, .. } => {
                                    println!("DISPLAY {label} {text:?} {rect:?}")
                                }
                                NodeKind::Rect(_) if rect.width <= 2. && rect.height > 5. => {
                                    println!("CARET {label} {rect:?}")
                                }
                                _ => {}
                            }
                        }
                    }
                }
                window.close();
            });
        })
}
