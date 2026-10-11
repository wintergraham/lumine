//! Display-rate animation workload shared with the GPUI and QuickGUI
//! comparison adapters (see lumine_workload::animation). Retained nodes whose
//! reactive styles read a clock advanced once per display refresh.
use std::time::{Duration, Instant};
use lumine::{compose::prelude::*, scene::Color, timer::sleep};
use lumine_desktop::{Application, WindowOptions};
use lumine_workload::{Workload, animation::*};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let seconds = Workload::from_env().seconds;
    Application::new()
        .window(WindowOptions {
            title: "lumine animation".into(),
            width: 960.,
            height: 720.,
            ..Default::default()
        })
        .run(move |cx| {
            let clock = cx.ui.signal(0_f32);
            let frames = cx.ui.signal(0_u64);
            let start = Instant::now();
            {
                let (clock, frames, source) = (clock.clone(), frames.clone(), cx.frames.clone());
                cx.tasks.spawn(async move {
                    loop {
                        source.next().await;
                        clock.set(start.elapsed().as_secs_f32());
                        frames.update(|n| *n += 1);
                    }
                });
            }
            if seconds > 0. {
                let (window, frames) = (cx.window.clone(), frames.clone());
                cx.tasks.spawn(async move {
                    sleep(Duration::from_secs_f64(seconds)).await;
                    let n = frames.get();
                    eprintln!(
                        "animation_frames={n} fps={:.1}",
                        n as f64 / start.elapsed().as_secs_f64()
                    );
                    window.close();
                });
            }
            let dots = (0..COLUMNS * ROWS).map(|i| {
                let (x, y) = dot_position(i);
                let clock = clock.clone();
                div()
                    .absolute()
                    .ml(x)
                    .mt(y)
                    .size(DOT, DOT)
                    .rounded(DOT / 2.)
                    .bg(rgb(DOT_COLOR))
                    .reactive_style(move || Styles::new().opacity(dot_opacity(i, clock.get())))
            });
            let chars: Vec<char> = SHIMMER.chars().collect();
            let count = chars.len();
            let line = row()
                .absolute()
                .ml(SHIMMER_ORIGIN.0)
                .mt(SHIMMER_ORIGIN.1)
                .children(chars.into_iter().enumerate().map(|(i, ch)| {
                    let clock = clock.clone();
                    text(ch.to_string()).reactive_style(move || {
                        Styles::new().text_color(Color(
                            255,
                            255,
                            255,
                            shimmer_alpha(i, count, clock.get()),
                        ))
                    })
                }));
            cx.render(
                overlay()
                    .w_full()
                    .h_full()
                    .bg(rgb(BACKGROUND))
                    .text_size(18.)
                    .children(dots)
                    .child(line),
            );
        })
}
