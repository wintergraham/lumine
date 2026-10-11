//! A lumine client for a running Zeron: browse sessions, read live transcripts,
//! send messages, stop a turn and switch the session's model and reasoning.
//! It speaks the engine's local IPC (see `support/zeron_rpc.rs`), the same
//! protocol the Zeron window uses, so both stay in sync.
//! Run: cargo run --release -p lumine-desktop --example zeron
//! `ZERON_IPC_PORT` selects another engine (e.g. a sandboxed `zeron headless`);
//! `ZERON_SEND=<text>` and `ZERON_MODEL=<id>` drive the composer for tests;
//! `ZERON_OPEN=<title>` preselects a chat.
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    f32::consts::FRAC_PI_2,
    rc::Rc,
    sync::Arc,
    time::Duration,
};
use lumine::{
    affine::Affine,
    compose::prelude::*,
    cursor::Cursor,
    input::{EventPhase, InputEvent, Key},
    reactive::Signal,
    scene::Color,
    timer::sleep,
};
use lumine_desktop::{Application, WindowOptions};

#[cfg(target_os = "macos")]
#[path = "support/glass.rs"]
mod glass;
#[path = "support/zeron_markdown.rs"]
mod markdown;
#[path = "support/zeron_model.rs"]
mod model;
#[path = "support/zeron_rpc.rs"]
mod rpc;
#[path = "support/zeron_view.rs"]
mod view;

use model::{ChatRow, Status, Transcript};
use rpc::Rpc;
use view::{ACCENT, Anim, DANGER, FAINT, MUTED, TEXT, icon, icon_data, white};

const TITLE: &str = "Zeron · lumine";
const WIDTH: f64 = 1320.;
const HEIGHT: f64 = 880.;
/// The transcript column's text width (832 less 48 px gutters), for
/// estimating the heights of rows not yet mounted.
const COLUMN: f32 = 736.;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .window(WindowOptions {
            title: TITLE.into(),
            width: WIDTH,
            height: HEIGHT,
            transparent: true,
            decorations: cfg!(target_os = "macos"),
            min_size: Some((900., 580.)),
            ..Default::default()
        })
        .run(move |cx| {
            #[cfg(target_os = "macos")]
            {
                let window = cx.window.clone();
                cx.tasks.spawn(async move {
                    sleep(Duration::from_millis(1)).await;
                    if glass::inset_titlebar(TITLE) {
                        window.set_inner_size(WIDTH, HEIGHT);
                    }
                });
            }
            cx.ui.theme.hover = white(0);
            cx.ui.theme.accent = white(52);
            // One connection at a time; the UI below remounts on each one.
            let generation = cx.ui.signal(0_u32);
            let offline = cx.ui.signal(Some("Connecting to Zeron…".to_owned()));
            let current: Rc<RefCell<Option<Rpc>>> = Rc::default();
            {
                let (generation, offline, current) =
                    (generation.clone(), offline.clone(), current.clone());
                cx.tasks.spawn(async move {
                    loop {
                        match rpc::connect().await {
                            Ok(client) => {
                                *current.borrow_mut() = Some(client.clone());
                                offline.set(None);
                                generation.update(|g| *g += 1);
                                let reason = client.pump().await;
                                current.borrow_mut().take();
                                offline.set(Some(format!("Lost Zeron: {reason}")));
                                generation.update(|g| *g += 1);
                            }
                            Err(error) => {
                                offline.set(Some(format!(
                                    "Waiting for Zeron at {} ({error})",
                                    rpc::engine_url()
                                )));
                            }
                        }
                        sleep(Duration::from_secs(2)).await;
                    }
                });
            }
            // ZERON_PREVIEW=<markdown file>: render it offline through the
            // transcript view, for checking markdown without a live session.
            if let Ok(path) = std::env::var("ZERON_PREVIEW") {
                cx.render(
                    switch(|| (), move |_, cx| preview(cx, &path))
                        .w_full()
                        .h_full(),
                );
                return;
            }
            let window = cx.window.clone();
            cx.render(
                switch(
                    move || generation.get(),
                    move |_, cx| match current.borrow().clone() {
                        Some(client) => app(cx, client, window.clone()),
                        None => waiting(offline.clone(), window.clone()),
                    },
                )
                .w_full()
                .h_full(),
            );
        })
}

fn preview(cx: &mut Context, path: &str) -> View {
    let source =
        std::fs::read_to_string(path).unwrap_or_else(|e| format!("Couldn't read {path}: {e}"));
    let transcript: model::Shared = Rc::new(RefCell::new(Transcript::new(cx.runtime())));
    let anim = Anim::new(cx);
    let tool = |id: &str, call: Value, output: &str, extra: Value| {
        let mut part = json!({ "kind": "tool", "id": id, "call": call, "resolved": true, "isError": false, "output": output });
        if let (Some(part), Some(extra)) = (part.as_object_mut(), extra.as_object()) {
            part.extend(extra.clone());
        }
        part
    };
    let frame = json!({ "reset": [
        { "id": "u", "role": "user", "createdAt": 0, "parts": [{ "kind": "text", "id": "t", "text": "Render this markdown preview." }] },
        { "id": "a", "role": "assistant", "createdAt": 0, "status": "complete", "parts": [
            { "kind": "reasoning", "id": "r", "text": "Reading the file, then checking tables, lists and code render like Zeron's." },
            tool("t1", json!({ "kind": "readFile", "path": path }), "", json!({})),
            tool("t2", json!({ "kind": "exec", "command": "cargo test -p lumine-desktop --example zeron" }), "running 1 test\ntest markdown::tests::runs_map_back_to_source ... ok\ntest result: ok. 1 passed", json!({})),
            tool("t3", json!({ "kind": "editFile", "path": "crates/lumine-desktop/examples/zeron.rs" }), "", json!({
                "diffStats": [{ "path": "zeron.rs", "additions": 3, "deletions": 1 }],
                "diff": { "path": "zeron.rs", "oldText": "fn main() {\n    run();\n}\n", "newText": "fn main() {\n    // Preview first.\n    preview();\n    run();\n}\n" }
            })),
            { "kind": "text", "id": "md", "text": source },
        ]}
    ]});
    transcript.borrow_mut().apply(&frame, anim.now());
    let ctx = view::Ctx {
        anim: anim.clone(),
        rpc: None,
        transcript: transcript.clone(),
        tasks: cx.tasks(),
        toggles: Default::default(),
    };
    let blocks = transcript.borrow().blocks.clone();
    let body = {
        let (keys, previous) = (blocks.clone(), blocks.clone());
        keyed(
            move || keys.get(),
            move |block, cx| {
                let before = previous.with_untracked(|b| {
                    b.iter()
                        .position(|x| *x == block)
                        .and_then(|i| i.checked_sub(1))
                        .map(|i| b[i].clone())
                });
                view::block(cx, &ctx, &block, before)
            },
        )
        .w_full()
    };
    let offset = cx.state(0_f32);
    column()
        .w_full()
        .h_full()
        .bg(Color(13, 13, 15, 168))
        .text_color(TEXT)
        .child(div().w_full().h(38.))
        .child(
            scroll(offset)
                .scrollbar(true)
                .w_full()
                .grow()
                .min_h(0.)
                .child(
                    column().w_full().items_center().child(
                        column()
                            .w_full()
                            .max_w(832.)
                            .px(48.)
                            .pt(20.)
                            .pb(60.)
                            .child(body),
                    ),
                ),
        )
}

fn drag_region(view: View, window: lumine_desktop::WindowHandle) -> View {
    view.on_event(move |event| {
        if matches!(
            event.event,
            InputEvent::PointerDown {
                button: lumine::input::PointerButton::Primary,
                ..
            }
        ) {
            window.drag_window();
        }
    })
}

fn waiting(message: Signal<Option<String>>, window: lumine_desktop::WindowHandle) -> View {
    column()
        .w_full()
        .h_full()
        .bg(Color(10, 13, 20, 90))
        .text_color(TEXT)
        .child(drag_region(div().w_full().h(52.), window))
        .child(
            column()
                .grow()
                .items_center()
                .justify_center()
                .gap(10.)
                .child(
                    text("Zeron")
                        .text_size(24.)
                        .font_weight(650)
                        .letter_spacing(-0.4),
                )
                .child(
                    text_signal(move || message.get().unwrap_or_default())
                        .text_size(12.5)
                        .text_color(MUTED),
                ),
        )
}

/// A harness's models as (id, label, reasoning levels).
type Models = Signal<Vec<(String, String, Vec<String>)>>;

/// The app's shared state for one connection.
#[derive(Clone)]
struct App {
    rpc: Rpc,
    rows: Signal<Vec<ChatRow>>,
    statuses: Signal<HashMap<String, Status>>,
    selected: Signal<Option<String>>,
    models: Rc<RefCell<HashMap<String, Models>>>,
    /// Ticks every 30 s so relative times stay fresh.
    now: Signal<i64>,
    /// App-lifetime tasks, for loads cached beyond the view that starts them.
    tasks: lumine::compose::Tasks,
    anim: Anim,
}

fn app(cx: &mut Context, rpc: Rpc, window: lumine_desktop::WindowHandle) -> View {
    let state = App {
        rows: cx.state(Vec::new()),
        statuses: cx.state(HashMap::new()),
        selected: cx.state(None),
        models: Rc::default(),
        now: cx.state(model::now_seconds()),
        tasks: cx.tasks(),
        anim: Anim::new(cx),
        rpc: rpc.clone(),
    };
    // Chats are joined with project names from spaces.
    let raw_chats = Rc::new(RefCell::new(Value::Null));
    let spaces = Rc::new(RefCell::new(HashMap::new()));
    let refresh = {
        let (raw_chats, spaces, rows, selected) = (
            raw_chats.clone(),
            spaces.clone(),
            state.rows.clone(),
            state.selected.clone(),
        );
        move || {
            let joined = model::chat_rows(&raw_chats.borrow(), &spaces.borrow());
            // ZERON_OPEN=<title text> preselects a chat (for screenshots).
            let wanted = std::env::var("ZERON_OPEN").ok().map(|t| t.to_lowercase());
            let first = wanted
                .and_then(|wanted| {
                    joined
                        .iter()
                        .find(|row| row.title.to_lowercase().contains(&wanted))
                })
                .or(joined.first())
                .map(|row| row.id.clone());
            // Rows first: the chat view reads its row when it mounts.
            rows.set(joined);
            if selected.with_untracked(Option::is_none) && first.is_some() {
                selected.set(first);
            }
        }
    };
    let watch_chats = {
        let (raw_chats, refresh) = (raw_chats.clone(), refresh.clone());
        rpc.watch("WatchChats", json!({}), move |chats| {
            *raw_chats.borrow_mut() = chats;
            refresh();
        })
    };
    let watch_spaces = rpc.watch("WatchSpaces", json!({}), move |value| {
        *spaces.borrow_mut() = model::spaces(&value);
        refresh();
    });
    let watch_sessions = {
        let statuses = state.statuses.clone();
        rpc.watch("WatchSessions", json!({}), move |value| {
            statuses.set(model::statuses(&value));
        })
    };
    cx.retain((watch_chats, watch_spaces, watch_sessions));
    {
        let now = state.now.clone();
        cx.tasks().spawn(async move {
            loop {
                sleep(Duration::from_secs(30)).await;
                now.set(model::now_seconds());
            }
        });
    }
    let content = {
        let (state, window) = (state.clone(), window.clone());
        switch(
            {
                let selected = state.selected.clone();
                move || selected.get()
            },
            move |selected, cx| match selected {
                Some(id) => chat_view(cx, &state, id, window.clone()),
                None => column()
                    .w_full()
                    .h_full()
                    .items_center()
                    .justify_center()
                    .child(text("No sessions yet").text_size(13.).text_color(MUTED)),
            },
        )
        .w_full()
        .h_full()
    };
    // As in Zeron the transcript sits on the window's glass; ours lets a
    // little more of the desktop through.
    row()
        .w_full()
        .h_full()
        .items_stretch()
        .bg(Color(13, 13, 15, 168))
        .text_color(TEXT)
        .child(sidebar(cx, &state, window))
        .child(column().grow().min_w(0.).child(content))
}

// Sidebar --------------------------------------------------------------------

#[derive(Clone, PartialEq, Eq, Hash)]
enum SideKey {
    Project(String),
    Chat(String),
}

fn sidebar(cx: &mut Context, state: &App, window: lumine_desktop::WindowHandle) -> View {
    let offset = cx.state(0_f32);
    let query = cx.state(String::new());
    let collapsed = cx.state(HashSet::<String>::new());
    let items = {
        let (rows, query, collapsed) = (state.rows.clone(), query.clone(), collapsed.clone());
        move || {
            let needle = query.with(|q| q.trim().to_lowercase());
            let collapsed = collapsed.get();
            // Projects ordered by their most recent session.
            let mut order: Vec<String> = Vec::new();
            let mut by_project: HashMap<String, Vec<String>> = HashMap::new();
            rows.with(|rows| {
                for row in rows {
                    if !needle.is_empty()
                        && !row.title.to_lowercase().contains(&needle)
                        && !row.project.to_lowercase().contains(&needle)
                    {
                        continue;
                    }
                    if !by_project.contains_key(&row.project) {
                        order.push(row.project.clone());
                    }
                    by_project
                        .entry(row.project.clone())
                        .or_default()
                        .push(row.id.clone());
                }
            });
            order
                .into_iter()
                .flat_map(|project| {
                    let chats = by_project.remove(&project).unwrap_or_default();
                    let hidden = collapsed.contains(&project) && needle.is_empty();
                    std::iter::once(SideKey::Project(project)).chain(
                        chats
                            .into_iter()
                            .filter(move |_| !hidden)
                            .map(SideKey::Chat),
                    )
                })
                .collect::<Vec<_>>()
        }
    };
    let list = {
        let (state, collapsed) = (state.clone(), collapsed.clone());
        keyed(items, move |key, _| match key {
            SideKey::Project(name) => project_header(&state, &collapsed, name),
            SideKey::Chat(id) => chat_item(&state, id),
        })
        .w_full()
    };
    let search = {
        let placeholder = {
            let query = query.clone();
            text("Search sessions")
                .text_size(12.5)
                .text_color(FAINT)
                .reactive_style(move || {
                    if query.with(String::is_empty) {
                        Styles::new().flex()
                    } else {
                        Styles::new().hidden()
                    }
                })
        };
        row()
            .w_full()
            .h(32.)
            .px(10.)
            .gap(8.)
            .items_center()
            .rounded(9.)
            .bg(white(10))
            .border(1.)
            .border_color(white(12))
            .child(icon("search", 13., FAINT))
            .child(
                overlay().grow().min_w(0.).child(placeholder).child(
                    text_input("Search sessions", query)
                        .w_full()
                        .h(20.)
                        .text_size(12.5)
                        .bg(white(0))
                        .border(0.)
                        .p(0.)
                        .focus(|s| s),
                ),
            )
    };
    let footer = {
        let rows = state.rows.clone();
        row()
            .w_full()
            .h(44.)
            .px(16.)
            .gap(8.)
            .items_center()
            .border_edges(lumine::scene::Insets {
                top: 1.,
                right: 0.,
                bottom: 0.,
                left: 0.,
            })
            .border_color(white(12))
            .child(div().size(7., 7.).rounded(3.5).bg(Color(64, 214, 150, 255)))
            .child(text("Connected to Zeron").text_size(12.).text_color(MUTED))
            .child(div().grow())
            .child(
                text_signal(move || format!("{} sessions", rows.with(Vec::len)))
                    .text_size(11.5)
                    .text_color(FAINT),
            )
    };
    column()
        .w(256.)
        .h_full()
        .shrink_0()
        .bg(white(13))
        .border_edges(lumine::scene::Insets {
            top: 0.,
            right: 1.,
            bottom: 0.,
            left: 0.,
        })
        .border_color(white(26))
        .child(drag_region(div().w_full().h(38.).shrink_0(), window))
        .child(column().w_full().px(8.).pt(8.).pb(4.).child(search))
        .child(
            scroll(offset)
                .scrollbar(true)
                .w_full()
                .grow()
                .min_h(0.)
                .child(column().w_full().px(8.).pt(4.).pb(16.).gap(2.).child(list)),
        )
        .child(footer)
}

fn project_header(state: &App, collapsed: &Signal<HashSet<String>>, name: String) -> View {
    let count = {
        let (rows, name) = (state.rows.clone(), name.clone());
        text_signal(move || {
            rows.with(|rows| rows.iter().filter(|r| r.project == name).count())
                .to_string()
        })
        .text_size(11.)
        .text_color(white(70))
    };
    let chevron = {
        let (collapsed, name) = (collapsed.clone(), name.clone());
        svg_signal("fold", move || {
            let closed = collapsed.with(|c| c.contains(&name));
            let angle = if closed { -FRAC_PI_2 } else { 0. };
            Arc::new(
                icon_data("chevron")
                    .tinted(white(90))
                    .transformed(Affine::rotation(angle)),
            )
        })
        .size(11., 11.)
    };
    let toggle = {
        let (collapsed, name) = (collapsed.clone(), name.clone());
        move || {
            collapsed.update(|c| {
                if !c.remove(&name) {
                    c.insert(name.clone());
                }
            });
        }
    };
    // Zeron's section header: 28 px, muted label, chevron at the right
    // that turns from right (closed) to down (open).
    row()
        .w_full()
        .h(28.)
        .mt(12.)
        .mb(4.)
        .px(8.)
        .gap(6.)
        .items_center()
        .rounded(8.)
        .cursor(Cursor::Pointer)
        .hover(|s| s.bg(white(10)))
        .on_click(toggle)
        .child(
            text(name)
                .w(0.)
                .grow()
                .truncate()
                .text_size(12.)
                .font_weight(500)
                .text_color(white(92)),
        )
        .child(count)
        .child(chevron)
}

fn chat_item(state: &App, id: String) -> View {
    let row_of = {
        let (rows, id) = (state.rows.clone(), id.clone());
        move || rows.with(|rows| rows.iter().find(|r| r.id == id).cloned())
    };
    let title = {
        let row_of = row_of.clone();
        text_signal(move || row_of().map(|r| r.title).unwrap_or_default())
            .w(0.)
            .grow()
            .truncate()
    };
    let time = {
        let (row_of, now) = (row_of.clone(), state.now.clone());
        text_signal(move || {
            now.get();
            row_of().map(|r| model::ago(r.updated)).unwrap_or_default()
        })
        .shrink_0()
        .text_size(10.)
        .font_weight(500)
        .text_color(FAINT)
    };
    let status = {
        let (statuses, id) = (state.statuses.clone(), id.clone());
        move || statuses.with(|s| s.get(&id).copied().unwrap_or(Status::Idle))
    };
    // The corner: Zeron's mini spinner and "Working", a dot and "Input" /
    // "Failed", or the relative time.
    let corner = {
        let (status, word_status, anim) = (status.clone(), status.clone(), state.anim.clone());
        let guard = RefCell::new(None::<view::AmbientGuard>);
        let spinner = column().gap(1.).children((0..3).map(|r| {
            let (anim, status) = (anim.clone(), status.clone());
            row().gap(1.).children((0..2).map(move |c| {
                let (anim, status) = (anim.clone(), status.clone());
                const RING: [(usize, usize); 6] = [(0, 0), (0, 1), (1, 1), (2, 1), (2, 0), (1, 0)];
                let k = RING.iter().position(|&cell| cell == (r, c)).unwrap_or(0) as f32;
                let colour = [
                    Color(171, 161, 249, 255),
                    Color(139, 124, 246, 255),
                    Color(114, 102, 202, 255),
                ][r];
                div()
                    .size(2., 2.)
                    .rounded(1.)
                    .bg(colour)
                    .reactive_style(move || {
                        // Hidden unless working: no ticking while hidden.
                        if status() != Status::Working {
                            return Styles::new();
                        }
                        let t = (anim.ambient_clock.get() / 0.75 - k / 6.).rem_euclid(1.);
                        let o = if t < 0.45 {
                            1. - 0.9 * (t / 0.45)
                        } else if t < 0.92 {
                            0.1
                        } else {
                            1.
                        };
                        Styles::new().opacity(o)
                    })
            }))
        }));
        let glyph = div().size(6., 6.).rounded(3.).reactive_style({
            let status = status.clone();
            move || match status() {
                Status::AwaitingInput => Styles::new().flex().bg(ACCENT),
                Status::Errored => Styles::new().flex().bg(DANGER),
                _ => Styles::new().hidden(),
            }
        });
        let spinner = spinner.reactive_style({
            let status = status.clone();
            move || {
                let working = status() == Status::Working;
                let mut slot = guard.borrow_mut();
                match (working, slot.is_some()) {
                    (true, false) => *slot = Some(anim.ambient()),
                    (false, true) => *slot = None,
                    _ => {}
                }
                if working {
                    Styles::new().flex()
                } else {
                    Styles::new().hidden()
                }
            }
        });
        let word = text_signal(move || match word_status() {
            Status::Working => "Working".into(),
            Status::AwaitingInput => "Input".into(),
            Status::Errored => "Failed".into(),
            Status::Idle => String::new(),
        })
        .text_size(10.)
        .font_weight(500)
        .reactive_style({
            let status = status.clone();
            move || match status() {
                Status::Working => Styles::new().flex().text_color(Color(139, 124, 246, 150)),
                Status::AwaitingInput => Styles::new().flex().text_color(Color(139, 124, 246, 170)),
                Status::Errored => Styles::new().flex().text_color(Color(248, 113, 113, 170)),
                Status::Idle => Styles::new().hidden(),
            }
        });
        let time = time.reactive_style(move || {
            if status() == Status::Idle {
                Styles::new().flex()
            } else {
                Styles::new().hidden()
            }
        });
        row()
            .shrink_0()
            .gap(4.)
            .items_center()
            .child(spinner)
            .child(glyph)
            .child(word)
            .child(time)
    };
    let (selected, select) = (state.selected.clone(), state.selected.clone());
    let pick = id.clone();
    button()
        .w_full()
        .h(29.)
        .px(8.)
        .gap(8.)
        .rounded(8.)
        .items_center()
        .text_size(13.)
        .bg(white(0))
        .border(0.)
        .hover(|s| s.bg(white(24)))
        .active(|s| s.bg(white(30)))
        .focus(|s| s)
        .reactive_style(move || {
            let on = selected.with(|s| s.as_deref() == Some(id.as_str()));
            Styles::new()
                .bg(Color(235, 235, 235, if on { 28 } else { 0 }))
                .text_color(if on { TEXT } else { white(204) })
        })
        .child(title)
        .child(corner)
        .on_click(move || {
            select.set(Some(pick.clone()));
        })
}

// Chat -----------------------------------------------------------------------

fn chat_view(
    cx: &mut Context,
    state: &App,
    id: String,
    window: lumine_desktop::WindowHandle,
) -> View {
    let anim = state.anim.clone();
    let transcript: model::Shared = Rc::new(RefCell::new(Transcript::new(cx.runtime())));
    let (blocks, loaded, streaming) = {
        let t = transcript.borrow();
        (t.blocks.clone(), t.loaded.clone(), t.streaming.clone())
    };
    let page = cx.state(0_f32);
    let follow = Follow::new(cx, page.clone());
    // ZERON_SCROLL_SPEED=<points/s>: sweep the transcript up and down, like a
    // long trackpad fling, for profiling.
    if let Some(speed) = std::env::var("ZERON_SCROLL_SPEED")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
    {
        let (page, frames) = (page.clone(), cx.frames());
        cx.tasks().spawn(async move {
            sleep(Duration::from_secs(2)).await;
            let start = std::time::Instant::now();
            loop {
                frames.next().await;
                let travel = start.elapsed().as_secs_f32() * speed % 4000.;
                page.set(if travel < 2000. {
                    2000. - travel
                } else {
                    travel - 2000.
                });
            }
        });
    }
    let ctx = view::Ctx {
        anim: anim.clone(),
        rpc: Some(state.rpc.clone()),
        transcript: transcript.clone(),
        tasks: state.tasks.clone(),
        toggles: Default::default(),
    };
    // The transcript watch; a desync remounts it for a fresh snapshot.
    let resync = cx.state(0_u32);
    let subscription = {
        let (rpc, transcript, id, resync, follow, anim) = (
            state.rpc.clone(),
            transcript.clone(),
            id.clone(),
            resync.clone(),
            follow.clone(),
            anim.clone(),
        );
        switch(
            {
                let resync = resync.clone();
                move || resync.get()
            },
            move |_, cx| {
                let (transcript, resync, follow, anim) = (
                    transcript.clone(),
                    resync.clone(),
                    follow.clone(),
                    anim.clone(),
                );
                let runtime = cx.runtime();
                // `openingTail`: the newest 128 entries first, the full history
                // after, so long sessions paint at once (as Zeron opens them).
                let params = json!({ "chatId": id, "openingTail": true });
                let mounted = std::time::Instant::now();
                let (frames, tasks) = (cx.frames(), cx.tasks());
                let trace = std::env::var_os("ZERON_TRACE").is_some();
                // ZERON_REPLAY=<n>: re-stream this chat's last n entries
                // locally (read-only), for profiling streaming.
                let replay: Option<usize> = std::env::var("ZERON_REPLAY")
                    .ok()
                    .and_then(|n| n.parse().ok());
                let replaying = Rc::new(Cell::new(false));
                let watch = rpc.watch("WatchDocMessages", params, move |frame| {
                    if replaying.get() {
                        return;
                    }
                    let frame = match (replay, frame.get("reset").and_then(Value::as_array)) {
                        (Some(tail), Some(entries)) if frame.get("historyPending").is_none() => {
                            replaying.set(true);
                            let (reset, schedule) = replay_schedule(entries, tail);
                            let (transcript, runtime, anim, follow) = (
                                transcript.clone(),
                                runtime.clone(),
                                anim.clone(),
                                follow.clone(),
                            );
                            tasks.spawn(async move {
                                let start = std::time::Instant::now();
                                sleep(Duration::from_millis(600)).await;
                                for (at, frame) in schedule {
                                    let due = start + Duration::from_millis(600 + at);
                                    if let Some(wait) = due.checked_duration_since(std::time::Instant::now()) {
                                        sleep(wait).await;
                                    }
                                    let now = anim.now();
                                    runtime.batch(|| transcript.borrow_mut().apply(&frame, now));
                                    if frame.get("append").is_some() {
                                        anim.keep(0.45);
                                    }
                                    follow.schedule();
                                }
                                eprintln!("REPLAY done in {:?}", start.elapsed());
                            });
                            reset
                        }
                        _ => frame,
                    };
                    let first = frame.get("reset").is_some();
                    let now = anim.now();
                    let started = std::time::Instant::now();
                    // Batched: views built by these updates read the transcript.
                    let mut apply_time = std::time::Duration::ZERO;
                    let applied = runtime.batch(|| {
                        let applied = transcript.borrow_mut().apply(&frame, now);
                        apply_time = started.elapsed();
                        applied
                    });
                    if trace && first {
                        eprintln!(
                            "TRACE reset at {:?}: {} entries, model {:?}, views {:?} (tail preview: {})",
                            mounted.elapsed(),
                            frame["reset"].as_array().map_or(0, Vec::len),
                            apply_time,
                            started.elapsed() - apply_time,
                            frame.get("historyPending").is_some()
                        );
                        let (frames, mounted) = (frames.clone(), mounted);
                        tasks.spawn(async move {
                            frames.next().await;
                            frames.next().await;
                            eprintln!("TRACE painted by {:?}", mounted.elapsed());
                        });
                    }
                    if !applied {
                        resync.update(|n| *n += 1);
                        return;
                    }
                    if frame.get("append").is_some() {
                        // Keep the veil fading while text streams.
                        anim.keep(0.45);
                    }
                    if first {
                        follow.pin();
                    }
                    follow.schedule();
                });
                cx.retain(watch);
                div()
            },
        )
    };
    let chat = {
        let (rows, id) = (state.rows.clone(), id.clone());
        move || rows.with(|rows| rows.iter().find(|r| r.id == id).cloned())
    };
    let status = {
        let (statuses, id) = (state.statuses.clone(), id.clone());
        move || statuses.with(|s| s.get(&id).copied().unwrap_or(Status::Idle))
    };
    let working = {
        let (status, streaming) = (status.clone(), streaming.clone());
        move || status() == Status::Working || streaming.get()
    };
    // The transcript is virtualized: only rows near the viewport mount. The
    // rest take heights estimated from their text (prepared text measures at
    // any width by arithmetic) until they mount and are measured, so long
    // sessions open and scroll at the cost of what is on screen.
    #[derive(Clone, PartialEq, Eq, Hash)]
    enum Row {
        /// The top padding and, until the first snapshot, a skeleton.
        Head,
        Block(model::Block),
        /// The working indicator and the bottom padding.
        Tail,
    }
    // Resting at the end, the list stays there as rows are measured and
    // messages arrive.
    let heights = VariableHeights::new(&cx.runtime(), 0, 80.).anchor_end();
    let rows = {
        let blocks = blocks.clone();
        move || {
            let mut rows = vec![Row::Head];
            rows.extend(blocks.get().into_iter().map(Row::Block));
            rows.push(Row::Tail);
            rows
        }
    };
    // Heights of rows not yet mounted: text measured exactly, as laid out.
    let estimate = {
        let (ctx, measure) = (ctx.clone(), cx.text_measure());
        move |rows: &[Row], index: usize| match &rows[index] {
            Row::Head => 26.,
            Row::Tail => 48.,
            Row::Block(block) => {
                let previous = match index.checked_sub(1).map(|p| &rows[p]) {
                    Some(Row::Block(previous)) => Some(previous),
                    _ => None,
                };
                view::estimate(&ctx, &measure, block, previous, COLUMN)
            }
        }
    };
    let build = {
        let (ctx, previous, anim) = (ctx.clone(), blocks.clone(), anim.clone());
        let (loaded, working) = (loaded.clone(), working.clone());
        move |_, row: Row, cx: &mut Context| {
            let content = match row {
                Row::Head => {
                    // A skeleton until the first snapshot lands.
                    let loaded = loaded.clone();
                    column().w_full().pt(26.).child(
                        column()
                            .w_full()
                            .gap(12.)
                            .reactive_style(move || {
                                if loaded.get() {
                                    Styles::new().hidden()
                                } else {
                                    Styles::new().flex()
                                }
                            })
                            .children(
                                [72_f32, 90., 55., 80.]
                                    .map(|w| div().w_percent(w).h(12.).rounded(6.).bg(white(12))),
                            ),
                    )
                }
                Row::Block(block) => {
                    let before = previous.with_untracked(|b| {
                        b.iter()
                            .position(|x| *x == block)
                            .and_then(|i| i.checked_sub(1))
                            .map(|i| b[i].clone())
                    });
                    view::block(cx, &ctx, &block, before)
                }
                Row::Tail => {
                    column()
                        .w_full()
                        .pb(48.)
                        .child(view::trailer(cx, &anim, working.clone()))
                }
            };
            column()
                .w_full()
                .items_center()
                .child(column().w_full().max_w(832.).px(48.).child(content))
        }
    };
    // Zeron's column: 736 wide with 48 px gutters, first row 26 px under
    // the titlebar. The viewport is an isolated layer whose edges fade, so
    // text dissolves under the titlebar and just above the composer.
    // The viewport is the flex item itself: percentage heights inside a
    // grown item do not resolve, so the list grows into the space instead.
    // The composer overlaps its last 28 px, where its bottom fade runs.
    let scroller = measured_rows(page.clone(), heights, 2, rows, estimate, build)
        // Zeron's per-pixel edge mask: content dissolves over 28 px under the
        // titlebar and into the composer, with no layer to repaint.
        .fade_edges(28., 32.)
        .scrollbar(true)
        .w_full()
        .grow()
        .min_h(0.);
    let jump = {
        let (follow, pinned) = (follow.clone(), follow.pinned.clone());
        row()
            .h(30.)
            .px(12.)
            .gap(6.)
            .items_center()
            .rounded(15.)
            .bg(Color(40, 44, 56, 235))
            .border(1.)
            .border_color(white(24))
            .cursor(Cursor::Pointer)
            .text_size(12.)
            .text_color(TEXT)
            .hover(|s| s.bg(Color(52, 57, 72, 245)))
            .reactive_style(move || {
                if pinned.get() {
                    Styles::new().hidden()
                } else {
                    Styles::new().flex()
                }
            })
            .on_click(move || {
                follow.pin();
                follow.schedule();
            })
            .child(text("↓"))
            .child(text("Latest"))
    };
    let viewport = column()
        .w_full()
        .grow()
        .min_h(0.)
        .child(scroller)
        // "Latest", floating above the composer while scrolled up.
        .child(
            row()
                .absolute()
                .left(0.)
                .bottom(40.)
                .w_full()
                .justify_center()
                .child(jump),
        );
    let bottom = column()
        .w_full()
        .shrink_0()
        .mt(-28.)
        .items_center()
        .px(16.)
        .pb(8.)
        .child(composer(cx, state, &id, working))
        .child(footer(chat.clone()));
    column()
        .w_full()
        .h_full()
        .child(subscription)
        .child(titlebar(chat.clone(), window))
        .child(viewport)
        .child(bottom)
}

/// A reset holding all but the last `tail` entries, then those entries
/// streamed back: each appears empty, gains its parts one by one, and text
/// arrives six characters every 16 ms (about 375 characters a second).
fn replay_schedule(entries: &[Value], tail: usize) -> (Value, Vec<(u64, Value)>) {
    let split = entries.len().saturating_sub(tail);
    let reset = json!({ "reset": entries[..split] });
    let mut schedule = Vec::new();
    let mut at = 0_u64;
    let mut after = entries[..split].last().and_then(|e| e.get("id")).cloned();
    for entry in &entries[split..] {
        let id = entry.get("id").cloned().unwrap_or(Value::Null);
        let parts = entry
            .get("parts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut shown: Vec<Value> = Vec::new();
        let with_parts = |shown: &[Value]| {
            let mut entry = entry.clone();
            entry["parts"] = json!(shown);
            entry
        };
        schedule.push((
            at,
            json!({ "upsert": [{ "entry": with_parts(&shown), "after": after }] }),
        ));
        at += 250;
        for part in parts {
            let text = part.get("text").and_then(Value::as_str).map(str::to_owned);
            let mut empty = part.clone();
            if text.is_some() {
                empty["text"] = json!("");
            }
            shown.push(empty);
            schedule.push((at, json!({ "upsert": [{ "entry": with_parts(&shown) }] })));
            at += if text.is_some() { 60 } else { 350 };
            if let Some(text) = text {
                let chars: Vec<char> = text.chars().collect();
                for chunk in chars.chunks(6) {
                    let chunk: String = chunk.iter().collect();
                    schedule.push((
                        at,
                        json!({ "append": [{ "entry": id, "part": part.get("id"), "text": chunk }] }),
                    ));
                    at += 16;
                }
                shown.last_mut().unwrap()["text"] = json!(text);
            }
        }
        after = Some(id);
    }
    (reset, schedule)
}

/// Zeron's titlebar row: 38 px, content centred with the traffic lights.
fn titlebar(
    chat: impl Fn() -> Option<ChatRow> + Clone + 'static,
    window: lumine_desktop::WindowHandle,
) -> View {
    let (title_of, meta_of) = (chat.clone(), chat);
    drag_region(
        row()
            .w_full()
            .h(38.)
            .pt(4.)
            .px(16.)
            .gap(6.)
            .shrink_0()
            .items_center()
            .child(icon("bot", 14., MUTED))
            .child(
                text_signal(move || title_of().map(|c| c.title).unwrap_or_default())
                    .truncate()
                    .min_w(0.)
                    .text_size(12.)
                    .font_weight(500)
                    .text_color(white(216)),
            )
            .child(
                text_signal(move || meta_of().map(|c| c.project).unwrap_or_default())
                    .shrink_0()
                    .text_size(12.)
                    .text_color(white(88)),
            ),
        window,
    )
}

/// Under the composer: checkout and branch, as in Zeron.
fn footer(chat: impl Fn() -> Option<ChatRow> + Clone + 'static) -> View {
    let label = |glyph: &'static str, value: View| {
        row()
            .h(20.)
            .px(8.)
            .gap(6.)
            .items_center()
            .text_size(12.)
            .font_weight(500)
            .text_color(white(100))
            .child(text(glyph).text_size(11.))
            .child(value)
    };
    row()
        .w_full()
        .max_w(736.)
        .h(24.)
        .mt(2.)
        .px(10.)
        .gap(4.)
        .items_center()
        .child(label("▢", text("Local checkout")))
        .child(label(
            "⎇",
            text_signal(move || {
                chat()
                    .and_then(|c| c.branch)
                    .unwrap_or_else(|| "No ref".into())
            }),
        ))
}

/// Keeps the newest content in view while the reader is at the bottom.
/// Jumps wait a frame: scroll offsets clamp to the laid-out content, so a
/// jump made before new content is laid out would stop short.
struct Follow {
    offset: Signal<f32>,
    /// The largest offset seen, i.e. the bottom as of the last layout.
    bottom: Cell<f32>,
    pinned: Signal<bool>,
    scheduled: Cell<bool>,
    tasks: lumine::compose::Tasks,
    frames: lumine::frame::FrameClock,
}

impl Follow {
    fn new(cx: &mut Context, offset: Signal<f32>) -> Rc<Self> {
        let follow = Rc::new(Self {
            offset: offset.clone(),
            bottom: Cell::new(0.),
            pinned: cx.state(true),
            scheduled: Cell::new(false),
            tasks: cx.tasks(),
            frames: cx.frames(),
        });
        // Scrolling well above the bottom unpins; scrolling back pins again.
        let weak = Rc::downgrade(&follow);
        let effect = cx.runtime().effect(move || {
            let current = offset.get();
            // The jump's sentinel, before layout clamps it to the bottom.
            if current >= 1e8 {
                return;
            }
            if let Some(follow) = weak.upgrade() {
                follow.bottom.set(follow.bottom.get().max(current));
                follow.pinned.set(current + 80. >= follow.bottom.get());
            }
        });
        cx.retain(effect);
        follow
    }
    fn pin(&self) {
        self.pinned.set(true);
        self.bottom.set(0.);
    }
    /// After the next layout, jump to the bottom if pinned.
    fn schedule(self: &Rc<Self>) {
        if self.scheduled.replace(true) {
            return;
        }
        let follow = self.clone();
        self.tasks.spawn(async move {
            follow.frames.next().await;
            follow.scheduled.set(false);
            if follow.pinned.with_untracked(|p| *p) {
                follow.bottom.set(0.);
                follow.offset.set(1e9);
            }
        });
    }
}

// Composer -------------------------------------------------------------------

fn composer(
    cx: &mut Context,
    state: &App,
    id: &str,
    working: impl Fn() -> bool + Clone + 'static,
) -> View {
    let draft = cx.state(String::new());
    let sending = cx.state(false);
    let notice = cx.state(None::<String>);
    let send = {
        let (state, id, draft, sending, notice, working) = (
            state.clone(),
            id.to_owned(),
            draft.clone(),
            sending.clone(),
            notice.clone(),
            working.clone(),
        );
        let tasks = cx.tasks();
        Rc::new(move || {
            let prompt = draft.with_untracked(|d| d.trim().to_owned());
            if prompt.is_empty() || sending.with_untracked(|s| *s) {
                return;
            }
            let Some(chat) = state
                .rows
                .with_untracked(|rows| rows.iter().find(|r| r.id == id).cloned())
            else {
                return;
            };
            let busy = working();
            let (rpc, draft, sending, notice) = (
                state.rpc.clone(),
                draft.clone(),
                sending.clone(),
                notice.clone(),
            );
            sending.set(true);
            tasks.spawn(async move {
                let result = if busy {
                    // Mid-turn: queue it to run when the turn ends, as Zeron does.
                    rpc.call(
                        "QueueMessage",
                        json!({ "chatId": chat.id, "text": prompt, "holdForTurnEnd": true }),
                    )
                    .await
                } else {
                    let config = &chat.config;
                    rpc.call(
                        "QueueCommand",
                        json!({ "chatId": chat.id, "command": {
                            "kind": "run",
                            "messageId": uuid::Uuid::new_v4().to_string(),
                            "request": {
                                "prompt": prompt,
                                "harness": chat.harness(),
                                "model": config.get("model").cloned().unwrap_or(Value::Null),
                                "reasoning": config.get("reasoning").cloned().unwrap_or(Value::Null),
                                "modelOptions": config.get("modelOptions").cloned().unwrap_or_else(|| json!({})),
                                "cwd": chat.cwd.clone().unwrap_or_else(|| "~".into()),
                                "sandbox": config.get("sandbox").cloned().unwrap_or_else(|| json!("workspace-write")),
                                "autoApprove": false,
                                "resume": null,
                            }
                        }}),
                    )
                    .await
                };
                sending.set(false);
                match result {
                    Ok(_) => {
                        draft.set(String::new());
                        notice.set(if busy {
                            Some("Queued for after this turn".into())
                        } else {
                            None
                        });
                    }
                    Err(error) => {
                        notice.set(Some(format!("Couldn't send: {error}")));
                    }
                }
            });
        })
    };
    let interrupt = {
        let (rpc, id) = (state.rpc.clone(), id.to_owned());
        let tasks = cx.tasks();
        move || {
            let (rpc, id) = (rpc.clone(), id.clone());
            tasks.spawn(async move {
                let _ = rpc
                    .call(
                        "QueueCommand",
                        json!({ "chatId": id, "command": { "kind": "interrupt" } }),
                    )
                    .await;
            });
        }
    };
    // Test hook: ZERON_SEND=<text> sends it through this composer once.
    if let Ok(text_) = std::env::var("ZERON_SEND") {
        static SENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !SENT.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let (draft, send) = (draft.clone(), send.clone());
            cx.tasks().spawn(async move {
                sleep(Duration::from_millis(2500)).await;
                println!("ZERON_SEND {text_:?}");
                draft.set(text_);
                send();
            });
        }
    }
    let placeholder = {
        let draft = draft.clone();
        text("Do anything…")
            .text_size(14.)
            .line_height(22.75)
            .text_color(FAINT)
            .reactive_style(move || {
                if draft.with(String::is_empty) {
                    Styles::new().flex()
                } else {
                    Styles::new().hidden()
                }
            })
    };
    let input = {
        let (send, lines) = (send.clone(), draft.clone());
        text_area("Message", draft.clone())
            .w_full()
            .text_wrap(true)
            .text_size(14.)
            .line_height(22.75)
            .bg(white(0))
            .border(0.)
            .p(0.)
            .focus(|s| s)
            // Zeron's text area: 76 px empty, growing with its lines to 260.
            .reactive_style(move || {
                let count = lines.with(|d| d.lines().count() + usize::from(d.ends_with('\n')));
                Styles::new().h((count as f32 * 22.75).clamp(76., 260.))
            })
            .on_event(move |event| {
                if event.phase == EventPhase::Capture
                    && matches!(
                        event.event,
                        InputEvent::KeyDown { key: Key::Enter, modifiers, .. } if !modifiers.shift
                    )
                {
                    event.prevent_default();
                    event.stop_propagation();
                    send();
                }
            })
    };
    // A 28 px light circle: an arrow to send (35% when empty), a rounded
    // square to stop a running turn.
    let action = {
        let (working2, working3, draft) = (working.clone(), working.clone(), draft.clone());
        let arrow = text("↑")
            .text_size(14.)
            .font_weight(700)
            .text_color(Color(6, 6, 6, 255))
            .reactive_style(move || {
                if working2() {
                    Styles::new().hidden()
                } else {
                    Styles::new().flex()
                }
            });
        let square = div()
            .size(11., 11.)
            .rounded(3.)
            .bg(Color(6, 6, 6, 255))
            .reactive_style(move || {
                if working3() {
                    Styles::new().flex()
                } else {
                    Styles::new().hidden()
                }
            });
        let (busy, click) = (working.clone(), working.clone());
        button()
            .size(28., 28.)
            .rounded(14.)
            .items_center()
            .justify_center()
            .shrink_0()
            .bg(Color(232, 232, 234, 255))
            .focus(|s| s)
            .hover(|s| s.opacity(0.85))
            .reactive_style(move || {
                let empty = draft.with(|d| d.trim().is_empty());
                Styles::new().opacity(if busy() || !empty { 1. } else { 0.35 })
            })
            .child(arrow)
            .child(square)
            .on_click(move || {
                if click() {
                    interrupt();
                } else {
                    send();
                }
            })
    };
    let hint = text_signal(move || notice.get().unwrap_or_default())
        .text_size(11.5)
        .text_color(FAINT);
    // Zeron's frosted pill: 736 wide, radius 26, a 16 px backdrop blur
    // over a faint fill, and a cool hairline border.
    column()
        .w_full()
        .max_w(736.)
        .rounded(26.)
        .blur(16.)
        .bg(Color(8, 8, 10, 70))
        .border(1.)
        .border_color(Color(189, 199, 209, 30))
        .child(
            overlay()
                .w_full()
                .px(16.)
                .pt(16.)
                .pb(4.)
                .child(placeholder)
                .child(input),
        )
        .child(
            row()
                .w_full()
                .h(42.)
                .pt(2.)
                .pb(8.)
                .px(12.)
                .items_center()
                .gap(2.)
                .child(pickers_for(state, id))
                .child(hint.w(0.).grow().truncate().pl(8.))
                .child(action.ml(8.)),
        )
}

/// Set fields of the chat's config. `setChatConfig` replaces the whole
/// config, so the rest is copied from the chat row.
fn set_config(state: &App, id: &str, chat: &ChatRow, changes: &[(&str, Value)]) {
    let mut config = chat.config.clone();
    if !config.is_object() {
        config =
            json!({ "harness": chat.harness(), "modelOptions": {}, "sandbox": "workspace-write" });
    }
    for (key, value) in changes {
        config[*key] = value.clone();
    }
    let (rpc, id) = (state.rpc.clone(), id.to_owned());
    state.tasks.spawn(async move {
        if let Err(error) = rpc
            .call(
                "Mutate",
                json!({ "op": "setChatConfig", "chatId": id, "config": config }),
            )
            .await
        {
            eprintln!("zeron: updating the chat config failed: {error}");
        }
    });
}

fn models_for(state: &App, cx: &mut Context, harness: &str) -> Models {
    state
        .models
        .borrow_mut()
        .entry(harness.to_owned())
        .or_insert_with(|| {
            let models = cx.state(Vec::new());
            let (rpc, out, harness) = (state.rpc.clone(), models.clone(), harness.to_owned());
            state.tasks.spawn(async move {
                if let Ok(list) = rpc
                    .call("ListModels", json!({ "harness": harness, "force": false }))
                    .await
                {
                    let list = list
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|m| {
                            let id = m.get("id")?.as_str()?.to_owned();
                            let label = m
                                .get("label")
                                .and_then(Value::as_str)
                                .unwrap_or(&id)
                                .to_owned();
                            let levels = m
                                .get("reasoningLevels")
                                .and_then(Value::as_array)
                                .into_iter()
                                .flatten()
                                .filter_map(|l| l.as_str().map(str::to_owned))
                                .collect();
                            Some((id, label, levels))
                        })
                        .collect();
                    out.set(list);
                }
            });
            models
        })
        .clone()
}

/// Model and reasoning pickers, rebuilt when the chat's harness changes: it
/// decides the models offered.
fn pickers_for(state: &App, id: &str) -> View {
    let harness = {
        let (rows, id) = (state.rows.clone(), id.to_owned());
        move || rows.with(|rows| rows.iter().find(|r| r.id == id).map(ChatRow::harness))
    };
    let (state, id) = (state.clone(), id.to_owned());
    switch(harness, move |harness, cx| {
        pickers(
            cx,
            &state,
            &id,
            harness.unwrap_or_else(|| "claude-code".into()),
        )
    })
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Zeron's composer chip: 32 px, radius 8, a hover wash.
fn pill(label: View, open: Signal<bool>) -> View {
    button()
        .h(32.)
        .px(8.)
        .gap(6.)
        .rounded(8.)
        .items_center()
        .shrink_0()
        .bg(white(0))
        .border(0.)
        .hover(|s| s.bg(white(28)))
        .focus(|s| s)
        .text_color(white(220))
        .child(label)
        .child(svg("more", Arc::new(icon_data("chevron").tinted(FAINT))).size(10., 10.))
        .on_click(move || {
            open.set(true);
        })
}

fn menu_entry(label: String, current: bool, action: impl FnMut() + 'static) -> View {
    menu_item(label)
        .h(30.)
        .px(10.)
        .rounded(7.)
        .items_center()
        .text_size(12.5)
        .text_color(if current { TEXT } else { white(205) })
        .bg(white(if current { 20 } else { 0 }))
        .focus(|s| s.bg(white(26)))
        .hover(|s| s.bg(white(18)))
        .on_click(action)
}

fn menu_panel(view: View, width: f32, entries: usize) -> View {
    view.w(width)
        .h(entries.clamp(1, 12) as f32 * 32. + 12.)
        .p(6.)
        .gap(2.)
        .rounded(12.)
        .bg(Color(30, 33, 42, 248))
        .border(1.)
        .border_color(white(24))
}

fn pickers(cx: &mut Context, state: &App, id: &str, harness: String) -> View {
    let models = models_for(state, cx, &harness);
    let chat = {
        let (rows, id) = (state.rows.clone(), id.to_owned());
        move || rows.with(|rows| rows.iter().find(|r| r.id == id).cloned())
    };
    // Test hook: ZERON_MODEL=<model id> picks it once the list loads.
    if let Ok(wanted) = std::env::var("ZERON_MODEL") {
        let (state2, id2, chat, models) =
            (state.clone(), id.to_owned(), chat.clone(), models.clone());
        let done = Cell::new(false);
        let effect = cx.runtime().effect(move || {
            let known = models.with(|m| m.iter().any(|(model, _, _)| *model == wanted));
            if known
                && !done.replace(true)
                && let Some(chat) = chat()
            {
                println!("ZERON_MODEL switching {} to {wanted}", chat.id);
                set_config(&state2, &id2, &chat, &[("model", json!(wanted))]);
            }
        });
        cx.retain(effect);
    }
    let model_label = {
        let (chat, models, harness) = (chat.clone(), models.clone(), harness.clone());
        move || match chat().and_then(|c| c.model()) {
            Some(model) => models.with(|m| {
                m.iter()
                    .find(|(id, _, _)| *id == model)
                    .map_or(model.clone(), |(_, label, _)| label.clone())
            }),
            None => format!("{} default", title_case(&harness.replace('-', " "))),
        }
    };
    let levels = {
        let (chat, models) = (chat.clone(), models.clone());
        move || {
            let model = chat().and_then(|c| c.model());
            models.with(|m| {
                m.iter()
                    .find(|(id, _, _)| Some(id) == model.as_ref())
                    .or_else(|| m.first())
                    .map(|(_, _, levels)| levels.clone())
                    .unwrap_or_default()
            })
        }
    };
    let reasoning_label = {
        let chat = chat.clone();
        move || {
            chat()
                .and_then(|c| {
                    c.config
                        .get("reasoning")
                        .and_then(Value::as_str)
                        .map(title_case)
                })
                .unwrap_or_else(|| "Default".into())
        }
    };
    let model_open = cx.state(false);
    let model_picker = {
        let (state, id, chat) = (state.clone(), id.to_owned(), chat.clone());
        switch(
            {
                let models = models.clone();
                move || models.get()
            },
            move |list, _| {
                let current = chat().and_then(|c| c.model());
                let count = list.len();
                let entries: Vec<View> = list
                    .into_iter()
                    .map(|(model, label, _)| {
                        let (state, id, chat) = (state.clone(), id.clone(), chat.clone());
                        let is_current = current.as_deref() == Some(model.as_str());
                        menu_entry(label, is_current, move || {
                            if let Some(chat) = chat() {
                                set_config(&state, &id, &chat, &[("model", json!(model))]);
                            }
                        })
                    })
                    .collect();
                let label = model_label.clone();
                let anchor = pill(
                    text_signal(label).text_size(12.).font_weight(560),
                    model_open.clone(),
                );
                menu_panel(menu("Model", model_open.clone(), anchor), 260., count).children(entries)
            },
        )
    };
    let reasoning_open = cx.state(false);
    let reasoning_picker = {
        let (state, id, chat) = (state.clone(), id.to_owned(), chat.clone());
        switch(levels, move |levels, _| {
            if levels.is_empty() {
                return div();
            }
            let current = chat().and_then(|c| {
                c.config
                    .get("reasoning")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
            let count = levels.len();
            let entries: Vec<View> = levels
                .into_iter()
                .map(|level| {
                    let (state, id, chat) = (state.clone(), id.clone(), chat.clone());
                    let is_current = current.as_deref() == Some(level.as_str());
                    menu_entry(title_case(&level), is_current, move || {
                        if let Some(chat) = chat() {
                            set_config(&state, &id, &chat, &[("reasoning", json!(level))]);
                        }
                    })
                })
                .collect();
            let label = reasoning_label.clone();
            let anchor = pill(
                text_signal(label).text_size(12.).text_color(MUTED),
                reasoning_open.clone(),
            );
            menu_panel(
                menu("Reasoning", reasoning_open.clone(), anchor),
                180.,
                count,
            )
            .children(entries)
        })
    };
    row()
        .gap(6.)
        .items_center()
        .child(model_picker)
        .child(reasoning_picker)
}
