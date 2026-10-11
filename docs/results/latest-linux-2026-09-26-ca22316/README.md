# Latest Linux comparison — 2026-09-26

Measured commit `ca22316ec63f3605204eecfc2cee1cfeffac5c9a`, pulled before building. All three adapters received release verification builds and newly frozen binaries. Measurement ran **2026-09-26T04:35:49.462568+00:00–2026-09-26T04:47:52.563945+00:00**.

All 36 trials passed: 10,813 post-warmup samples; active model update rates 59.5–59.95/second against the unchanged 58–61 gate. Independent recomputation verified every CSV/raw metric, all medians/ranges, exact rotated trial order, all executable and loader hashes, and all 438 archived inputs. At audit time current archived/build inputs matched. No trials were discarded or retried.

Median CPU, percent of one logical core:

| Mode | lumine | GPUI | QuickGUI |
| --- | ---: | ---: | ---: |
| Idle | 0.00 | 0.13 | 0.00 |
| Streaming | 24.69 | 206.25 | 160.92 |
| Scrolling | 10.45 | 196.81 | 153.14 |
| Combined | 28.03 | 208.74 | 162.74 |

Median process RSS, MiB:

| Mode | lumine | GPUI | QuickGUI |
| --- | ---: | ---: | ---: |
| Idle | 100.60 | 105.40 | 129.06 |
| Streaming | 177.25 | 107.29 | 133.64 |
| Scrolling | 114.07 | 106.41 | 136.07 |
| Combined | 181.04 | 107.43 | 137.06 |

lumine used less CPU in all active modes and the lowest idle RSS. GPUI had the lowest RSS in all active modes. lumine streaming/combined RSS increased substantially versus the previous accepted `ed0d277` series: 108.90→177.25 MiB streaming and 112.00→181.04 MiB combined. CPU medians were somewhat lower for all three frameworks than that prior series. These are separate shared-host observations, not an isolated paired experiment or proof of a causal change. The previous contended `fcfcc9a` attempt remains rejected; no values from it are used in these accepted results.

This was **shared-host software Vulkan under Xvfb**, not hardware-GPU or macOS measurement. No cargo/rustc/rust-lld processes were present at preflight or in 145 observations during sampling; other shared-host activity remained, and snapshots can miss short processes. This task paused its own builds/browser work during sampling. See `host-build-observations.json` and raw host observations. Active ticks measure logical updates, not presented FPS.

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
