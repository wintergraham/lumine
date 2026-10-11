# Latest Linux comparison — 2026-09-26

Measured commit `10b90de10aada565dd598605f7e6902a4c83ce88`, pulled before building. All three adapters received release verification builds and newly frozen binaries. Measurement ran **2026-09-26T05:08:50.755039+00:00–2026-09-26T05:20:53.953657+00:00**.

All 36 trials passed: 10,815 post-warmup samples; active model update rates 59.25–59.95/second against the unchanged 58–61 gate. Independent recomputation verified every CSV/raw metric, all medians/ranges, exact rotated trial order, all executable and loader hashes, and all 439 archived inputs. At audit time current archived/build inputs matched. No trials were discarded or retried.

Median CPU, percent of one logical core:

| Mode | lumine | GPUI | QuickGUI |
| --- | ---: | ---: | ---: |
| Idle | 0.00 | 0.13 | 0.00 |
| Streaming | 24.63 | 205.74 | 161.07 |
| Scrolling | 10.45 | 194.33 | 153.40 |
| Combined | 27.88 | 208.81 | 161.67 |

Median process RSS, MiB:

| Mode | lumine | GPUI | QuickGUI |
| --- | ---: | ---: | ---: |
| Idle | 100.78 | 105.79 | 128.80 |
| Streaming | 107.58 | 108.00 | 133.16 |
| Scrolling | 109.73 | 105.62 | 135.87 |
| Combined | 110.47 | 107.64 | 136.78 |

Compared with accepted `ca22316`, lumine CPU remained essentially unchanged: 24.69→24.63% streaming, 10.45→10.45% scrolling, and 28.03→27.88% combined. Streaming RSS fell 177.25→107.58 MiB (39.3%) and combined RSS fell 181.04→110.47 MiB (39.0%). Scrolling RSS fell 114.07→109.73 MiB. This is consistent with the revision's retention of prepared text only for drawn content, but separate shared-host runs do not isolate causality. lumine used the lowest CPU in all active modes and lowest RSS in idle and streaming; GPUI retained the lowest scrolling/combined RSS. Small differences within trial ranges should not be treated as established rankings.

This was **shared-host software Vulkan under Xvfb**, not hardware-GPU or macOS measurement. No cargo, rustc or rust-lld processes at preflight or in 5-second observations. This task ran no builds or browser work during sampling. Other shared-host activity remained. Snapshot observations can miss short processes; ps CPU is lifetime averaged, not interval contention. See `host-build-observations.json` and raw host observations. Active ticks measure logical updates, not presented FPS.

`LP_NUM_THREADS=4` applies per Mesa pool. Captures show four llvmpipe workers for lumine and GPUI, eight for QuickGUI across two pools. RSS excludes GPU and other-process memory. Zero measured CPU means no accounted CPU increment between sampled endpoints, not literally no executed cycles.

Files:

- `summary.json`: medians and full observed ranges.
- `audit.json`, `independent-audit.json`: machine-readable validation.
- `current.csv` and per-trial JSON/logs: complete raw samples and application reports.
- `source.json`, `source.tar.gz`: frozen source inventory/archive including complete vendored wgpu-hal inputs.
- Build manifests/logs: release build proof and hashes.
- `lumine.png`, `gpui.png`, `quickgui.png`: matched 180-tick seeded captures, visually checked outside measurement. Content/geometry agree; font rasterization differs.
- `protocol.md`: preregistered conditions and acceptance rules.

Additional animation/busy/monitor examples are outside this four-mode comparison. Historical evidence remains unchanged.
