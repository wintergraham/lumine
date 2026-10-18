//! Opt-in CPU spans. Self time excludes nested work; GPU work is measured separately.
use crate::scene::NodeId;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
    time::Instant,
};

const MAX_SPANS: usize = 8192;
#[derive(Clone, Debug)]
pub struct CpuSpan {
    pub name: &'static str,
    pub node: Option<NodeId>,
    pub node_label: Option<String>,
    pub start_ms: f64,
    pub duration_ms: f64,
    pub excluded_ms: f64,
    pub self_ms: f64,
}
#[derive(Clone, Debug, Default)]
pub struct NodeProfile {
    pub calls: u64,
    pub self_ms: f64,
    pub total_ms: f64,
    pub max_ms: f64,
    pub repaints: u64,
    pub layout_ms: f64,
    pub paint_ms: f64,
    pub react_ms: f64,
    pub layout_queries: u64,
    pub cache_hits: u64,
    pub arrangements: u64,
}
struct OpenSpan {
    children_ms: f64,
    excluded_ms: f64,
    excluded: bool,
}
struct RateBucket {
    at_ms: f64,
    count: u64,
    cpu_ms: f64,
}
struct Recording {
    epoch: Instant,
    generation: u64,
    stack: Vec<OpenSpan>,
    spans: VecDeque<CpuSpan>,
    nodes: HashMap<NodeId, NodeProfile>,
    dropped: u64,
    rates: HashMap<&'static str, VecDeque<RateBucket>>,
}
impl Default for Recording {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            generation: 0,
            stack: Vec::new(),
            spans: VecDeque::new(),
            nodes: HashMap::new(),
            rates: HashMap::new(),
            dropped: 0,
        }
    }
}
#[derive(Clone, Default)]
pub struct Profiler {
    enabled: Rc<Cell<bool>>,
    recording: Rc<RefCell<Recording>>,
    labels: Rc<RefCell<HashMap<NodeId, String>>>,
    parents: Rc<RefCell<HashMap<NodeId, Option<NodeId>>>>,
    excluded: Rc<RefCell<HashSet<NodeId>>>,
    excluded_cost: Rc<Cell<f64>>,
}
impl Profiler {
    pub fn enabled(&self) -> bool {
        self.enabled.get()
    }
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
    }
    pub fn reset(&self) {
        let generation = self.recording.borrow().generation.wrapping_add(1);
        *self.recording.borrow_mut() = Recording {
            generation,
            ..Default::default()
        };
    }
    pub fn register_node(&self, node: NodeId, parent: NodeId) {
        self.parents.borrow_mut().insert(node, Some(parent));
    }
    pub fn register_root(&self, node: NodeId) {
        self.parents.borrow_mut().insert(node, None);
    }
    pub fn exclude(&self, node: NodeId) {
        self.excluded.borrow_mut().insert(node);
    }
    pub fn is_excluded(&self, node: NodeId) -> bool {
        let excluded = self.excluded.borrow();
        if excluded.is_empty() {
            return false;
        }
        let parents = self.parents.borrow();
        let mut at = Some(node);
        while let Some(node) = at {
            if excluded.contains(&node) {
                return true;
            }
            at = parents.get(&node).copied().flatten();
        }
        false
    }
    pub fn named(&self, node: NodeId) -> Option<String> {
        self.labels.borrow().get(&node).cloned()
    }
    pub fn label(&self, node: NodeId, label: String) {
        self.labels.borrow_mut().insert(node, label);
    }
    pub fn label_for(&self, node: NodeId) -> String {
        self.labels
            .borrow()
            .get(&node)
            .cloned()
            .unwrap_or_else(|| format!("{node:?}"))
    }
    pub fn remove(&self, node: NodeId) {
        self.labels.borrow_mut().remove(&node);
        self.parents.borrow_mut().remove(&node);
        self.excluded.borrow_mut().remove(&node);
        self.recording.borrow_mut().nodes.remove(&node);
    }
    pub fn layout_query(&self, node: NodeId, cached: bool) {
        if !self.enabled() || self.is_excluded(node) {
            return;
        }
        let mut recording = self.recording.borrow_mut();
        let stats = recording.nodes.entry(node).or_default();
        stats.layout_queries += 1;
        stats.cache_hits += u64::from(cached);
    }
    pub fn arranged(&self, node: NodeId) {
        if self.enabled() && !self.is_excluded(node) {
            self.recording
                .borrow_mut()
                .nodes
                .entry(node)
                .or_default()
                .arrangements += 1;
        }
    }
    pub fn repaint(&self, node: NodeId) {
        if self.enabled() && !self.is_excluded(node) {
            self.recording
                .borrow_mut()
                .nodes
                .entry(node)
                .or_default()
                .repaints += 1;
        }
    }
    pub fn nodes(&self) -> Vec<(NodeId, NodeProfile)> {
        self.recording
            .borrow()
            .nodes
            .iter()
            .map(|(id, stats)| (*id, stats.clone()))
            .collect()
    }
    pub fn spans(&self) -> Vec<CpuSpan> {
        self.recording.borrow().spans.iter().cloned().collect()
    }
    pub fn elapsed_ms(&self) -> f64 {
        self.recording.borrow().epoch.elapsed().as_secs_f64() * 1000.
    }
    pub fn dropped(&self) -> u64 {
        self.recording.borrow().dropped
    }
    /// Approximate rolling second, independent of the much shorter raw trace ring.
    pub fn excluded_cost_ms(&self) -> f64 {
        self.excluded_cost.get()
    }
    pub fn recent(&self, name: &str) -> (u64, f64) {
        let recording = self.recording.borrow();
        let cutoff = recording.epoch.elapsed().as_secs_f64() * 1000. - 1000.;
        recording
            .rates
            .get(name)
            .into_iter()
            .flatten()
            .filter(|b| b.at_ms >= cutoff)
            .fold((0, 0.), |(count, ms), b| (count + b.count, ms + b.cpu_ms))
    }
    pub fn span(&self, name: &'static str, node: Option<NodeId>) -> Option<SpanGuard> {
        if !self.enabled() {
            return None;
        }
        let start = Instant::now();
        let mut recording = self.recording.borrow_mut();
        let generation = recording.generation;
        let excluded = node.is_some_and(|node| self.is_excluded(node))
            || recording.stack.last().is_some_and(|s| s.excluded);
        recording.stack.push(OpenSpan {
            children_ms: 0.,
            excluded_ms: 0.,
            excluded,
        });
        Some(SpanGuard {
            profiler: self.clone(),
            name,
            node,
            node_label: node.map(|id| self.label_for(id)),
            start,
            generation,
        })
    }
}
pub struct SpanGuard {
    profiler: Profiler,
    name: &'static str,
    node: Option<NodeId>,
    node_label: Option<String>,
    start: Instant,
    generation: u64,
}
impl Drop for SpanGuard {
    fn drop(&mut self) {
        let duration_ms = self.start.elapsed().as_secs_f64() * 1000.;
        let mut recording = self.profiler.recording.borrow_mut();
        if recording.generation != self.generation {
            return;
        }
        let Some(open) = recording.stack.pop() else {
            return;
        };
        let self_ms = (duration_ms - open.children_ms).max(0.);
        if open.excluded && recording.stack.last().is_none_or(|parent| !parent.excluded) {
            self.profiler
                .excluded_cost
                .set(self.profiler.excluded_cost.get() + duration_ms);
        }
        if let Some(parent) = recording.stack.last_mut() {
            parent.children_ms += duration_ms;
            parent.excluded_ms += if open.excluded {
                duration_ms
            } else {
                open.excluded_ms
            };
        }
        if open.excluded {
            return;
        }
        let measured_ms = (duration_ms - open.excluded_ms).max(0.);
        if let Some(node) = self
            .node
            .filter(|node| self.profiler.parents.borrow().contains_key(node))
        {
            let stats = recording.nodes.entry(node).or_default();
            stats.calls += 1;
            stats.self_ms += self_ms;
            if matches!(self.name, "measure" | "intrinsic" | "arrange") {
                stats.layout_ms += self_ms;
            }
            if self.name == "paint/prepare" {
                stats.paint_ms += self_ms;
            }
            if self.name.starts_with("React/") {
                stats.react_ms += self_ms;
            }
            stats.total_ms += measured_ms;
            stats.max_ms = stats.max_ms.max(measured_ms);
        }
        let start_ms = self
            .start
            .saturating_duration_since(recording.epoch)
            .as_secs_f64()
            * 1000.;
        let now_ms = recording.epoch.elapsed().as_secs_f64() * 1000.;
        let at_ms = (now_ms / 100.).floor() * 100.;
        let rates = recording.rates.entry(self.name).or_default();
        while rates.front().is_some_and(|b| b.at_ms < now_ms - 1100.) {
            rates.pop_front();
        }
        if rates.back().is_none_or(|b| b.at_ms != at_ms) {
            rates.push_back(RateBucket {
                at_ms,
                count: 0,
                cpu_ms: 0.,
            });
        }
        let bucket = rates.back_mut().unwrap();
        bucket.count += 1;
        bucket.cpu_ms += measured_ms;
        let node_label = self.node_label.take();
        if recording.spans.len() == MAX_SPANS {
            recording.spans.pop_front();
            recording.dropped += 1;
        }
        recording.spans.push_back(CpuSpan {
            name: self.name,
            node: self.node,
            node_label,
            start_ms,
            duration_ms,
            excluded_ms: open.excluded_ms,
            self_ms,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rates_survive_trace_eviction_and_excluded_tools_do_not_become_hotspots() {
        use crate::scene::{Layout, NodeKind, Scene, Style};
        let mut scene = Scene::new(800., 600.);
        let profiler = scene.profiler();
        let element = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Column),
            Style::default(),
        );
        let tools = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Column),
            Style::default(),
        );
        let child = scene.append(tools, NodeKind::Container(Layout::Column), Style::default());
        profiler.label(element, "Outgoing element".into());
        assert!(!profiler.is_excluded(child));
        profiler.exclude(tools);
        assert!(profiler.is_excluded(child));
        assert!(!profiler.is_excluded(element));
        profiler.set_enabled(true);
        {
            let _app = profiler.span("app", Some(element));
            scene.remove(element);
            let _tools = profiler.span("tools", Some(child));
            profiler.layout_query(child, false);
        }
        assert!(!profiler.nodes().iter().any(|(id, _)| *id == child));
        assert!(profiler.spans().iter().all(|s| s.name != "tools"));
        assert_eq!(
            profiler.spans()[0].node_label.as_deref(),
            Some("Outgoing element")
        );
        for _ in 0..MAX_SPANS + 3 {
            drop(profiler.span("rate", None));
        }
        assert_eq!(profiler.recent("rate").0, MAX_SPANS as u64 + 3);
    }
    #[test]
    fn nested_spans_exclude_children_and_reset_discards_open_work() {
        let profiler = Profiler::default();
        assert!(profiler.span("off", None).is_none());
        profiler.set_enabled(true);
        {
            let _parent = profiler.span("parent", None);
            let _child = profiler.span("child", None);
        }
        let spans = profiler.spans();
        assert_eq!(spans.len(), 2);
        assert!((spans[1].self_ms + spans[0].duration_ms - spans[1].duration_ms).abs() < 1e-6);
        let open = profiler.span("old", None);
        profiler.reset();
        drop(open);
        assert!(profiler.spans().is_empty());
        for _ in 0..MAX_SPANS + 3 {
            drop(profiler.span("bounded", None));
        }
        assert_eq!(profiler.spans().len(), MAX_SPANS);
        assert_eq!(profiler.dropped(), 3);
    }
}
