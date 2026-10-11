//! Frosted-glass counter with a custom, comet-style inset titlebar.
//! Run: cargo run --release -p lumine-desktop --example glass_counter
//!
//! macOS: the window keeps its native traffic lights over a transparent,
//! full-size-content titlebar, and the desktop behind it is blurred. Other
//! platforms fall back to a frameless translucent window with drawn controls.
use lumine::{
    compose::prelude::*,
    input::{EventPhase, InputEvent, Key, PointerButton},
    text_layout::{FontFeatures, TextAlign},
};
use lumine_desktop::{Application, WindowOptions};

const TITLE: &str = "lumine glass counter";
const WIDTH: f64 = 340.;
const HEIGHT: f64 = 420.;
const TITLEBAR: f32 = 40.;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .window(WindowOptions {
            title: TITLE.into(),
            width: WIDTH,
            height: HEIGHT,
            transparent: true,
            decorations: cfg!(target_os = "macos"),
            min_size: Some((280., 340.)),
            ..Default::default()
        })
        .run(move |cx| {
            #[cfg(target_os = "macos")]
            {
                // The native window is realised after this closure returns.
                let window = cx.window.clone();
                cx.tasks.spawn(async move {
                    lumine::timer::sleep(std::time::Duration::from_millis(1)).await;
                    if glass::inset_titlebar(TITLE) {
                        // Content now spans the titlebar; resize so the host observes it.
                        window.set_inner_size(WIDTH, HEIGHT);
                    }
                });
            }
            let window = cx.window.clone();
            let view = cx.render(component(move |cx| {
                let count = cx.state(0_i64);
                let (inc, dec, reset, keys, label, tint) = (
                    count.clone(),
                    count.clone(),
                    count.clone(),
                    count.clone(),
                    count.clone(),
                    count.clone(),
                );
                let drag = window.clone();
                // Untitled drag strip under the traffic lights.
                let titlebar = div().w_full().h(TITLEBAR).on_event(move |event| {
                    if matches!(
                        event.event,
                        InputEvent::PointerDown {
                            button: PointerButton::Primary,
                            ..
                        }
                    ) {
                        drag.drag_window();
                    }
                });
                let number = text_signal(move || label.get().to_string())
                    .text_size(104.)
                    .font_weight(200)
                    .letter_spacing(-3.)
                    .font_features(FontFeatures::new([(*b"tnum", 1)]))
                    .text_align(TextAlign::Center)
                    .reactive_style(move || {
                        Styles::new().text_color(match tint.get() {
                            n if n > 0 => rgb(0xb4f5d8),
                            n if n < 0 => rgb(0xffc2b8),
                            _ => rgb(0xffffff),
                        })
                    });
                let stepper = row()
                    .p(5.)
                    .gap(4.)
                    .items_center()
                    .rounded(30.)
                    .bg(rgba(0xffffff12))
                    .border(1.)
                    .border_color(rgba(0xffffff22))
                    .child(step_button(false).on_click(move || {
                        dec.update(|n| *n -= 1);
                    }))
                    .child(div().w(1.).h(22.).bg(rgba(0xffffff26)))
                    .child(step_button(true).on_click(move || {
                        inc.update(|n| *n += 1);
                    }));
                column()
                    .w_full()
                    .h_full()
                    .bg(rgba(0x0c101860))
                    .rounded(if cfg!(target_os = "macos") { 0. } else { 14. })
                    .focusable(true)
                    .on_event(move |event| {
                        if event.phase == EventPhase::Bubble {
                            return;
                        }
                        let InputEvent::KeyDown { key, .. } = &event.event else {
                            return;
                        };
                        match key {
                            Key::Character(c) if c == "+" || c == "=" => {
                                keys.update(|n| *n += 1);
                            }
                            Key::Character(c) if c == "-" => {
                                keys.update(|n| *n -= 1);
                            }
                            Key::Character(c) if c == "0" => {
                                keys.set(0);
                            }
                            _ => return,
                        }
                        event.prevent_default();
                    })
                    .child(titlebar)
                    .child(
                        column()
                            .w_full()
                            .grow()
                            .items_center()
                            .justify_center()
                            .gap(6.)
                            .pb(TITLEBAR)
                            .child(
                                text("COUNT")
                                    .text_size(11.)
                                    .font_weight(600)
                                    .letter_spacing(2.4)
                                    .text_color(rgba(0xffffff70)),
                            )
                            .child(row().w_full().justify_center().child(number))
                            .child(div().h(22.))
                            .child(stepper)
                            .child(div().h(14.))
                            .child(
                                button()
                                    .px(14.)
                                    .py(6.)
                                    .rounded(999.)
                                    .bg(rgba(0xffffff00))
                                    .text_size(12.)
                                    .font_weight(500)
                                    .text_color(rgba(0xffffff80))
                                    .hover(|s| s.bg(rgba(0xffffff12)).text_color(rgb(0xffffff)))
                                    .active(|s| s.bg(rgba(0xffffff20)))
                                    .focus(|s| s)
                                    .child("Reset")
                                    .on_click(move || {
                                        reset.set(0);
                                    }),
                            ),
                    )
            }));
            cx.ui.input.focus(&cx.ui.scene, Some(view.node()));
        })
}

fn step_button(plus: bool) -> View {
    // Drawn glyphs: the font's minus sits on the baseline, not the plus's centre.
    let bar = |w: f32, h: f32| div().absolute().size(w, h).rounded(1.).bg(rgb(0xffffff));
    let mut icon = overlay().size(16., 16.).child(bar(16., 2.).mt(7.));
    if plus {
        icon = icon.child(bar(2., 16.).ml(7.));
    }
    button()
        .size(76., 48.)
        .rounded(24.)
        .items_center()
        .justify_center()
        .bg(rgba(0xffffff00))
        .hover(|s| s.bg(rgba(0xffffff18)))
        .active(|s| s.bg(rgba(0xffffff2c)))
        // lumine focuses buttons on click and has no focus-visible; no ring.
        .focus(|s| s)
        .child(icon)
}

#[cfg(target_os = "macos")]
#[path = "support/glass.rs"]
mod glass;
