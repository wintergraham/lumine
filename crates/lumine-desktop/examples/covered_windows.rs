//! Two independent updating surfaces; a covered window must not stall the UI loop.
use std::time::Duration;
use lumine::compose::prelude::*;
use lumine_desktop::{Application, WindowOptions};
fn content(cx: &mut lumine_desktop::WindowContext, name: &'static str) {
    let tick = cx.ui.signal(0u32);
    let read = tick.clone();
    cx.render(
        column()
            .p(20.)
            .child(text_signal(move || format!("{name}: {}", read.get()))),
    );
    cx.tasks.spawn(async move {
        for i in 1..=24 {
            lumine::timer::sleep(Duration::from_millis(250)).await;
            tick.set(i);
            println!("TICK {name} {i}");
        }
    });
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .window(WindowOptions {
            title: "lumine covered window".into(),
            width: 360.,
            height: 220.,
            ..Default::default()
        })
        .run(|cx| {
            content(cx, "covered");
            cx.windows.open(
                WindowOptions {
                    title: "lumine covering window".into(),
                    width: 600.,
                    height: 400.,
                    ..Default::default()
                },
                |cx| content(cx, "covering"),
            );
            let windows = cx.windows.clone();
            cx.tasks.spawn(async move {
                lumine::timer::sleep(Duration::from_secs(8)).await;
                println!("COMPLETE");
                windows.quit();
            });
            println!("READY");
        })
}
