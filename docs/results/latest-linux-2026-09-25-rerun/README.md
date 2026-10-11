# Latest Linux performance rerun — 2026-09-25

Measured commit `ed0d277623e5dcbe74640fd80276bfd2fa2f0084`, pulled before building. All three adapters received release verification builds and newly frozen binaries. Measurement ran approximately **09:35:10–09:47:13 UTC** on 2026-09-25.

All 36 trials passed: 10,803 post-warmup samples; active model update rates 58.65–59.95/second against the preregistered 58–61 gate. Independent recomputation verified every CSV/raw metric, all medians/ranges, exact rotated trial order, all three executable hashes, the Vulkan loader hash, and all 435 archived inputs. At the end-of-measurement audit, current archived/build inputs still matched. No trials were discarded or retried.

Median CPU, percent of one logical core:

| Mode | lumine | GPUI | QuickGUI |
| --- | ---: | ---: | ---: |
| Idle | 0.00 | 0.13 | 0.00 |
| Streaming | 26.40 | 217.04 | 169.97 |
| Scrolling | 12.43 | 205.45 | 160.94 |
| Combined | 29.23 | 218.25 | 177.28 |

Median process RSS, MiB:

| Mode | lumine | GPUI | QuickGUI |
| --- | ---: | ---: | ---: |
| Idle | 103.30 | 109.38 | 131.48 |
| Streaming | 108.90 | 109.29 | 134.86 |
| Scrolling | 111.70 | 108.54 | 138.52 |
| Combined | 112.00 | 110.39 | 140.02 |

lumine used substantially less CPU in all three active modes. lumine had the lowest median RSS in idle and streaming; GPUI had the lowest median RSS in scrolling and combined. The small streaming RSS difference should not be interpreted as a reliable general advantage. RSS excludes GPU and other-process memory.

Compared with the previous measured revision `9d118f8`, lumine's streaming CPU median fell from 92.01% to 26.40%, scrolling from 134.24% to 12.43%, and combined from 174.48% to 29.23%. These are observations from separate shared-host series, not an isolated paired causal experiment. Historical evidence remains unchanged in `../latest-linux-2026-09-25/`.

This was **shared-host software Vulkan under Xvfb**, not hardware-GPU or macOS measurement. No builds were present at preflight, and this task paused its own builds/browser work during sampling. Unrelated cargo/rustc/rust-lld activity nevertheless appeared from approximately 35 seconds after sampling started through its end. See `host-build-observations.json` and the raw host log; process ancestry is not inferred for exited processes. This contention may affect results. The entire series is preserved, without removing repeats or retrying. Active tick counts establish logical updates, not presentation FPS.

`LP_NUM_THREADS=4` was applied to every application, but it is a per-Mesa-pool setting. Captures show four llvmpipe workers for lumine and GPUI and eight for QuickGUI across two pools. An observed 0% CPU means no accounted CPU increment between sampled endpoints, not literally zero executed cycles.

Files:

- `summary.json`: all medians and observed ranges in original units.
- `audit.json`, `independent-audit.json`: machine-readable validation.
- `current.csv` and per-trial `.json`/`.log`: complete results, raw samples and application ticks.
- `source.json`, `source.tar.gz`: frozen source inventory/archive, including complete vendored wgpu-hal inputs.
- `*-build-manifest.json`, `build-*.log`: build proof and executable hashes.
- `lumine.png`, `gpui.png`, `quickgui.png`: matched seeded 180-tick screenshots outside timed trials; content/geometry visually checked, expected font rasterizer differences.
- `protocol.md`: preregistered conditions and acceptance rules.

The additional animation/busy/monitor examples were not timed in this four-mode comparison. No historical measurements substitute for current data.
