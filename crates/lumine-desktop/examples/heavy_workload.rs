//! The heavy dashboard comparison scene (see lumine_workload::heavy), shared
//! with the GPUI and QuickGUI adapters: every live value is a reactive text
//! or style reading the `live` tick, and the table is a virtual list.
//! Run with LUMINE_MODE=both LUMINE_SECONDS=10.
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
use lumine::{compose::prelude::*, reactive::Signal};
use lumine_desktop::{Application, WindowOptions};
use lumine_workload::{Mode, heavy::*};

fn at(view: View, (x, y): (f32, f32), width: f32, height: f32) -> View {
    view.absolute().ml(x).mt(y).size(width, height)
}
fn panel(view: View, radius: f32) -> View {
    view.rounded(radius)
        .border(1.)
        .border_color(rgb(BORDER))
        .bg(rgb(PANEL))
}

fn tile(index: usize, live: Signal<u64>) -> View {
    panel(at(overlay(), tile_position(index), TILE.0, TILE.1), 8.)
        .child(at(
            text(tile_label(index))
                .text_size(11.)
                .line_height(14.)
                .text_color(rgb(MUTED)),
            TILE_LABEL,
            176.,
            14.,
        ))
        .child(at(
            text_signal(move || tile_value(index, live.get()))
                .text_size(20.)
                .line_height(26.),
            TILE_VALUE,
            176.,
            26.,
        ))
}

fn card(index: usize, live: Signal<u64>) -> View {
    let value = {
        let live = live.clone();
        text_signal(move || card_value(index, live.get()))
    };
    let fill = {
        let live = live.clone();
        at(div(), (PROGRESS.0, PROGRESS.1), 0., PROGRESS.3)
            .rounded(2.)
            .bg(rgb(ACCENT))
            .reactive_style(move || Styles::new().w(card_progress(index, live.get())))
    };
    let bars = (0..BARS).map(|bar| {
        let live = live.clone();
        div()
            .absolute()
            .ml(bar_x(bar))
            .w(BAR_WIDTH)
            .bg(rgb(ACCENT))
            .reactive_style(move || {
                let height = bar_height(index, bar, live.get());
                Styles::new().mt(SPARK_BASE - height).h(height)
            })
    });
    panel(at(overlay(), card_position(index), CARD.0, CARD.1), 6.)
        .child(at(
            text(card_title(index))
                .text_size(11.)
                .line_height(14.)
                .text_color(rgb(MUTED)),
            CARD_TITLE,
            104.,
            14.,
        ))
        .child(at(
            value.text_size(16.).line_height(20.),
            CARD_VALUE,
            104.,
            20.,
        ))
        .child(
            at(div(), (PROGRESS.0, PROGRESS.1), PROGRESS.2, PROGRESS.3)
                .rounded(2.)
                .bg(rgb(TRACK)),
        )
        .child(fill)
        .children(bars)
}

fn cell(value: View, column: usize, width: f32) -> View {
    at(
        value.text_size(12.).line_height(16.),
        (COLUMNS[column].0, CELL_TOP),
        width,
        16.,
    )
}

fn table_row(row: usize, live: Signal<u64>) -> View {
    let pill = {
        let (live, label) = (live.clone(), live.clone());
        at(overlay(), (PILL.0, PILL.1), PILL.2, PILL.3)
            .rounded(8.)
            .reactive_style(move || Styles::new().bg(rgb(status(row, live.get()).1)))
            .child(at(
                text_signal(move || status(row, label.get()).0.to_owned())
                    .text_size(10.)
                    .line_height(14.),
                PILL_TEXT,
                56.,
                14.,
            ))
    };
    let latency = {
        let live = live.clone();
        text_signal(move || row_latency(row, live.get()))
    };
    overlay()
        .w(TABLE.0)
        .h(TABLE_ROW)
        .bg(rgb(row_color(row)))
        .child(cell(text(row_id(row)).text_color(rgb(MUTED)), 0, 70.))
        .child(cell(text(row_worker(row)), 1, 150.))
        .child(pill)
        .child(cell(latency, 3, 70.))
        .child(cell(
            text_signal(move || row_throughput(row, live.get())),
            4,
            80.,
        ))
}

fn table(live: Signal<u64>, scroll: Signal<f32>) -> View {
    let header = COLUMNS.iter().enumerate().map(|(column, (_, title))| {
        at(
            text(*title)
                .text_size(11.)
                .line_height(14.)
                .text_color(rgb(MUTED)),
            (COLUMNS[column].0, 7.),
            80.,
            14.,
        )
    });
    let rows = virtual_list(
        scroll,
        TABLE_ROW,
        TABLE_OVERSCAN,
        || TABLE_ROWS,
        |row| row,
        move |_, row, _| table_row(row, live.clone()),
    );
    panel(at(overlay(), TABLE_ORIGIN, TABLE.0, TABLE.1), 8.)
        .overflow_hidden()
        .child(
            at(overlay(), (0., 0.), TABLE.0, TABLE_HEADER)
                .bg(rgb(HEADER))
                .children(header),
        )
        .child(at(rows, (0., TABLE_HEADER), TABLE.0, TABLE_VIEWPORT))
}

fn dashboard(live: Signal<u64>, scroll: Signal<f32>, log: Signal<String>) -> View {
    overlay()
        .w(WIDTH)
        .h(HEIGHT)
        .bg(rgb(BACKGROUND))
        .text_color(rgb(TEXT))
        .font_family(FONT)
        .child(at(
            text(TITLE).text_size(18.).line_height(24.),
            TITLE_ORIGIN,
            600.,
            24.,
        ))
        .children((0..TILES).map(|index| tile(index, live.clone())))
        .children((0..CARDS).map(|index| card(index, live.clone())))
        .child(table(live, scroll))
        .child(
            panel(at(overlay(), LOG_ORIGIN, LOG.0, LOG.1), 8.).child(at(
                text_signal(move || log.get())
                    .text_size(11.)
                    .line_height(LOG_LINE_HEIGHT)
                    .text_color(rgb(MUTED)),
                LOG_TEXT,
                LOG.0 - 24.,
                LOG_LINES as f32 * LOG_LINE_HEIGHT,
            )),
        )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .window(WindowOptions {
            title: "lumine heavy".into(),
            width: WIDTH as f64,
            height: HEIGHT as f64,
            resizable: false,
            ..Default::default()
        })
        .run(|cx| {
            let work = Rc::new(RefCell::new(Heavy::from_env()));
            let live = cx.ui.signal(0_u64);
            let scroll = cx.ui.signal(0_f32);
            let log = cx.ui.signal(work.borrow().visible_log());
            cx.render(dashboard(live.clone(), scroll.clone(), log.clone()));
            cx.on_closed({
                let work = work.clone();
                move || {
                    println!(
                        "{{\"framework\":\"lumine\",\"scene\":\"heavy\",\"ticks\":{}}}",
                        work.borrow().frames
                    )
                }
            });
            let runtime = cx.ui.runtime.clone();
            let window = cx.window.clone();
            cx.tasks.spawn(async move {
                let start = Instant::now();
                let (mode, seconds) = {
                    let work = work.borrow();
                    (work.mode, work.seconds)
                };
                let end = (seconds > 0.).then(|| start + Duration::from_secs_f64(seconds));
                if mode == Mode::Idle {
                    if let Some(end) = end {
                        lumine::timer::sleep_until(end).await;
                        window.close();
                    }
                    return;
                }
                let period = Heavy::period();
                let mut next = start + period;
                loop {
                    lumine::timer::sleep_until(end.map_or(next, |end| end.min(next))).await;
                    if end.is_some_and(|end| Instant::now() >= end) {
                        window.close();
                        break;
                    }
                    runtime.batch(|| {
                        let mut work = work.borrow_mut();
                        work.tick();
                        if matches!(mode, Mode::Stream | Mode::Both) {
                            live.set(work.live);
                            log.set(work.visible_log());
                        }
                        if matches!(mode, Mode::Scroll | Mode::Both) {
                            scroll.set(work.scroll);
                        }
                    });
                    next += period;
                    let now = Instant::now();
                    if next <= now {
                        next = now + period;
                    }
                }
            });
        })
}
