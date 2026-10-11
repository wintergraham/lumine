# Heavy dashboard: lumine and GPUI on an M5 Max

`scripts/run_macos_comparison.py --scene heavy`, 2026-09-27. Apple M5 Max
(Mac17,6), macOS 26.4 (25E246), Metal, on AC power. A personal Mac in use:
other applications kept running, so the host was not isolated.

## The scene

`lumine_workload::heavy`: a 1280×800 window (2560×1600 pixels on this Retina
display) where most content changes every tick. All values derive from shared
deterministic functions, so both adapters draw the same pixels
([lumine](lumine.png), [GPUI](gpui.png)):

- 6 stat tiles with live values;
- 30 rounded, bordered service cards, each with a live value, a progress bar
  and a 26-bar sparkline that scrolls one sample per tick (780 moving bars);
- a 5-column table of 100,000 rows with coloured status pills, live latency
  and throughput, scrolling 14 px per tick;
- a 5-line streaming log.

Modes: `idle` draws everything once; `stream` advances all live data;
`scroll` scrolls the table only; `both` does both. Active modes request 60
updates per second.

Adapters, each in its framework's idiomatic best case:

- lumine ([`heavy_workload`](../../../crates/lumine-desktop/examples/heavy_workload.rs)):
  retained views; every live value is a reactive text or style, and the table
  is a `virtual_list`.
- GPUI 0.2.2 ([`bin/heavy`](../../../comparisons/gpui/src/bin/heavy.rs)):
  tiles and cards, the table and the log are separately cached views, and
  each tick notifies only the views whose data changed.
- QuickGUI is not included: its adapter
  ([`bin/heavy`](../../../comparisons/quickgui/src/bin/heavy.rs)) panics at
  startup (`index out of bounds: the len is 32 but the index is 32` in
  `renderer/text_system.rs`), its text renderer pool holding 32 batches.

## Protocol

As the macOS lab comparison: three rotated repeats of each mode per
framework, 20 s each, the first 5 s excluded and the next 15 s sampled every
50 ms for CPU (percent of one core), physical footprint and resident size.
24 trials, 6,509 samples. Every active trial delivered 59.7–59.95 updates per
requested second (gate 58–61); idle trials none.

Both adapters drew every update. Separately from the timed trials, over 10 s
of `both`, lumine presented 60–61 frames per second (`LUMINE_GPU_STATS`) and GPUI
rendered its dashboard view 592 times for 591 ticks (a temporary counter,
removed; the rebuilt GPUI binary's hash matches the one measured).

## Results

Medians of three repeats (range in parentheses).

| Mode | lumine CPU | GPUI CPU | lumine vs GPUI |
|---|---|---|---|
| idle | 0.00% (0.00–0.00) | 0.37% (0.37–0.37) | — |
| stream | 10.91% (10.61–11.20) | 20.56% (19.89–20.99) | −47% |
| scroll | 3.98% (3.98–3.99) | 13.22% (13.15–13.95) | −70% |
| both | 11.48% (11.28–11.53) | 20.94% (20.94–21.27) | −45% |

| Mode | lumine footprint | GPUI footprint | lumine RSS | GPUI RSS |
|---|---|---|---|---|
| idle | 91.2 MiB | 104.0 MiB | 103.1 MiB | 73.2 MiB |
| stream | 295.3 MiB | 267.4 MiB | 106.7 MiB | 73.8 MiB |
| scroll | 290.3 MiB | 267.3 MiB | 105.0 MiB | 74.1 MiB |
| both | 295.8 MiB | 270.4 MiB | 107.3 MiB | 74.9 MiB |

lumine used about half GPUI's CPU with live data and under a third while
scrolling. GPUI used less memory while active: about 25–28 MiB less physical
footprint and 33 MiB less resident memory. Idle footprint varied between runs
(GPUI 84–104 MiB across the two runs made today) and does not rank either.

CPU is the process's own; GPU execution and WindowServer compositing are not
included.

## Source

Measured before commit: `commit` in `audit.json` is the parent (92fd6a4) and
`source_dirty` is true. The new adapter files were untracked when measured,
which that audit's `source_diff_sha256` does not cover (the runner now
includes them); their SHA-256:

```
276f112d35df1957e8f62de9bdf850f50aaac7efc173d8f55dc9ae7f68dbb17e  crates/lumine-desktop/examples/heavy_workload.rs
0e5232a4f2e5b13e6a088e93990f626dbe149a66210cf0b1f26c4c19db67a0c3  comparisons/gpui/src/bin/heavy.rs
29220325b70179b0d6794efa427f0fabd221b2c660519056d0eb669d7fdce374  comparisons/quickgui/src/bin/heavy.rs
d7fb4f9f435bd8121034d7a1237704c965b2c9f1737adfab28e501c01c193614  comparisons/workload/src/lib.rs
```

Files: `current.csv` (one row per trial), `summary.json` (medians and ranges),
`audit.json` (acceptance, environment, binary hashes), and per-trial JSON and
logs.
