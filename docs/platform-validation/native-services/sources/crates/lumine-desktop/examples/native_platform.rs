//! Native application activation and retained window geometry controls.
use std::time::Duration;
use lumine::compose::prelude::*;
use lumine_desktop::{Application, ApplicationEvent, DisplaySelector, WindowBounds, WindowOptions};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let smoke = std::env::args().any(|a| a == "--smoke");
    Application::new()
        .application_id("org.lumine.NativePlatform")?
        .quit_on_last_window_close(false)
        .on_application_event(|event, windows| {
            println!("APPLICATION {event:?}");
            match event {
                ApplicationEvent::OpenUrls(urls)
                    if urls.iter().any(|url| url == "lumine-test:quit") =>
                {
                    windows.quit()
                }
                ApplicationEvent::Reopen => {
                    windows.open(
                        WindowOptions {
                            title: "lumine reopened".into(),
                            width: 320.,
                            height: 240.,
                            ..Default::default()
                        },
                        |cx| {
                            cx.render(column().p(20.).child("Reopened by the operating system"));
                            println!("REOPENED");
                        },
                    );
                }
                _ => {}
            }
        })
        .window(WindowOptions {
            title: "lumine native platform".into(),
            bounds: Some(WindowBounds::new(420, 320).at(100, 120)),
            min_size: Some((240., 180.)),
            display: Some(DisplaySelector::Primary),
            decorations: false,
            ..Default::default()
        })
        .run(move |cx| {
            let drag = cx.window.clone();
            let close = cx.window.clone();
            cx.render(
                column()
                    .gap(10.)
                    .child(
                        button()
                            .w(420.)
                            .h(36.)
                            .child("Drag this custom titlebar")
                            .on_event(move |event| {
                                if matches!(
                                    event.event,
                                    lumine::input::InputEvent::PointerDown {
                                        button: lumine::input::PointerButton::Primary,
                                        ..
                                    }
                                ) {
                                    drag.drag_window();
                                }
                            }),
                    )
                    .child(
                        button()
                            .h(36.)
                            .child("Close window; retain application")
                            .on_click(move || close.close()),
                    ),
            );
            cx.on_closed(|| println!("ROOT_CLOSED"));
            if smoke {
                let window = cx.window.clone();
                let info = cx.window_info.clone();
                cx.tasks.spawn(async move {
                    lumine::timer::sleep(Duration::from_millis(600)).await;
                    println!(
                        "INITIAL {:?} {:?} displays={:?}",
                        info.bounds.get(),
                        info.capabilities.get(),
                        info.displays.get()
                    );
                    window.set_fullscreen(true);
                    lumine::timer::sleep(Duration::from_millis(600)).await;
                    println!(
                        "FULLSCREEN {} {:?}",
                        info.fullscreen.get(),
                        info.bounds.get()
                    );
                    window.set_fullscreen(false);
                    lumine::timer::sleep(Duration::from_millis(600)).await;
                    println!("RESTORED {} {:?}", info.fullscreen.get(), info.bounds.get());
                    window.set_min_inner_size(Some((300., 220.)));
                    window.set_bounds(WindowBounds::new(100, 80).at(220, 180));
                    lumine::timer::sleep(Duration::from_millis(600)).await;
                    println!(
                        "MINIMUM {:?} error={:?}",
                        info.bounds.get(),
                        info.last_request_error.get()
                    );
                    println!("CONTROLS_READY");
                    lumine::timer::sleep(Duration::from_secs(2)).await;
                    window.refresh_window_info();
                    lumine::timer::sleep(Duration::from_millis(100)).await;
                    println!(
                        "AFTER_DRAG {:?} error={:?}",
                        info.bounds.get(),
                        info.last_request_error.get()
                    );
                    window.close();
                });
            }
        })
}
