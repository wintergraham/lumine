# Component API release smoke

A release build of `lumine-desktop --example component_workload` completed all four five-second modes on Linux Xvfb with Mesa llvmpipe software Vulkan. All active modes reported 299 workload ticks (approximately 60 Hz), so the earlier debug streaming slowdown was not reproduced in this release smoke. Every application exited successfully. The Xvfb and Openbox processes created by the harness were terminated afterward.

| Mode | Workload ticks | Mounted rows at exit | CPU (% of one core) | Mean RSS (MiB) |
| --- | ---: | ---: | ---: | ---: |
| idle | 0 | 17 | 0.00 | 124.97 |
| stream | 299 | 17 | 136.07 | 127.60 |
| scroll | 299 | 19 | 167.67 | 127.71 |
| both | 299 | 19 | 206.16 | 126.85 |

These are single runs with a one-second process-start warmup and 50 ms `/proc` sampling. Tick counts describe workload updates, not GPU presentation timestamps. CPU includes llvmpipe software rendering. This is release functionality and throughput smoke evidence, not a hardware GPU benchmark, a repeated performance comparison, or evidence that components outperform another framework. The prior low-level adapter comparison is a different workload implementation and run protocol.

`metadata.json` contains the release executable SHA-256, source hashes, platform, and environment. `source.tar.gz` preserves those source files; `build.log` records the successful locked release build. Each mode has its application log and raw CPU/RSS samples. `summary.json` contains the table's unrounded values. `run.py` preserves the harness; it imports the repository's `/proc` sampler from `scripts/compare.py`.

Build command:

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 cargo build --release -p lumine-desktop --example component_workload --locked
```
