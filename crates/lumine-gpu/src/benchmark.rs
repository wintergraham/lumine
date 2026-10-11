//! Shared native benchmark output. Enable the `benchmark` Cargo feature.
use serde_json::{Value, json};
use std::{collections::HashMap, io::Write, path::PathBuf};
pub fn summary(samples: &[f64], budget_ms: f64) -> Value {
    assert!(!samples.is_empty());
    assert!(samples.iter().all(|v| v.is_finite() && *v >= 0.));
    assert!(budget_ms.is_finite() && budget_ms > 0.);
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let percentile = |p: f64| sorted[((sorted.len() as f64 * p).ceil() as usize).max(1) - 1];
    let mut result = json!({
        "samples": sorted.len(), "mean_ms": sorted.iter().sum::<f64>() / sorted.len() as f64,
        "p50_ms": percentile(0.5), "p95_ms": percentile(0.95), "p99_ms": percentile(0.99),
        "max_ms": sorted.last().unwrap(),
        "over_budget": sorted.iter().filter(|ms| **ms > budget_ms).count(),
    });
    if std::env::var_os("LUMINE_BENCH_RAW").is_some() {
        result["samples_ms"] = json!(samples);
    }
    result
}
pub fn emit(record: Value) {
    println!("RENDER_BENCH {record}");
    if let Some(path) =
        std::env::var_os("LUMINE_BENCH_OUTPUT").or_else(|| std::env::var_os("ASK_BENCH_OUTPUT"))
    {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(file, "{record}").unwrap();
    }
}
pub struct Config {
    pub frames: usize,
    pub warmup: usize,
    pub hz: f64,
    pub in_flight: usize,
    pub only: Option<String>,
    pub trace: Option<PathBuf>,
}
impl Default for Config {
    fn default() -> Self {
        let read = |key: &str, legacy: &str, default: &str| {
            std::env::var(key)
                .or_else(|_| std::env::var(legacy))
                .unwrap_or_else(|_| default.into())
        };
        let result = Self {
            frames: read("LUMINE_BENCH_FRAMES", "ASK_BENCH_FRAMES", "240")
                .parse()
                .unwrap(),
            warmup: read("LUMINE_BENCH_WARMUP", "LUMINE_BENCH_WARMUP", "30")
                .parse()
                .unwrap(),
            hz: read("LUMINE_BENCH_HZ", "ASK_BENCH_HZ", "144")
                .parse()
                .unwrap(),
            in_flight: read("LUMINE_BENCH_IN_FLIGHT", "LUMINE_BENCH_IN_FLIGHT", "1")
                .parse()
                .unwrap(),
            only: std::env::var("LUMINE_BENCH_ONLY").ok(),
            trace: std::env::var_os("LUMINE_BENCH_TRACE").map(PathBuf::from),
        };
        assert!(result.frames > 0 && result.hz.is_finite() && result.hz > 0.);
        assert!((1..=8).contains(&result.in_flight));
        result
    }
}
impl Config {
    pub fn budget_ms(&self) -> f64 {
        1000. / self.hz
    }
    pub fn includes(&self, name: &str) -> bool {
        self.only.as_ref().is_none_or(|only| name.contains(only))
    }
}
/// Chrome/Perfetto CPU timeline, independent of GPU timestamp clocks.
#[derive(Default)]
pub struct Trace {
    events: Vec<Value>,
    tracks: HashMap<String, usize>,
}
impl Trace {
    pub fn span(&mut self, name: &str, track: &str, start_ms: f64, duration_ms: f64) {
        let tid = if let Some(tid) = self.tracks.get(track) {
            *tid
        } else {
            let tid = self.tracks.len() + 1;
            self.tracks.insert(track.to_owned(), tid);
            self.events.push(json!({"name": "thread_name", "ph": "M",
                "pid": 1, "tid": tid, "args": {"name": track}}));
            tid
        };
        self.events
            .push(json!({"name": name, "cat": "lumine", "ph": "X",
            "pid": 1, "tid": tid, "ts": start_ms * 1000., "dur": duration_ms * 1000.}));
    }
    pub fn write(&self, path: &std::path::Path) -> std::io::Result<()> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        serde_json::to_writer(file, &json!({"traceEvents": self.events}))?;
        Ok(())
    }
}
#[test]
fn percentiles_and_budget() {
    let result = summary(&[9., 1., 3., 2.], 3.);
    assert_eq!(result["p50_ms"], 2.);
    assert_eq!(result["p95_ms"], 9.);
    assert_eq!(result["mean_ms"], 3.75);
    assert_eq!(result["over_budget"], 1);
    assert_eq!(summary(&[1.], 1.)["over_budget"], 0);
}
