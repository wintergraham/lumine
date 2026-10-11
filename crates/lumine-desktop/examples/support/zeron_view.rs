//! The transcript: user bubbles, streamed markdown under a fading veil, tool
//! groups with a connector tree, and entrance motion for rows that arrive
//! live. Layout and motion follow Zeron's `crates/ui/src/transcript.rs` and
//! `markdown/`; colours are white at graded alpha so glass shows through.
//!
//! Every animation reads one page clock (`Anim`) that ticks on display frames
//! only while something moves, so a settled transcript costs nothing.

use super::markdown::{self as md, Align, Change, Piece, Style, Token};
use super::model::{self, Block, PartKind, Tool};
use super::rpc::Rpc;
use serde_json::json;
use std::{
    cell::Cell, collections::HashMap, f32::consts::FRAC_PI_2, rc::Rc, sync::Arc, time::Instant,
};
use lumine::{
    affine::Affine,
    compose::{Tasks, prelude::*},
    cursor::Cursor,
    frame::FrameClock,
    reactive::Signal,
    rich_text::{RichText, TextMeasure, TextRun},
    scene::Color,
    svg::SvgData,
    text_layout::{FontFamily, FontStyle, LineHeight},
};

// Palette ---------------------------------------------------------------------

// Zeron's dark tokens (#e8e8ea / #a9a9ae / #85858a) as white at the alpha
// that reads the same over glass; accent and status colours as in Zeron.
pub const TEXT: Color = Color(255, 255, 255, 234);
pub const BODY: Color = Color(255, 255, 255, 226);
pub const MUTED: Color = Color(255, 255, 255, 160);
pub const FAINT: Color = Color(255, 255, 255, 122);
pub const HAIRLINE: Color = Color(255, 255, 255, 26);
pub const ACCENT: Color = Color(139, 124, 246, 255);
pub const DANGER: Color = Color(248, 113, 113, 255);
pub const ADDED: Color = Color(52, 211, 153, 255);

pub fn white(alpha: u8) -> Color {
    Color(255, 255, 255, alpha)
}

fn fade(color: Color, alpha: f32) -> Color {
    Color(
        color.0,
        color.1,
        color.2,
        (f32::from(color.3) * alpha.clamp(0., 1.)).round() as u8,
    )
}

fn mix(from: Color, to: Color, t: f32) -> Color {
    let lerp = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color(
        lerp(from.0, to.0),
        lerp(from.1, to.1),
        lerp(from.2, to.2),
        lerp(from.3, to.3),
    )
}

// Easing -------------------------------------------------------------------------

pub fn ease_out_expo(x: f32) -> f32 {
    let x = x.clamp(0., 1.);
    if x >= 1. {
        1.
    } else {
        1. - 2_f32.powf(-10. * x)
    }
}

fn ease_out_quint(x: f32) -> f32 {
    1. - (1. - x.clamp(0., 1.)).powi(5)
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0., 1.);
    x * x * (3. - 2. * x)
}

fn window(p: f32, start: f32, end: f32) -> f32 {
    ((p - start) / (end - start)).clamp(0., 1.)
}

/// A value easing from `from` to `target` starting at `at`.
#[derive(Clone, Copy, PartialEq)]
pub struct Tween {
    pub from: f32,
    pub target: f32,
    pub at: f32,
}

impl Tween {
    pub fn settled(value: f32) -> Self {
        Self {
            from: value,
            target: value,
            at: f32::NEG_INFINITY,
        }
    }
    pub fn value(self, now: f32, seconds: f32) -> f32 {
        self.from + (self.target - self.from) * ease_out_expo((now - self.at) / seconds)
    }
    pub fn toward(self, target: f32, now: f32, seconds: f32) -> Self {
        Self {
            from: self.value(now, seconds),
            target,
            at: now,
        }
    }
}

// Clock ------------------------------------------------------------------------

/// Looping ambient motion (shimmer, pulses) looks the same at this rate.
const AMBIENT_HZ: f64 = 30.;

/// The page clock: full rate until `until`, `AMBIENT_HZ` while any ambient
/// loop is registered, stopped otherwise. Views read it with `at`, which
/// stops tracking once their animation has settled.
#[derive(Clone)]
pub struct Anim {
    /// Full-rate time for fades and tweens.
    pub clock: Signal<f32>,
    /// Time for looping decoration (spinners, shimmer), stepped at
    /// `AMBIENT_HZ` even while `clock` runs at the display rate, so text
    /// streaming does not redraw every loader at 120 Hz.
    pub ambient_clock: Signal<f32>,
    origin: Instant,
    until: Rc<Cell<f32>>,
    ambient: Rc<Cell<u32>>,
    running: Rc<Cell<bool>>,
    tasks: Tasks,
    frames: FrameClock,
}

impl Anim {
    pub fn new(cx: &mut Context) -> Self {
        Self {
            clock: cx.state(0.),
            ambient_clock: cx.state(0.),
            origin: Instant::now(),
            until: Rc::new(Cell::new(0.)),
            ambient: Rc::new(Cell::new(0)),
            running: Rc::new(Cell::new(false)),
            tasks: cx.tasks(),
            frames: cx.frames(),
        }
    }
    pub fn now(&self) -> f32 {
        self.origin.elapsed().as_secs_f32()
    }
    /// Keep full-rate frames coming for `seconds` more.
    pub fn keep(&self, seconds: f32) {
        self.until.set(self.until.get().max(self.now() + seconds));
        self.run();
    }
    /// Register an ambient loop for as long as the guard lives.
    pub fn ambient(&self) -> AmbientGuard {
        self.ambient.set(self.ambient.get() + 1);
        self.run();
        AmbientGuard(self.ambient.clone())
    }
    fn run(&self) {
        if self.running.replace(true) {
            return;
        }
        let anim = self.clone();
        self.tasks.spawn(async move {
            loop {
                let full = anim.now() < anim.until.get();
                if !full && anim.ambient.get() == 0 {
                    break;
                }
                let frame = if full {
                    anim.frames.next().await
                } else {
                    anim.frames.next().max_rate(AMBIENT_HZ).await
                };
                // The frame's presentation time, not the wake-up time: steps
                // land a whole refresh apart however late the task runs.
                let at = frame
                    .time
                    .saturating_duration_since(anim.origin)
                    .as_secs_f32();
                let at = at.max(anim.clock.with_untracked(|c| *c));
                anim.clock.set(at);
                // A millisecond of slack keeps a 120 Hz display on every
                // fourth refresh rather than drifting to every fifth.
                let last = anim.ambient_clock.with_untracked(|c| *c);
                if at - last >= 1. / AMBIENT_HZ as f32 - 0.001 {
                    anim.ambient_clock.set(at);
                }
            }
            anim.running.set(false);
        });
    }
    /// The time, tracked until `end` and constant afterwards.
    pub fn at(&self, end: f32) -> f32 {
        let now = self.clock.with_untracked(|now| *now);
        if now >= end { now } else { self.clock.get() }
    }
}

pub struct AmbientGuard(Rc<Cell<u32>>);

impl Drop for AmbientGuard {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

// Icons ---------------------------------------------------------------------

const ICONS: [(&str, &[u8]); 12] = [
    ("chevron", include_bytes!("icons/alt-arrow-down.svg")),
    ("bot", include_bytes!("icons/bot.svg")),
    ("thought", include_bytes!("icons/chat-round-line.svg")),
    ("checklist", include_bytes!("icons/checklist.svg")),
    ("document-add", include_bytes!("icons/document-add.svg")),
    ("document", include_bytes!("icons/document.svg")),
    ("folder", include_bytes!("icons/folder-with-files.svg")),
    ("global", include_bytes!("icons/global.svg")),
    ("search", include_bytes!("icons/magnifer.svg")),
    ("pen", include_bytes!("icons/pen.svg")),
    ("terminal", include_bytes!("icons/terminal.svg")),
    ("widget", include_bytes!("icons/widget.svg")),
];

thread_local! {
    static ICON_DATA: HashMap<&'static str, SvgData> = ICONS
        .iter()
        .map(|(name, bytes)| (*name, SvgData::new(*bytes).expect("bundled icon")))
        .collect();
}

pub fn icon_data(name: &str) -> SvgData {
    ICON_DATA.with(|icons| {
        icons
            .get(name)
            .or_else(|| icons.get("widget"))
            .cloned()
            .expect("icon")
    })
}

pub fn icon(name: &str, size: f32, color: Color) -> View {
    svg(name, Arc::new(icon_data(name).tinted(color))).size(size, size)
}

// Transcript -------------------------------------------------------------------

/// What the transcript view needs from the app.
#[derive(Clone)]
pub struct Ctx {
    pub anim: Anim,
    /// `None` in the offline markdown preview.
    pub rpc: Option<Rpc>,
    pub transcript: model::Shared,
    pub tasks: Tasks,
    pub toggles: Toggles,
}

/// What the reader or the stream opened and closed, by part or entry, so a
/// row scrolled out of the virtualized transcript comes back as it was left.
#[derive(Clone, Default)]
pub struct Toggles(Rc<std::cell::RefCell<HashMap<String, bool>>>);

impl Toggles {
    fn get(&self, key: &str) -> Option<bool> {
        self.0.borrow().get(key).copied()
    }
    fn set(&self, key: &str, open: bool) {
        self.0.borrow_mut().insert(key.to_owned(), open);
    }
}

/// Zeron's spacing: 16 before a new turn, 12 around tool groups, 8 else.
fn gap(block: &Block, previous: Option<&Block>) -> f32 {
    match (block, previous) {
        (_, None) => 0.,
        (Block::User(_), _) => 22.,
        (Block::Group(_), _) | (_, Some(Block::Group(_))) => 10.,
        _ => 12.,
    }
}

/// A block's height in a `width` column before it is mounted: its text
/// measured as the view sets it (by the prepared text layout uses, so this
/// is line-breaking arithmetic once shaped) and the chrome around it. The
/// transcript replaces it with the measured height once the row mounts.
pub fn estimate(
    ctx: &Ctx,
    measure: &TextMeasure,
    block: &Block,
    previous: Option<&Block>,
    width: f32,
) -> f32 {
    let lines = |text: &str, size: f32, line: f32, width: f32| -> f32 {
        if text.is_empty() {
            return line;
        }
        let font = FontStyle {
            line_height: LineHeight::px(line),
            ..FontStyle::default()
        };
        let run = TextRun {
            range: 0..text.len(),
            font,
            font_size: size,
            ..Default::default()
        };
        RichText::new(text, vec![run])
            .map_or(line, |rich| measure.measure(&rich, Some(width.max(1.))).1)
    };
    let transcript = ctx.transcript.borrow();
    let body = match block {
        Block::User(entry) => {
            let text = transcript
                .user_text
                .get(entry)
                .map(|t| t.with_untracked(String::clone))
                .unwrap_or_default();
            let long = text.len() > 420 || text.lines().count() > 6;
            let open = ctx.toggles.get(&format!("user:{entry}")).unwrap_or(false);
            let body = lines(&text, 14., 22., width * 0.8 - 34.);
            let body = if long && !open {
                body.min(6. * 22.)
            } else {
                body
            };
            body + 22. + if long { 24. } else { 0. }
        }
        Block::Text(key) => transcript.parts.get(key).map_or(0., |part| {
            let parsed = part.text.with_untracked(|t| md::parse(t));
            let mut height = 0.;
            for (i, block) in parsed.iter().enumerate() {
                height += match (i.checked_sub(1).map(|p| kind(&parsed[p])), kind(block)) {
                    (None, _) => 0.,
                    (Some(1), 1) => 4.,
                    _ => 12.,
                };
                height += match block {
                    md::Block::Para(run) => lines(&run.text, 14., 22., width),
                    md::Block::Heading(level, run) => match level {
                        1 => lines(&run.text, 19., 27., width),
                        2 => lines(&run.text, 16., 24., width),
                        3 => lines(&run.text, 15., 22., width),
                        _ => lines(&run.text, 14., 22., width),
                    },
                    md::Block::Item { depth, run, .. } => {
                        lines(&run.text, 14., 22., width - *depth as f32 * 26. - 26.)
                    }
                    md::Block::Quote(run) => lines(&run.text, 14., 22., width - 24.) + 12.,
                    md::Block::Code { run, .. } => {
                        28. + run.text.lines().count().max(1) as f32 * 18. + 20.
                    }
                    md::Block::Table { rows, .. } => (rows.len() + 1) as f32 * 34.,
                    md::Block::Rule => 9.,
                };
            }
            height
        }),
        Block::Error(key) => transcript.parts.get(key).map_or(0., |part| {
            part.text
                .with_untracked(|text| lines(text, 13., 20., width - 60.))
                + 22.
        }),
        Block::Group(key) => {
            let members = transcript
                .groups
                .get(key)
                .map_or(0, |m| m.with_untracked(Vec::len));
            let open = ctx.toggles.get(&format!("group:{key}")).unwrap_or(false);
            26. + if open { members as f32 * 30. } else { 0. }
        }
    };
    gap(block, previous) + body
}

/// One transcript row, with Zeron's entrance: 500 ms expo, rising 4 px.
pub fn block(cx: &mut Context, ctx: &Ctx, block: &Block, previous: Option<Block>) -> View {
    let born = ctx.transcript.borrow().born(block);
    let gap = gap(block, previous.as_ref());
    let view = match block {
        Block::User(entry) => user_bubble(cx, ctx, entry),
        Block::Text(key) => match ctx.transcript.borrow().parts.get(key).cloned() {
            Some(part) => markdown(cx, ctx, part),
            None => div(),
        },
        Block::Error(key) => error_card(ctx, key),
        Block::Group(key) => match ctx.transcript.borrow().groups.get(key).cloned() {
            Some(members) => tool_group(cx, ctx, key, members),
            None => div(),
        },
    };
    let view = view.mt(gap);
    match born {
        // Remounted after its entrance (scrolled back into view): no motion.
        Some(born) if ctx.anim.now() < born + 0.5 => {
            ctx.anim.keep(0.6);
            let anim = ctx.anim.clone();
            view.reactive_style(move || {
                let p = ease_out_expo((anim.at(born + 0.5) - born) / 0.5);
                Styles::new().opacity(p).translate(0., 6. * (1. - p))
            })
        }
        _ => view,
    }
}

fn user_bubble(cx: &mut Context, ctx: &Ctx, entry: &str) -> View {
    let text_ = ctx.transcript.borrow().user_text.get(entry).cloned();
    let long = text_
        .as_ref()
        .is_some_and(|t| t.with_untracked(|t| t.len() > 420 || t.lines().count() > 6));
    let memory = format!("user:{entry}");
    let expanded = cx.state(ctx.toggles.get(&memory).unwrap_or(false));
    let body = {
        let expanded = expanded.clone();
        text_signal(move || text_.as_ref().map(Signal::get).unwrap_or_default())
            .w_full()
            .text_wrap(true)
            .text_size(14.)
            .line_height(22.)
            .text_color(TEXT)
            .reactive_style(move || {
                if long && !expanded.get() {
                    Styles::new()
                        .line_clamp(6)
                        .text_overflow(lumine::text_layout::TextOverflow::Ellipsis)
                } else {
                    Styles::new().line_clamp(0)
                }
            })
    };
    let more = {
        let (label, toggle) = (expanded.clone(), expanded.clone());
        row()
            .gap(4.)
            .items_center()
            .text_size(12.5)
            .text_color(MUTED)
            .cursor(Cursor::Pointer)
            .hover(|s| s.text_color(TEXT))
            .on_click({
                let toggles = ctx.toggles.clone();
                move || {
                    toggle.update(|e| *e = !*e);
                    toggles.set(&memory, toggle.with_untracked(|open| *open));
                }
            })
            .child(text_signal(move || {
                if label.get() {
                    "Show less".into()
                } else {
                    "Show more".into()
                }
            }))
            .when(!long, |v| v.hidden())
    };
    column().w_full().items_end().child(
        column()
            .max_w_percent(80.)
            .rounded(18.)
            .px(16.)
            .py(10.)
            .gap(6.)
            .bg(white(18))
            .border(1.)
            .border_color(white(14))
            .child(body)
            .child(more),
    )
}

fn error_card(ctx: &Ctx, key: &str) -> View {
    let message = ctx
        .transcript
        .borrow()
        .parts
        .get(key)
        .map(|p| p.text.clone());
    row()
        .w_full()
        .px(14.)
        .py(10.)
        .gap(10.)
        .rounded(12.)
        .bg(Color(255, 112, 114, 22))
        .border(1.)
        .border_color(Color(255, 112, 114, 64))
        .child(text("!").font_weight(700).text_size(13.).text_color(DANGER))
        .child(
            text_signal(move || message.as_ref().map(Signal::get).unwrap_or_default())
                .grow()
                .min_w(0.)
                .text_wrap(true)
                .text_size(13.)
                .line_height(20.)
                .text_color(fade(DANGER, 0.95)),
        )
}

// Markdown ---------------------------------------------------------------------

/// A streamed chunk: (source offset, arrival, fade seconds).
type Chunk = (usize, f32, f32);

/// Veil alpha for a source offset (Zeron's `veil.rs`): the chunk holding it
/// fades `1 - (1 - p)^1.6`, sped up by 30% per chunk beyond two still fading.
fn veil_alpha(chunks: &[Chunk], offset: usize, now: f32) -> f32 {
    let active = chunks
        .iter()
        .filter(|(_, born, fade)| now - born < *fade)
        .count();
    let boost = 1. + 0.3 * active.saturating_sub(2) as f32;
    match chunks.iter().rev().find(|(start, _, _)| *start <= offset) {
        Some((_, born, fade)) => {
            let p = ((now - born) * boost / fade).clamp(0., 1.);
            1. - (1. - p).powf(1.6)
        }
        None => 1.,
    }
}

/// Split `piece` at chunk boundaries and fade each part.
fn veiled(piece: &Piece, chunks: &[Chunk], now: f32, mut span: impl FnMut(String, f32)) {
    let end = piece.start + piece.text.len();
    let mut cuts: Vec<usize> = chunks
        .iter()
        .map(|(start, _, _)| *start)
        .filter(|start| *start > piece.start && *start < end)
        .collect();
    cuts.dedup();
    let mut from = piece.start;
    for cut in cuts.into_iter().chain([end]) {
        if cut > from
            && piece.text.is_char_boundary(from - piece.start)
            && piece.text.is_char_boundary(cut - piece.start)
        {
            span(
                piece.text[from - piece.start..cut - piece.start].to_owned(),
                veil_alpha(chunks, from, now),
            );
            from = cut;
        }
    }
}

fn styled_span(text: String, style: Style, base: Color, alpha: f32) -> TextSpan {
    let span = text_span(text);
    match style {
        Style::Plain => span.text_color(fade(base, alpha)),
        Style::Bold => span.text_color(fade(TEXT, alpha)).font_weight(600),
        Style::Italic => span.text_color(fade(base, alpha)).italic(true),
        Style::BoldItalic => span
            .text_color(fade(TEXT, alpha))
            .font_weight(600)
            .italic(true),
        Style::Strike => span
            .text_color(fade(MUTED, alpha))
            .strikethrough(lumine::rich_text::Decoration::new(1.)),
        // Accent on a 22% accent wash, as Zeron paints inline code.
        Style::Code => span
            .font_family(FontFamily::Monospace)
            .text_size(12.5)
            .text_color(fade(Color(171, 161, 249, 255), alpha))
            .background(fade(Color(139, 124, 246, 56), alpha)),
        // Links keep the text colour over a muted underline.
        Style::Link => span.text_color(fade(TEXT, alpha)).underline({
            let mut line = lumine::rich_text::Decoration::new(1.);
            line.color = Some(fade(MUTED, alpha));
            line
        }),
    }
}

/// Spans for a run, veiled while its chunks fade.
fn run_spans(run: &md::Run, base: Color, chunks: &[Chunk], now: f32) -> Vec<TextSpan> {
    let mut spans = Vec::new();
    for piece in md::inline(run) {
        veiled(&piece, chunks, now, |text, alpha| {
            spans.push(styled_span(text, piece.style, base, alpha))
        });
    }
    spans
}

/// The clock for text at `range` of a streaming part: tracked (so the view
/// redraws each frame) only while a chunk overlapping it is still fading.
/// Settled blocks of a long reply then recompute when text arrives, not on
/// every frame of every other block's fade.
fn veil_now(anim: &Anim, chunks: &[Chunk], range: std::ops::Range<usize>) -> f32 {
    let now = anim.clock.with_untracked(|now| *now);
    let fading = chunks
        .iter()
        .enumerate()
        .any(|(index, (start, born, fade))| {
            let end = chunks.get(index + 1).map_or(usize::MAX, |next| next.0);
            *start < range.end && end > range.start && now - born < *fade
        });
    if fading { anim.at(newest(chunks)) } else { now }
}
/// When the last chunk finishes fading: views track the clock until then.
fn newest(chunks: &[Chunk]) -> f32 {
    chunks
        .iter()
        .map(|(_, born, fade)| born + fade)
        .fold(f32::NEG_INFINITY, f32::max)
}

fn markdown(cx: &mut Context, ctx: &Ctx, part: Rc<model::Part>) -> View {
    let blocks = cx.state(Vec::<md::Block>::new());
    {
        let (blocks, text_) = (blocks.clone(), part.text.clone());
        let effect = cx.runtime().effect(move || {
            let parsed = text_.with(|t| md::parse(t));
            blocks.set(parsed);
        });
        cx.retain(effect);
    }
    // A block that changes kind while streaming (a line that turns out to
    // start a table) gets a fresh view: key by index and kind.
    let keys = blocks.clone();
    let (ctx, part) = (ctx.clone(), part.clone());
    keyed(
        move || {
            keys.with(|b| {
                b.iter()
                    .enumerate()
                    .map(|(i, block)| (i, kind(block), i.checked_sub(1).map(|p| kind(&b[p]))))
                    .collect::<Vec<_>>()
            })
        },
        move |(index, this, before), cx| {
            // Zeron: 12 px between blocks, 4 px between list items.
            let gap = match before {
                None => 0.,
                Some(1) if this == 1 => 4.,
                Some(_) => 12.,
            };
            md_block(cx, &ctx, &part, &blocks, index).mt(gap)
        },
    )
    .w_full()
}

fn kind(block: &md::Block) -> u8 {
    match block {
        md::Block::Para(_) => 0,
        md::Block::Heading(level, _) => 10 + level,
        md::Block::Item { .. } => 1,
        md::Block::Quote(_) => 2,
        md::Block::Code { .. } => 3,
        md::Block::Table { .. } => 4,
        md::Block::Rule => 5,
    }
}

fn md_block(
    cx: &mut Context,
    ctx: &Ctx,
    part: &Rc<model::Part>,
    blocks: &Signal<Vec<md::Block>>,
    index: usize,
) -> View {
    let this = {
        let blocks = blocks.clone();
        move || blocks.with(|b| b.get(index).cloned())
    };
    let chunks = part.chunks.clone();
    let anim = ctx.anim.clone();
    let spans_of = {
        let (chunks, anim) = (chunks.clone(), anim.clone());
        move |run: &md::Run, base: Color| {
            chunks.with(|c| {
                let now = veil_now(&anim, c, run.start..run.start + run.text.len());
                run_spans(run, base, c, now)
            })
        }
    };
    match this() {
        Some(md::Block::Code { lang, .. }) => code_block(cx, ctx, lang, this, chunks),
        Some(md::Block::Table { .. }) => table(cx, this, spans_of),
        Some(md::Block::Rule) => div().w_full().h(1.).my(4.).bg(HAIRLINE),
        Some(md::Block::Heading(level, _)) => rich_text_signal(move || match this() {
            Some(md::Block::Heading(_, run)) => spans_of(&run, TEXT),
            _ => Vec::new(),
        })
        .w_full()
        .text_wrap(true)
        .text_size(match level {
            1 => 19.,
            2 => 16.,
            3 => 15.,
            _ => 14.,
        })
        .line_height(match level {
            1 => 27.,
            2 => 24.,
            _ => 22.,
        })
        .font_weight(600)
        .letter_spacing(if level == 1 { -0.2 } else { 0. }),
        Some(md::Block::Item {
            depth,
            marker,
            task,
            ..
        }) => {
            let body = rich_text_signal({
                let spans_of = spans_of.clone();
                move || match this() {
                    Some(md::Block::Item { run, .. }) => spans_of(&run, BODY),
                    _ => Vec::new(),
                }
            })
            .w(0.)
            .grow()
            .text_wrap(true)
            .text_size(14.)
            .line_height(22.);
            // A 5 px accent disc, an accent "N." or a checkbox, in an
            // 18 px column centred on the first line.
            let marker_view = match task {
                Some(done) => row()
                    .size(15., 15.)
                    .mt(3.5)
                    .rounded(3.)
                    .items_center()
                    .justify_center()
                    .border(1.)
                    .border_color(if done { ACCENT } else { white(56) })
                    .bg(if done { ACCENT } else { white(0) })
                    .child(
                        text(if done { "✓" } else { "" })
                            .text_size(10.)
                            .font_weight(700)
                            .text_color(Color(6, 6, 6, 255)),
                    ),
                None if marker == "•" => row()
                    .w(18.)
                    .h(22.)
                    .shrink_0()
                    .items_center()
                    .child(div().size(5., 5.).ml(1.).rounded(2.5).bg(ACCENT)),
                None => text(marker)
                    .min_w(18.)
                    .shrink_0()
                    .text_size(14.)
                    .line_height(22.)
                    .text_color(ACCENT),
            };
            row()
                .w_full()
                .gap(8.)
                .items_start()
                .pl(depth as f32 * 26.)
                .child(marker_view)
                .child(body)
        }
        Some(md::Block::Quote(_)) => row()
            .w_full()
            .rounded_corners(lumine::decoration::Corners {
                top_left: 0.,
                top_right: 6.,
                bottom_right: 6.,
                bottom_left: 0.,
            })
            .bg(Color(139, 124, 246, 13))
            .border_edges(lumine::scene::Insets {
                top: 0.,
                right: 0.,
                bottom: 0.,
                left: 2.,
            })
            .border_color(Color(139, 124, 246, 153))
            .pl(12.)
            .pr(10.)
            .py(6.)
            .child(
                rich_text_signal(move || match this() {
                    Some(md::Block::Quote(run)) => spans_of(&run, MUTED),
                    _ => Vec::new(),
                })
                .w_full()
                .text_wrap(true)
                .text_size(14.)
                .line_height(22.),
            ),
        _ => rich_text_signal(move || match this() {
            Some(md::Block::Para(run)) => spans_of(&run, BODY),
            _ => Vec::new(),
        })
        .w_full()
        .text_wrap(true)
        .text_size(14.)
        .line_height(22.),
    }
}

/// Zeron's runtime syntax palette.
fn token_color(token: Token) -> Color {
    match token {
        Token::Text => Color(232, 232, 234, 255),
        Token::Keyword => Color(139, 124, 246, 255),
        Token::String => Color(52, 211, 153, 255),
        Token::Number => Color(250, 204, 21, 255),
        Token::Comment => Color(146, 146, 154, 255),
        Token::Function => Color(96, 165, 250, 255),
        Token::Type => Color(192, 132, 252, 255),
        Token::Punct => Color(161, 161, 170, 255),
    }
}

fn code_block(
    cx: &mut Context,
    ctx: &Ctx,
    lang: String,
    this: impl Fn() -> Option<md::Block> + Clone + 'static,
    chunks: Signal<Vec<Chunk>>,
) -> View {
    const LINE: f32 = 18.;
    let copied = cx.state(false);
    let anim = ctx.anim.clone();
    let code = {
        let (this, lang) = (this.clone(), lang.clone());
        rich_text_signal(move || {
            let Some(md::Block::Code { run, .. }) = this() else {
                return Vec::new();
            };
            let now = chunks.with(|c| veil_now(&anim, c, run.start..run.start + run.text.len()));
            let mut spans = Vec::new();
            chunks.with(|chunks| {
                for (range, token) in md::highlight(&run.text, &lang) {
                    let piece = Piece {
                        text: run.text[range.clone()].to_owned(),
                        start: run.start + range.start,
                        style: Style::Plain,
                    };
                    veiled(&piece, chunks, now, |text, alpha| {
                        spans.push(text_span(text).text_color(fade(token_color(token), alpha)));
                    });
                }
            });
            spans
        })
        .text_wrap(false)
        .font_family(FontFamily::Monospace)
        .text_size(12.5)
        .line_height(LINE)
    };
    let offset = cx.state(0_f32);
    let height_of = this.clone();
    let copy = {
        let (this, copied) = (this.clone(), copied.clone());
        let tasks = ctx.tasks.clone();
        move || {
            if let Some(md::Block::Code { run, .. }) = this()
                && let Ok(mut clipboard) = arboard::Clipboard::new()
                && clipboard.set_text(run.text).is_ok()
            {
                copied.set(true);
                let copied = copied.clone();
                tasks.spawn(async move {
                    lumine::timer::sleep(std::time::Duration::from_millis(1400)).await;
                    copied.set(false);
                });
            }
        }
    };
    column()
        .w_full()
        .rounded(10.)
        .bg(white(9))
        .border(1.)
        .border_color(white(26))
        .overflow_hidden()
        .child(
            row()
                .w_full()
                .h(28.)
                .pl(12.)
                .pr(5.)
                .items_center()
                .justify_between()
                .bg(white(5))
                .border_edges(lumine::scene::Insets {
                    top: 0.,
                    right: 0.,
                    bottom: 1.,
                    left: 0.,
                })
                .border_color(white(26))
                .child(text(lang).text_size(11.).text_color(MUTED))
                .child(
                    row()
                        .h(22.)
                        .px(6.)
                        .gap(4.)
                        .rounded(5.)
                        .items_center()
                        .cursor(Cursor::Pointer)
                        .text_size(10.5)
                        .text_color(MUTED)
                        .hover(|s| s.bg(white(20)).text_color(TEXT))
                        .on_click(copy)
                        .child(text_signal(move || {
                            if copied.get() {
                                "✓ Copied".into()
                            } else {
                                "Copy".into()
                            }
                        })),
                ),
        )
        .child(
            // Scroll views have a default height: size this one to its lines.
            scroll_x(offset)
                .w_full()
                .reactive_style(move || {
                    let lines = match height_of() {
                        Some(md::Block::Code { run, .. }) => {
                            run.text.trim_end_matches('\n').lines().count().max(1)
                        }
                        _ => 1,
                    };
                    Styles::new().h(lines as f32 * LINE + 20.)
                })
                .child(column().px(12.).py(10.).child(code)),
        )
}

fn row_view() -> View {
    row()
}

/// Zeron's column sizing: each column's natural width is max(content, 48)
/// + 24; widths share the table in that proportion.
fn column_weights(header: &[md::Run], rows: &[Vec<md::Run>]) -> Vec<f32> {
    (0..header.len())
        .map(|i| {
            let longest = std::iter::once(header.get(i))
                .chain(rows.iter().map(|row| row.get(i)))
                .flatten()
                .map(|run| run.text.chars().count())
                .max()
                .unwrap_or(4);
            (longest as f32 * 7.5).clamp(48., 420.) + 24.
        })
        .collect()
}

fn table(
    cx: &mut Context,
    this: impl Fn() -> Option<md::Block> + Clone + 'static,
    spans_of: impl Fn(&md::Run, Color) -> Vec<TextSpan> + Clone + 'static,
) -> View {
    // Rebuilt when the column count or widths change as the table streams.
    let shape = {
        let this = this.clone();
        move || match this() {
            Some(md::Block::Table {
                aligns,
                header,
                rows,
            }) => Some((
                aligns,
                column_weights(&header, &rows)
                    .iter()
                    .map(|w| *w as u32)
                    .collect::<Vec<_>>(),
            )),
            _ => None,
        }
    };
    let _ = cx;
    switch(shape, move |shape, _| {
        let Some((aligns, weights)) = shape else {
            return div();
        };
        let columns = weights.len();
        let row_count = {
            let this = this.clone();
            move || match this() {
                Some(md::Block::Table { rows, .. }) => rows.len(),
                _ => 0,
            }
        };
        let cell = {
            let (this, spans_of) = (this.clone(), spans_of.clone());
            move |row: Option<usize>, column: usize| {
                let (this, spans_of) = (this.clone(), spans_of.clone());
                let align = aligns.get(column).copied().unwrap_or(Align::Left);
                // Alignment by the cell's justification: the text keeps its
                // natural width inside it, wrapping only when too long.
                let text_ = rich_text_signal(move || {
                    let Some(md::Block::Table { header, rows, .. }) = this() else {
                        return Vec::new();
                    };
                    let run = match row {
                        None => header.get(column).cloned(),
                        Some(row) => rows.get(row).and_then(|r| r.get(column)).cloned(),
                    };
                    run.map_or_else(Vec::new, |run| {
                        spans_of(&run, if row.is_none() { TEXT } else { BODY })
                    })
                })
                .min_w(0.)
                .text_wrap(true)
                .text_size(14.)
                .line_height(22.)
                .when(row.is_none(), |c| c.font_weight(700));
                let cell = row_view()
                    .w(0.)
                    .flex_grow(weights.get(column).copied().unwrap_or(72) as f32)
                    .p(12.)
                    .child(text_);
                match align {
                    Align::Left => cell.justify_start(),
                    Align::Center => cell.justify_center(),
                    Align::Right => cell.justify_end(),
                }
            }
        };
        // Frameless, as in Zeron: only 1 px rules between rows.
        let rule = || lumine::scene::Insets {
            top: 0.,
            right: 0.,
            bottom: 1.,
            left: 0.,
        };
        let header_row = {
            let cell = cell.clone();
            row()
                .w_full()
                .border_edges(rule())
                .border_color(white(26))
                .children((0..columns).map(move |c| cell(None, c)))
        };
        let body = keyed(
            move || (0..row_count()).collect::<Vec<_>>(),
            move |r, _| {
                let cell = cell.clone();
                row()
                    .w_full()
                    .border_edges(rule())
                    .border_color(white(20))
                    .children((0..columns).map(move |c| cell(Some(r), c)))
            },
        )
        .w_full();
        column().w_full().child(header_row).child(body)
    })
    .w_full()
}

// Tool groups ----------------------------------------------------------------

fn summary(tools: &[(PartKind, Tool)]) -> String {
    let count = |verbs: &[&str]| {
        tools
            .iter()
            .filter(|(_, t)| verbs.contains(&t.verb))
            .count()
    };
    let thoughts = tools
        .iter()
        .filter(|(k, _)| *k == PartKind::Reasoning)
        .count();
    let failed = tools.iter().filter(|(_, t)| t.error).count();
    let mut parts = Vec::new();
    let mut add = |n: usize, one: &str, many: &str| {
        if n == 1 {
            parts.push(one.to_owned());
        } else if n > 1 {
            parts.push(many.replace('N', &n.to_string()));
        }
    };
    add(thoughts, "thought process", "thought N times");
    add(count(&["Run"]), "ran 1 command", "ran N commands");
    add(
        count(&["Edit", "Write", "Patch"]),
        "edited 1 file",
        "edited N files",
    );
    add(count(&["Read"]), "read 1 file", "read N files");
    add(
        count(&["Search", "Glob"]),
        "searched once",
        "searched N times",
    );
    add(
        count(&["Fetch", "Web"]),
        "fetched 1 page",
        "fetched N pages",
    );
    add(count(&["Agent"]), "ran 1 agent", "ran N agents");
    add(count(&["Todo"]), "updated todos", "updated todos");
    add(count(&["Tool", "MCP"]), "used 1 tool", "used N tools");
    add(failed, "1 failed", "N failed");
    let mut text = parts.join(" · ");
    if let Some(first) = text.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    text
}

/// A group is live while a tool runs or its thought is still streaming.
fn group_running(ctx: &Ctx, members: &Signal<Vec<String>>) -> bool {
    let transcript = ctx.transcript.borrow();
    let streaming = transcript.streaming.get();
    let last_block = transcript.blocks.with(|b| b.last().cloned());
    members.with(|keys| {
        let open_tool = keys.iter().any(|key| {
            transcript
                .parts
                .get(key)
                .is_some_and(|p| p.kind == PartKind::Tool && !p.tool.with(|t| t.resolved))
        });
        // The newest group of a streaming turn is still gathering rows.
        let newest = matches!(&last_block, Some(Block::Group(key)) if keys.first() == Some(key));
        open_tool || (streaming && newest)
    })
}

fn tool_group(cx: &mut Context, ctx: &Ctx, key: &str, members: Signal<Vec<String>>) -> View {
    // Live groups start open; history arrives folded, as in Zeron.
    let live = group_running(ctx, &members);
    let memory = format!("group:{key}");
    let remembered = ctx.toggles.get(&memory);
    let open = cx.state(Tween::settled(if remembered.unwrap_or(live) {
        1.
    } else {
        0.
    }));
    let user_toggled = Rc::new(Cell::new(remembered.is_some()));
    // Fold once the group finishes, unless the reader opened it.
    {
        let (ctx2, members2, open2, toggled, memory2) = (
            ctx.clone(),
            members.clone(),
            open.clone(),
            user_toggled.clone(),
            memory.clone(),
        );
        let was_running = Cell::new(live);
        let effect = cx.runtime().effect(move || {
            let running = group_running(&ctx2, &members2);
            if was_running.replace(running) && !running && !toggled.get() {
                let now = ctx2.anim.now();
                open2.update(|t| *t = t.toward(0., now, 0.22));
                ctx2.toggles.set(&memory2, false);
                ctx2.anim.keep(0.3);
            }
        });
        cx.retain(effect);
    }
    // The title shimmers while the group runs.
    let shimmer = Rc::new(std::cell::RefCell::new(None::<AmbientGuard>));
    let title = {
        let (ctx, members) = (ctx.clone(), members.clone());
        let shimmer = shimmer.clone();
        rich_text_signal(move || {
            let tools: Vec<_> = members.with(|keys| {
                let transcript = ctx.transcript.borrow();
                keys.iter()
                    .filter_map(|key| transcript.parts.get(key))
                    .map(|p| (p.kind, p.tool.get()))
                    .collect()
            });
            let text_ = summary(&tools);
            let running = group_running(&ctx, &members);
            {
                let mut guard = shimmer.borrow_mut();
                match (running, guard.is_some()) {
                    (true, false) => *guard = Some(ctx.anim.ambient()),
                    (false, true) => *guard = None,
                    _ => {}
                }
            }
            if !running {
                return vec![text_span(text_)];
            }
            // Zeron's 3.4 s sweep: a soft peak of full white across muted.
            let phase = (ctx.anim.ambient_clock.get() / 3.4).rem_euclid(1.);
            let centre = -0.4 + 1.8 * phase;
            let last = (text_.chars().count().max(2) - 1) as f32;
            text_
                .chars()
                .enumerate()
                .map(|(i, ch)| {
                    let glow = smoothstep(1. - (i as f32 / last - centre).abs() / 0.36);
                    text_span(ch.to_string()).text_color(mix(MUTED, TEXT, glow))
                })
                .collect()
        })
    };
    let chevron = {
        let (open, anim) = (open.clone(), ctx.anim.clone());
        svg_signal("toggle", move || {
            let tween = open.get();
            let value = tween.value(anim.at(tween.at + 0.22), 0.22);
            Arc::new(
                icon_data("chevron")
                    .tinted(MUTED)
                    .transformed(Affine::rotation(-FRAC_PI_2 * (1. - value))),
            )
        })
        .size(14., 14.)
    };
    let rows = {
        let (ctx, members, open) = (ctx.clone(), members.clone(), open.clone());
        let (keys, opened_by) = (members.clone(), open.clone());
        // Rows mount on first open: folded history costs one header each.
        let opened = Cell::new(false);
        keyed(
            move || {
                if opened_by.with(|t| t.target > 0.5) {
                    opened.set(true);
                }
                if opened.get() { keys.get() } else { Vec::new() }
            },
            move |key, cx| {
                // Look the part up now: groups outlive the parts that exist
                // when they are built.
                let part = ctx.transcript.borrow().parts.get(&key).cloned();
                match part {
                    Some(part) => {
                        let (members, this) = (members.clone(), key.clone());
                        let position = move || {
                            members.with(|m| (m.first() == Some(&this), m.last() == Some(&this)))
                        };
                        tool_row(cx, &ctx, &key, part, position, open.clone())
                    }
                    None => div(),
                }
            },
        )
        .w_full()
    };
    let toggle = {
        let (open, anim, toggles) = (open.clone(), ctx.anim.clone(), ctx.toggles.clone());
        move || {
            user_toggled.set(true);
            let now = anim.now();
            open.update(|t| {
                let target = if t.target > 0.5 { 0. } else { 1. };
                *t = t.toward(target, now, 0.22);
                toggles.set(&memory, target > 0.5);
            });
            anim.keep(0.3);
        }
    };
    column()
        .w_full()
        .child(
            row()
                .h(26.)
                .gap(6.)
                .pr(6.)
                .items_center()
                .text_size(12.5)
                .line_height(18.)
                .text_color(MUTED)
                .hover(|s| s.text_color(TEXT))
                .cursor(Cursor::Pointer)
                .on_click(toggle)
                .child(
                    row()
                        .size(22., 18.)
                        .items_center()
                        .justify_center()
                        .child(chevron),
                )
                .child(title),
        )
        .child(column().w_full().pt(2.).child(rows))
}

/// Lines a row shows when expanded: output, a diff, or a thought.
fn detail_lines(
    tool: &Tool,
    thought: Option<&str>,
    fetched: Option<&str>,
) -> Vec<(String, Color, Option<Color>)> {
    const CAP: usize = 40;
    if let Some(thought) = thought {
        return wrap(thought, 92)
            .into_iter()
            .map(|l| (l, FAINT, None))
            .collect();
    }
    let diff = match (fetched, &tool.diff) {
        (Some(json), _) if tool.diff_ref.is_some() && json.trim_start().starts_with('{') => {
            serde_json::from_str::<serde_json::Value>(json)
                .ok()
                .and_then(|v| {
                    Some((
                        v.get("oldText").and_then(|t| t.as_str()).map(str::to_owned),
                        v.get("newText")?.as_str()?.to_owned(),
                    ))
                })
        }
        (_, Some((_, old, new))) => Some((old.clone(), new.clone())),
        _ => None,
    };
    if let Some((old, new)) = diff {
        let mut lines: Vec<_> = md::line_diff(old.as_deref().unwrap_or(""), &new, 2)
            .into_iter()
            .map(|line| match line {
                None => ("⋯".to_owned(), FAINT, None),
                Some((Change::Added, l)) => (
                    format!("+ {l}"),
                    Color(120, 230, 170, 255),
                    Some(Color(64, 214, 150, 26)),
                ),
                Some((Change::Removed, l)) => (
                    format!("− {l}"),
                    Color(255, 150, 150, 255),
                    Some(Color(255, 112, 114, 24)),
                ),
                Some((Change::Same, l)) => (format!("  {l}"), MUTED, None),
            })
            .collect();
        if lines.len() > CAP {
            let more = lines.len() - CAP;
            lines.truncate(CAP);
            lines.push((format!("… {more} more lines"), FAINT, None));
        }
        return lines;
    }
    let source = fetched.unwrap_or(&tool.output);
    let mut lines: Vec<_> = source
        .lines()
        .map(|l| {
            let color = if l.contains("error") || l.contains("FAILED") || l.contains("panicked") {
                fade(DANGER, 0.9)
            } else {
                MUTED
            };
            (l.to_owned(), color, None)
        })
        .collect();
    while lines.last().is_some_and(|(l, _, _)| l.trim().is_empty()) {
        lines.pop();
    }
    if lines.len() > CAP {
        let more = lines.len() - CAP;
        lines.truncate(CAP);
        lines.push((format!("… {more} more lines"), FAINT, None));
    }
    lines
}

fn wrap(text: &str, columns: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > columns {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines
}

fn tool_row(
    cx: &mut Context,
    ctx: &Ctx,
    key: &str,
    part: Rc<model::Part>,
    position: impl Fn() -> (bool, bool) + Clone + 'static,
    group_open: Signal<Tween>,
) -> View {
    const LINE: f32 = 18.;
    let think = part.kind == PartKind::Reasoning;
    let born = part.born;
    let anim = ctx.anim.clone();
    // Thoughts stream open and fold when their turn moves on.
    let memory = format!("tool:{key}");
    let expanded = cx.state(Tween::settled(
        if ctx.toggles.get(&memory).unwrap_or(think && born.is_some()) {
            1.
        } else {
            0.
        },
    ));
    let fetched = cx.state(None::<String>);
    if born.is_some_and(|born| anim.now() < born + 0.6) {
        anim.keep(0.7);
    }
    let tool = part.tool.clone();
    let tint = {
        let tool = tool.clone();
        move || {
            if tool.with(|t| t.error) {
                DANGER
            } else {
                MUTED
            }
        }
    };
    // Zeron's row entrance: height over 360 ms expo, connector 480 ms quint,
    // content rising as the branch draws.
    let timeline = {
        let anim = anim.clone();
        move || match born {
            Some(born) => ease_out_quint((anim.at(born + 0.5) - born) / 0.48),
            None => 1.,
        }
    };
    let row_style = {
        let (anim, group_open, tint, timeline) = (
            anim.clone(),
            group_open.clone(),
            tint.clone(),
            timeline.clone(),
        );
        move || {
            let grow = match born {
                Some(born) => ease_out_expo((anim.at(born + 0.4) - born) / 0.36),
                None => 1.,
            };
            let _ = &timeline;
            let fold = group_open.get();
            let open = fold.value(anim.at(fold.at + 0.22), 0.22);
            Styles::new()
                .h(32. * grow * open)
                .opacity(open)
                .text_color(tint())
        }
    };
    let (line_span, branch_span) = ((0.2, 0.62), (0.55, 1.));
    let line = {
        let (position, timeline) = (position.clone(), timeline.clone());
        div().w(1.).bg(white(34)).reactive_style(move || {
            let (_, last) = position();
            let p = window(timeline(), line_span.0, line_span.1);
            Styles::new().h(if last { 16. } else { 32. } * p)
        })
    };
    let branch = {
        let timeline = timeline.clone();
        div().h(1.).bg(white(34)).reactive_style(move || {
            Styles::new().w(14. * window(timeline(), branch_span.0, branch_span.1))
        })
    };
    let content_style = {
        let timeline = timeline.clone();
        move || {
            let p = window(timeline(), branch_span.0, branch_span.1);
            Styles::new().opacity(p).translate(0., 4. * (1. - p))
        }
    };
    let icon_view = {
        let (tool, tint) = (tool.clone(), tint.clone());
        svg_signal("tool", move || {
            let name = if think {
                "thought"
            } else {
                tool.with(|t| t.icon)
            };
            Arc::new(icon_data(name).tinted(tint()))
        })
        .size(15., 15.)
        .ml(5.)
        .mt(8.5)
    };
    let label = {
        let (tool, text_, streaming) = (
            tool.clone(),
            part.text.clone(),
            ctx.transcript.borrow().streaming.clone(),
        );
        text_signal(move || {
            if think {
                let live = streaming.get() && text_.with(|t| !t.is_empty()) && born.is_some();
                if live {
                    "Thinking".into()
                } else {
                    "Thought process".into()
                }
            } else {
                tool.with(|t| t.verb.to_owned())
            }
        })
        .shrink_0()
    };
    let detail = {
        let (tool, badge) = (tool.clone(), tool.clone());
        row()
            .min_w(0.)
            .gap(6.)
            .items_center()
            .child(
                text_signal(move || {
                    tool.with(|t| {
                        let detail = if t.file {
                            t.detail.rsplit('/').next().unwrap_or(&t.detail)
                        } else {
                            &t.detail
                        };
                        match &t.subagent_tail {
                            Some(tail) if !t.resolved => format!("{detail} — {tail}"),
                            _ => detail.to_owned(),
                        }
                    })
                })
                .truncate()
                .min_w(0.),
            )
            .reactive_style(move || {
                if badge.with(|t| t.file) {
                    Styles::new()
                        .h(22.)
                        .pl(7.)
                        .pr(8.)
                        .rounded(6.)
                        .bg(white(14))
                        .border(1.)
                        .border_color(white(12))
                } else {
                    Styles::new().h(22.).px(0.).bg(white(0)).border(0.)
                }
            })
            .when(think, |d| d.hidden())
    };
    let stats = {
        let tool = tool.clone();
        rich_text_signal(move || match tool.with(|t| t.stats) {
            Some((added, removed)) => vec![
                text_span(format!("+{added} ")).text_color(ADDED),
                text_span(format!("−{removed}")).text_color(DANGER),
            ],
            None => Vec::new(),
        })
        .shrink_0()
        .text_size(11.5)
        .font_family(FontFamily::Monospace)
    };
    let running = {
        let tool = tool.clone();
        div()
            .size(6., 6.)
            .rounded(3.)
            .bg(ACCENT)
            .shrink_0()
            .reactive_style(move || {
                if !think && !tool.with(|t| t.resolved) {
                    Styles::new().flex()
                } else {
                    Styles::new().hidden()
                }
            })
    };
    let lines = {
        let (tool, text_, fetched) = (tool.clone(), part.text.clone(), fetched.clone());
        move || {
            let thought = think.then(|| text_.get());
            fetched.with(|f| tool.with(|t| detail_lines(t, thought.as_deref(), f.as_deref())))
        }
    };
    let body = {
        let lines = lines.clone();
        let (anim, expanded, group_open) = (anim.clone(), expanded.clone(), group_open.clone());
        let text_view = rich_text_signal({
            let lines = lines.clone();
            move || {
                let lines = lines();
                let count = lines.len();
                lines
                    .into_iter()
                    .enumerate()
                    .map(|(i, (line, color, background))| {
                        let text_ = if i + 1 < count { line + "\n" } else { line };
                        let span = text_span(text_).text_color(color);
                        match background {
                            Some(bg) => span.background(bg),
                            None => span,
                        }
                    })
                    .collect()
            }
        })
        .text_wrap(false)
        .text_size(12.)
        .line_height(LINE)
        .when(!think, |t| t.font_family(FontFamily::Monospace));
        column()
            .w_full()
            .pl(56.)
            .pr(8.)
            .overflow_hidden()
            .reactive_style(move || {
                let open = expanded.get();
                let fold = group_open.get();
                let p = open.value(anim.at(open.at + 0.2), 0.2)
                    * fold.value(anim.at(fold.at + 0.22), 0.22);
                let count = lines().len();
                if p <= 0.001 || count == 0 {
                    return Styles::new().hidden();
                }
                let full = count as f32 * LINE + 14.;
                Styles::new().flex().h(full * p).opacity(p.powf(0.6))
            })
            .child(column().w_full().py(6.).child(text_view))
    };
    // Close a streaming thought once the turn moves past it.
    if think && born.is_some() {
        let (expanded, anim, streaming, toggles, memory) = (
            expanded.clone(),
            anim.clone(),
            ctx.transcript.borrow().streaming.clone(),
            ctx.toggles.clone(),
            memory.clone(),
        );
        let was = Cell::new(streaming.with_untracked(|live| *live));
        let effect = cx.runtime().effect(move || {
            let live = streaming.get();
            if was.replace(live) && !live {
                let now = anim.now();
                expanded.update(|t| *t = t.toward(0., now, 0.2));
                toggles.set(&memory, false);
                anim.keep(0.3);
            }
        });
        cx.retain(effect);
    }
    let toggle = {
        let (expanded, anim, tool, fetched, rpc, tasks, toggles) = (
            expanded.clone(),
            anim.clone(),
            tool.clone(),
            fetched.clone(),
            ctx.rpc.clone(),
            ctx.tasks.clone(),
            ctx.toggles.clone(),
        );
        move || {
            let now = anim.now();
            let opening = expanded.with_untracked(|t| t.target < 0.5);
            expanded.update(|t| *t = t.toward(if opening { 1. } else { 0. }, now, 0.2));
            toggles.set(&memory, opening);
            anim.keep(0.3);
            // Full output or diff, fetched once on first open.
            let blob = tool.with_untracked(|t| t.diff_ref.clone().or_else(|| t.output_ref.clone()));
            if opening
                && fetched.with_untracked(Option::is_none)
                && let Some(blob) = blob
                && let Some(rpc) = &rpc
            {
                let (rpc, fetched) = (rpc.clone(), fetched.clone());
                tasks.spawn(async move {
                    if let Ok(reply) = rpc.call("FetchToolBlob", json!({ "blobRef": blob })).await
                        && let Some(text_) = reply.get("text").and_then(|t| t.as_str())
                    {
                        fetched.set(Some(text_.to_owned()));
                    }
                });
            }
        }
    };
    column()
        .w_full()
        .child(
            row()
                .w_full()
                .overflow_hidden()
                .items_start()
                .text_size(12.5)
                .line_height(18.)
                .hover(|s| s.text_color(TEXT))
                .cursor(Cursor::Pointer)
                .reactive_style(row_style)
                .on_click(toggle)
                .child(
                    row()
                        .w(48.)
                        .h(32.)
                        .shrink_0()
                        .items_start()
                        .child(div().w(10.))
                        .child(line)
                        .child(column().w(14.).pt(16.).child(branch))
                        .child(icon_view),
                )
                .child(
                    row()
                        .grow()
                        .min_w(0.)
                        .h(32.)
                        .ml(8.)
                        .gap(8.)
                        .items_center()
                        .overflow_hidden()
                        .reactive_style(content_style)
                        .child(label)
                        .child(detail)
                        .child(stats)
                        .child(running),
                ),
        )
        .child(body)
}

// Working trailer --------------------------------------------------------------

const WORDS: [&str; 12] = [
    "Thinking",
    "Reading",
    "Tracing",
    "Untangling",
    "Pondering",
    "Weaving",
    "Wrangling",
    "Mulling",
    "Sifting",
    "Tinkering",
    "Composing",
    "Assembling",
];

/// "Thinking… 12s" under a live turn, with a pulsing trio of dots.
pub fn trailer(
    cx: &mut Context,
    anim: &Anim,
    working: impl Fn() -> bool + Clone + 'static,
) -> View {
    let started = cx.state(None::<f32>);
    let guard = Rc::new(std::cell::RefCell::new(None::<AmbientGuard>));
    {
        let (anim, started, working, guard) = (
            anim.clone(),
            started.clone(),
            working.clone(),
            guard.clone(),
        );
        let effect = cx.runtime().effect(move || {
            let live = working();
            let mut slot = guard.borrow_mut();
            if live && slot.is_none() {
                *slot = Some(anim.ambient());
                started.set(Some(anim.now()));
            } else if !live && slot.is_some() {
                *slot = None;
                started.set(None);
            }
        });
        cx.retain(effect);
    }
    // Zeron's gradient spinner: 3×3 dots, rows pale blue, peach, pink; each
    // dips to 10% over 45% of a 750 ms cycle, with the wave climbing upward.
    let spinner = column().gap(1.25).children((0..3).map(|row_| {
        let anim = anim.clone();
        row().gap(1.25).children((0..3).map(move |col| {
            let anim = anim.clone();
            let colour = [
                Color(182, 211, 239, 255),
                Color(237, 177, 133, 255),
                Color(248, 136, 160, 255),
            ][row_];
            let phase = ((2 - row_) as f32 + (col as f32 - 1.).abs()) / 4.;
            div()
                .size(2.5, 2.5)
                .rounded(1.25)
                .bg(colour)
                .reactive_style(move || {
                    let t = (anim.ambient_clock.get() / 0.75 - phase).rem_euclid(1.);
                    let opacity = if t < 0.45 {
                        1. - 0.9 * (1. - (1. - t / 0.45).powi(3))
                    } else if t < 0.92 {
                        0.1
                    } else {
                        1.
                    };
                    Styles::new().opacity(opacity)
                })
        }))
    }));
    let word = {
        let (anim, started) = (anim.clone(), started.clone());
        text_signal(move || {
            let since = started.get().unwrap_or(0.);
            let index = ((anim.ambient_clock.get() - since) / 7.).max(0.) as usize % WORDS.len();
            format!("{}…", WORDS[index])
        })
        .text_size(12.)
        .text_color(MUTED)
    };
    let elapsed = {
        let (anim, started) = (anim.clone(), started.clone());
        text_signal(move || {
            let seconds = (anim.ambient_clock.get() - started.get().unwrap_or(0.)).max(0.) as u32;
            if seconds >= 60 {
                format!("{}m {}s", seconds / 60, seconds % 60)
            } else {
                format!("{seconds}s")
            }
        })
        .text_size(11.)
        .mt(1.)
        .text_color(FAINT)
    };
    row()
        .pt(16.)
        .gap(8.)
        .items_center()
        .reactive_style(move || {
            if working() {
                Styles::new().flex()
            } else {
                Styles::new().hidden()
            }
        })
        .child(spinner)
        .child(word)
        .child(elapsed)
}
