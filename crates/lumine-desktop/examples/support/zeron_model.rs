//! Zeron's chats, projects, session status and transcripts as lumine signals.
//! Wire shapes follow Zeron's `crates/proto` (entities, agent, view) and
//! `crates/doc` (schema, parts, transcript_delta); unknown fields are ignored.

use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};
use lumine::reactive::{Runtime, Signal};

fn str_of<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// One sidebar row.
#[derive(Clone, PartialEq)]
pub struct ChatRow {
    pub id: String,
    pub title: String,
    pub project: String,
    pub space_id: Option<String>,
    pub cwd: Option<String>,
    pub branch: Option<String>,
    /// Kept whole: `setChatConfig` replaces it and runs copy it.
    pub config: Value,
    /// Seconds since the Unix epoch of the last activity.
    pub updated: i64,
}

impl ChatRow {
    pub fn harness(&self) -> String {
        str_of(&self.config, "harness")
            .unwrap_or("claude-code")
            .to_owned()
    }
    pub fn model(&self) -> Option<String> {
        str_of(&self.config, "model").map(str::to_owned)
    }
}

/// Parse an RFC 3339 timestamp to Unix seconds.
pub fn unix_seconds(stamp: &str) -> Option<i64> {
    let (date, time) = stamp.split_once('T')?;
    let mut date = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (date.next()??, date.next()??, date.next()??);
    let clock: String = time.chars().take(8).collect();
    let mut clock = clock.split(':').map(|p| p.parse::<i64>().ok());
    let (h, min, s) = (clock.next()??, clock.next()??, clock.next()??);
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let mut seconds = days * 86_400 + h * 3600 + min * 60 + s;
    // Offsets: "Z", "+hh:mm" or "-hh:mm" after the fraction.
    if let Some(sign_at) = time[8..].find(['+', '-']) {
        let offset = &time[8 + sign_at..];
        let sign = if offset.starts_with('-') { -1 } else { 1 };
        let mut parts = offset[1..]
            .split(':')
            .map(|p| p.parse::<i64>().unwrap_or(0));
        let (oh, om) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
        seconds -= sign * (oh * 3600 + om * 60);
    }
    Some(seconds)
}

pub fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// "now", "4m", "3h", "2d", "5w".
pub fn ago(seconds: i64) -> String {
    let age = (now_seconds() - seconds).max(0);
    match age {
        0..60 => "now".into(),
        60..3600 => format!("{}m", age / 60),
        3600..86_400 => format!("{}h", age / 3600),
        86_400..604_800 => format!("{}d", age / 86_400),
        _ => format!("{}w", age / 604_800),
    }
}

/// Join `WatchChats` with `WatchSpaces` project names.
pub fn chat_rows(chats: &Value, spaces: &HashMap<String, (String, String)>) -> Vec<ChatRow> {
    let mut rows: Vec<ChatRow> = chats
        .as_array()
        .into_iter()
        .flatten()
        .filter(|chat| {
            !chat
                .get("archived")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .filter(|chat| chat.get("parentChatId").is_none_or(Value::is_null))
        .filter_map(|chat| {
            let id = str_of(chat, "id")?.to_owned();
            let space_id = str_of(chat, "spaceId").map(str::to_owned);
            let space = space_id.as_ref().and_then(|id| spaces.get(id));
            let updated = str_of(chat, "lastMessageAt")
                .or_else(|| str_of(chat, "createdAt"))
                .and_then(unix_seconds)
                .unwrap_or(0);
            Some(ChatRow {
                title: str_of(chat, "title")
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or("New session")
                    .to_owned(),
                project: space.map_or_else(|| "Sessions".to_owned(), |(name, _)| name.clone()),
                cwd: str_of(chat, "cwd")
                    .map(str::to_owned)
                    .or_else(|| space.map(|(_, path)| path.clone())),
                branch: str_of(chat, "branch").map(str::to_owned),
                config: chat.get("config").cloned().unwrap_or(Value::Null),
                space_id,
                updated,
                id,
            })
        })
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.updated));
    rows
}

/// `WatchSpaces` → id → (display name, path).
pub fn spaces(value: &Value) -> HashMap<String, (String, String)> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|space| {
            let path = str_of(space, "path").unwrap_or_default().to_owned();
            let name = str_of(space, "name")
                .map(str::to_owned)
                .unwrap_or_else(|| path.rsplit('/').next().unwrap_or("Project").to_owned());
            Some((str_of(space, "id")?.to_owned(), (name, path)))
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Idle,
    Working,
    AwaitingInput,
    Errored,
}

/// `WatchSessions` → chat id → status. `working` rows idle for 45 s are stale.
pub fn statuses(value: &Value) -> HashMap<String, Status> {
    let now = now_seconds();
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|session| {
            let updated = str_of(session, "updatedAt")
                .and_then(unix_seconds)
                .unwrap_or(now);
            let status = match str_of(session, "status")? {
                "working" if now - updated < 45 => Status::Working,
                "awaitingInput" => Status::AwaitingInput,
                "errored" => Status::Errored,
                _ => Status::Idle,
            };
            Some((str_of(session, "chatId")?.to_owned(), status))
        })
        .fold(HashMap::new(), |mut map, (chat, status)| {
            // Several hosts may report: any active state wins.
            let entry = map.entry(chat).or_insert(Status::Idle);
            if *entry == Status::Idle {
                *entry = status;
            }
            map
        })
}

// Transcript -----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PartKind {
    Text,
    Reasoning,
    Tool,
    Error,
    Other,
}

/// A tool call as Zeron's `view::tool_chip_content` labels it.
#[derive(Clone, PartialEq, Default)]
pub struct Tool {
    pub verb: &'static str,
    pub detail: String,
    pub icon: &'static str,
    /// File tools show the file name in a badge.
    pub file: bool,
    pub resolved: bool,
    pub error: bool,
    pub output: String,
    /// `+added −removed` for edits.
    pub stats: Option<(u64, u64)>,
    /// Sidecar keys for the full output and the full diff (`FetchToolBlob`).
    pub output_ref: Option<String>,
    pub output_bytes: Option<u64>,
    pub diff_ref: Option<String>,
    /// An inline diff from older documents: (path, old, new).
    pub diff: Option<(String, Option<String>, String)>,
    /// A running subagent's latest line.
    pub subagent_tail: Option<String>,
}

fn tool(part: &Value) -> Tool {
    let call = part.get("call").cloned().unwrap_or(Value::Null);
    let field = |key: &str| str_of(&call, key).unwrap_or_default().to_owned();
    let (verb, detail, icon, file) = match str_of(&call, "kind").unwrap_or_default() {
        "exec" => ("Run", field("command"), "terminal", false),
        "readFile" => ("Read", field("path"), "document", true),
        "writeFile" => ("Write", field("path"), "document-add", true),
        "editFile" => ("Edit", field("path"), "pen", true),
        "applyPatch" => (
            "Patch",
            field("path"),
            "document",
            !field("path").is_empty(),
        ),
        "search" => {
            let path = field("path");
            let pattern = field("pattern");
            let detail = if path.is_empty() {
                pattern
            } else {
                format!("{pattern} in {path}")
            };
            ("Search", detail, "search", false)
        }
        "glob" => ("Glob", field("pattern"), "folder", false),
        "webFetch" => ("Fetch", field("url"), "global", false),
        "webSearch" => ("Web", field("query"), "global", false),
        "todo" => {
            let items = call.get("items").and_then(Value::as_array);
            let done = items
                .into_iter()
                .flatten()
                .filter(|i| i.get("done").and_then(Value::as_bool).unwrap_or(false))
                .count();
            let total = items.map_or(0, Vec::len);
            ("Todo", format!("{done}/{total} done"), "checklist", false)
        }
        "mcp" => (
            "MCP",
            format!("{} · {}", field("server"), field("tool")),
            "widget",
            false,
        ),
        _ => {
            let name = field("name");
            let agent = matches!(name.as_str(), "Task" | "Agent" | "spawn_agent");
            if agent {
                let description = call
                    .get("input")
                    .and_then(|i| str_of(i, "description"))
                    .unwrap_or_default()
                    .to_owned();
                ("Agent", description, "bot", false)
            } else {
                ("Tool", name, "widget", false)
            }
        }
    };
    let stats = part
        .get("diffStats")
        .and_then(Value::as_array)
        .map(|stats| {
            stats.iter().fold((0, 0), |(a, d), s| {
                (
                    a + s.get("additions").and_then(Value::as_u64).unwrap_or(0),
                    d + s.get("deletions").and_then(Value::as_u64).unwrap_or(0),
                )
            })
        });
    Tool {
        verb,
        detail: detail.replace('\n', " "),
        icon,
        file,
        resolved: part
            .get("resolved")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        error: part
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        output: match part.get("output") {
            Some(Value::String(output)) => output.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        },
        stats: stats.filter(|(a, d)| a + d > 0),
        output_ref: str_of(part, "outputRef").map(str::to_owned),
        output_bytes: part.get("outputBytes").and_then(Value::as_u64),
        diff_ref: str_of(part, "diffRef").map(str::to_owned),
        diff: part.get("diff").and_then(|diff| {
            Some((
                str_of(diff, "path")?.to_owned(),
                str_of(diff, "oldText").map(str::to_owned),
                str_of(diff, "newText")?.to_owned(),
            ))
        }),
        subagent_tail: str_of(part, "subagentTail").map(str::to_owned),
    }
}

pub struct Part {
    pub kind: PartKind,
    /// Text, reasoning or error message; grows as it streams.
    pub text: Signal<String>,
    pub tool: Signal<Tool>,
    /// Recently streamed chunks as (byte offset, arrival, fade seconds), for
    /// the fade-in veil. Older chunks are dropped once fully faded.
    pub chunks: Signal<Vec<(usize, f32, f32)>>,
    /// Zeron's veil paces fades to the stream: an average of the gaps
    /// between appends (ms), and when the last one arrived.
    pace: std::cell::Cell<(f32, f32)>,
    /// When the part first appeared, if it arrived live (not in a snapshot).
    pub born: Option<f32>,
}

struct Entry {
    role: String,
    streaming: bool,
    parts: Vec<String>,
    born: Option<f32>,
}

/// A row of the transcript: parts of consecutive tool calls and thoughts
/// share a group, as in Zeron.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Block {
    User(String),
    Text(String),
    Group(String),
    Error(String),
}

pub struct Transcript {
    runtime: Runtime,
    order: Vec<String>,
    entries: HashMap<String, Entry>,
    pub parts: HashMap<String, Rc<Part>>,
    pub blocks: Signal<Vec<Block>>,
    /// Group key → its parts, in order.
    pub groups: HashMap<String, Signal<Vec<String>>>,
    /// User entry → its text.
    pub user_text: HashMap<String, Signal<String>>,
    /// The newest assistant entry is still streaming.
    pub streaming: Signal<bool>,
    /// Loaded at least once (a reset arrived).
    pub loaded: Signal<bool>,
    now: f32,
    snapshot: bool,
    /// Tool rows born in the current frame, for their stagger.
    staggered: u32,
}

pub type Shared = Rc<RefCell<Transcript>>;

impl Transcript {
    pub fn new(runtime: Runtime) -> Self {
        Self {
            blocks: runtime.signal(Vec::new()),
            streaming: runtime.signal(false),
            loaded: runtime.signal(false),
            runtime,
            order: Vec::new(),
            entries: HashMap::new(),
            parts: HashMap::new(),
            groups: HashMap::new(),
            user_text: HashMap::new(),
            now: 0.,
            snapshot: false,
            staggered: 0,
        }
    }
    /// When a block first appeared live, for its entrance animation.
    pub fn born(&self, block: &Block) -> Option<f32> {
        match block {
            Block::User(entry) => self.entries.get(entry).and_then(|e| e.born),
            Block::Text(key) | Block::Group(key) | Block::Error(key) => {
                self.parts.get(key).and_then(|p| p.born)
            }
        }
    }

    /// Apply one `WatchDocMessages` frame: a `reset` snapshot or deltas.
    /// Returns false when the frame does not line up and a resubscribe is due.
    /// `now` is the page clock, used to time entrance and streaming fades.
    pub fn apply(&mut self, frame: &Value, now: f32) -> bool {
        self.now = now;
        self.staggered = 0;
        let mut structural = false;
        if let Some(entries) = frame.get("reset").and_then(Value::as_array) {
            self.order.clear();
            self.entries.clear();
            // Snapshot rows were already there: they don't animate in.
            self.snapshot = true;
            for entry in entries {
                if let Some(id) = self.ingest(entry) {
                    self.order.push(id);
                }
            }
            self.snapshot = false;
            let live: HashSet<_> = self
                .entries
                .values()
                .flat_map(|e| e.parts.clone())
                .collect();
            self.parts.retain(|key, _| live.contains(key));
            structural = true;
            self.loaded.set(true);
        }
        for upsert in frame
            .get("upsert")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(entry) = upsert.get("entry") else {
                continue;
            };
            let existed = str_of(entry, "id").is_some_and(|id| self.entries.contains_key(id));
            let before = str_of(entry, "id")
                .and_then(|id| self.entries.get(id))
                .map(|e| e.parts.clone());
            let Some(id) = self.ingest(entry) else {
                continue;
            };
            if !existed {
                let after = str_of(upsert, "after");
                let at = after
                    .and_then(|after| self.order.iter().position(|e| e == after))
                    .map_or(0, |i| i + 1);
                self.order.insert(at.min(self.order.len()), id);
                structural = true;
            } else if before.as_ref() != self.entries.get(&id).map(|e| &e.parts) {
                structural = true;
            }
        }
        for append in frame
            .get("append")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(entry), Some(part), Some(text)) = (
                str_of(append, "entry"),
                str_of(append, "part"),
                str_of(append, "text"),
            ) else {
                continue;
            };
            let key = format!("{entry}/{part}");
            match self.parts.get(&key) {
                Some(part) => {
                    let offset = part.text.with_untracked(String::len);
                    // duration = clamp(3 × EMA(gap), 120, 400) ms, EMA from 160.
                    let (ema, last) = part.pace.get();
                    let gap = ((now - last) * 1000.).clamp(0., 1000.);
                    let ema = if last == f32::NEG_INFINITY {
                        160.
                    } else {
                        ema * 0.7 + gap * 0.3
                    };
                    part.pace.set((ema, now));
                    let duration = (ema * 3.).clamp(120., 400.) / 1000.;
                    part.chunks.update(|chunks| {
                        chunks.retain(|(_, born, fade)| now - born < fade * 1.5);
                        chunks.push((offset, now, duration));
                    });
                    part.text.update(|t| t.push_str(text));
                    if let Some(len) = append.get("len").and_then(Value::as_u64)
                        && part.text.with_untracked(String::len) as u64 != len
                    {
                        return false;
                    }
                    if let Some(user) = self.user_text.get(entry) {
                        user.update(|t| t.push_str(text));
                    }
                }
                None => return false,
            }
        }
        for removed in frame
            .get("remove")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = removed.as_str()
                && let Some(entry) = self.entries.remove(id)
            {
                self.order.retain(|e| e != id);
                for key in entry.parts {
                    self.parts.remove(&key);
                }
                structural = true;
            }
        }
        if let Some(count) = frame.get("count").and_then(Value::as_u64)
            && count as usize != self.order.len()
        {
            return false;
        }
        if structural {
            self.rebuild();
        }
        let streaming = self
            .order
            .last()
            .and_then(|id| self.entries.get(id))
            .is_some_and(|e| e.streaming && e.role != "user");
        self.streaming.set(streaming);
        true
    }

    /// Create or update an entry's parts; returns its id.
    fn ingest(&mut self, entry: &Value) -> Option<String> {
        let id = str_of(entry, "id")?.to_owned();
        let role = str_of(entry, "role").unwrap_or("assistant").to_owned();
        let mut keys = Vec::new();
        let mut user = String::new();
        for part in entry
            .get("parts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(part_id) = str_of(part, "id") else {
                continue;
            };
            let key = format!("{id}/{part_id}");
            let kind = match str_of(part, "kind").unwrap_or_default() {
                "text" => PartKind::Text,
                "reasoning" => PartKind::Reasoning,
                "tool" => PartKind::Tool,
                "error" => PartKind::Error,
                _ => PartKind::Other,
            };
            let text = str_of(part, "text")
                .or_else(|| str_of(part, "message"))
                .unwrap_or_default()
                .to_owned();
            if kind == PartKind::Text {
                if !user.is_empty() {
                    user.push('\n');
                }
                user.push_str(&text);
            }
            let tool = if kind == PartKind::Tool {
                tool(part)
            } else {
                Tool::default()
            };
            match self.parts.get(&key) {
                Some(existing) => {
                    existing.text.set(text);
                    existing.tool.set(tool);
                }
                None => {
                    let part = Part {
                        kind,
                        text: self.runtime.signal(text),
                        tool: self.runtime.signal(tool),
                        chunks: self.runtime.signal(Vec::new()),
                        pace: std::cell::Cell::new((160., f32::NEG_INFINITY)),
                        // Parts in a snapshot were already there. Tool rows
                        // arriving together enter 65 ms apart, as in Zeron.
                        born: (!self.snapshot).then(|| {
                            let delay = if matches!(kind, PartKind::Tool | PartKind::Reasoning) {
                                self.staggered += 1;
                                0.065 * (self.staggered - 1) as f32
                            } else {
                                0.
                            };
                            self.now + delay
                        }),
                    };
                    self.parts.insert(key.clone(), Rc::new(part));
                }
            }
            keys.push(key);
        }
        if role == "user" {
            match self.user_text.get(&id) {
                Some(signal) => {
                    signal.set(user);
                }
                None => {
                    self.user_text.insert(id.clone(), self.runtime.signal(user));
                }
            }
        }
        let streaming = str_of(entry, "status") == Some("streaming");
        let born = match self.entries.get(&id) {
            Some(existing) => existing.born,
            None => (!self.snapshot).then_some(self.now),
        };
        self.entries.insert(
            id.clone(),
            Entry {
                role,
                streaming,
                parts: keys,
                born,
            },
        );
        Some(id)
    }

    fn rebuild(&mut self) {
        let mut blocks = Vec::new();
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        let mut open_group: Option<usize> = None;
        for id in &self.order {
            let Some(entry) = self.entries.get(id) else {
                continue;
            };
            if entry.role == "user" {
                open_group = None;
                blocks.push(Block::User(id.clone()));
                continue;
            }
            for key in &entry.parts {
                let Some(part) = self.parts.get(key) else {
                    continue;
                };
                match part.kind {
                    PartKind::Text => {
                        open_group = None;
                        blocks.push(Block::Text(key.clone()));
                    }
                    PartKind::Reasoning | PartKind::Tool => match open_group {
                        Some(index) => groups[index].1.push(key.clone()),
                        None => {
                            open_group = Some(groups.len());
                            groups.push((key.clone(), vec![key.clone()]));
                            blocks.push(Block::Group(key.clone()));
                        }
                    },
                    PartKind::Error => {
                        open_group = None;
                        blocks.push(Block::Error(key.clone()));
                    }
                    PartKind::Other => {}
                }
            }
        }
        for (key, members) in groups {
            match self.groups.get(&key) {
                Some(signal) => {
                    signal.set(members);
                }
                None => {
                    self.groups.insert(key, self.runtime.signal(members));
                }
            }
        }
        self.blocks.set(blocks);
    }
}
