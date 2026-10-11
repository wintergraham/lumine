# Latest Linux performance — 2026-09-25

Measured commit `9d118f8ac8e42b4d5d127ca4c24224985781cef4`, with fresh release builds of all three adapters. The source archive includes the complete vendored wgpu-hal inputs. Measurement ran approximately **03:17:06–03:29:10 UTC** on 2026-09-25.

All 36 trials passed: 10,806 post-warmup samples; active model update rates 58.6–59.95/second against the preregistered 58–61 gate. Independent recomputation verified every CSV/raw metric, all medians/ranges, exact rotated trial order, all three executable hashes, the Vulkan loader hash, and all 433 archived inputs. At the end-of-measurement audit, current archived/build inputs still matched. No trials were discarded or retried.

Median CPU, percent of one logical core:

| Mode | lumine | GPUI | QuickGUI |
| --- | ---: | ---: | ---: |
| Idle | 0.00 | 0.20 | 0.00 |
| Streaming | 92.01 | 215.62 | 169.97 |
| Scrolling | 134.24 | 208.96 | 163.36 |
| Combined | 174.48 | 213.33 | 174.02 |

Median process RSS, MiB:

| Mode | lumine | GPUI | QuickGUI |
| --- | ---: | ---: | ---: |
| Idle | 106.70 | 105.70 | 129.72 |
| Streaming | 112.31 | 108.14 | 133.90 |
| Scrolling | 112.22 | 107.06 | 136.60 |
| Combined | 112.59 | 108.42 | 137.68 |

lumine used less CPU in the streaming and scrolling cases. Combined CPU ranges overlap with QuickGUI: lumine163.25–175.12%, QuickGUI170.36–175.82%, so the tiny median difference is not a meaningful ranking. GPUI had lower median RSS in every mode. RSS excludes GPU and other-process memory.

This was **shared-host software Vulkan under Xvfb**, not hardware-GPU or macOS measurement. Although no builds were present at preflight and this task paused its own builds/browser work, unrelated cargo/rustc/rust-lld activity appeared during sampling. Observed ancestry pointed to `/home/ubuntu/GitHub/comet`; see `host-build-observations.json` and the raw host log. The series is preserved with this contention caveat rather than silently retrying or removing repeats. Active tick counts establish logical updates, not presentation FPS.

`LP_NUM_THREADS=4` was applied to every application, but it is a per-Mesa-pool setting. Captures show four llvmpipe workers for lumine and GPUI and eight for QuickGUI across two pools. Applications also have different non-Mesa thread counts. An observed0% CPU means no accounted CPU increment between sampled endpoints, not literally zero executed cycles.

Files:

- `summary.json`: all medians and observed ranges in original units.
- `site-data.json`: normalized RSS MiB and metadata for the graph page.
- `audit.json`, `independent-audit.json`: machine-readable validation.
- `current.csv` and per-trial `.json`/`.log`: complete results, raw samples and application ticks.
- `source.json`, `source.tar.gz`: frozen input inventory/archive.
- `*-build-manifest.json`, `build-*.log`: fresh build proof and executable hashes.
- `lumine.png`, `gpui.png`, `quickgui.png`: matched seeded180tick screenshots outside timed trials; geometry/content checked visually, rasterizer font differences expected.
- `protocol.md`: preregistered conditions and acceptance rules.

The additional animation/busy/monitor examples added on latest main were not timed in this four-mode comparison. No historical measurements are presented as current data.
