# Preregistered parity expansion comparison

Prepared before measurements. No performance conclusions yet.

Compare the current retained lumine component workload with the unchanged GPUI
0.2.2 and pinned QuickGUI adapters. Reuse their previously built release binaries
only after checking both source and executable hashes against the original build
manifest. Build lumine afresh after all parity implementation and validation edits
finish. Freeze source hashes before/after the build and archive the source inputs.

The protocol matches the prior four-worker condition: private Xvfb/Openbox,
Mesa llvmpipe, the same private Vulkan loader, and `LP_NUM_THREADS=4` for every
framework. Three repeats of idle/stream/scroll/both across three frameworks give
36 trials, rotating framework order. Each requests 20 seconds; the first five
seconds are excluded from sampled CPU/RSS statistics. Preserve raw process
samples, exit status, errors, delivered logical ticks and host process observations.

Every active trial must deliver 58–61 logical updates per requested second.
This is a model-update gate, not presentation-rate validation. Failed trials are
retained, and a failed gate prohibits equivalent-work speed claims. Do not pool
historical series, drop failed repeats, change gates or silently retry. A retry
requires a new directory and an explicit reason.

No repository builds or source edits during measurement. Record unrelated host
builds if observed; this shared host has no CPU pinning or thermal isolation.
Report all three repeats, medians and observed ranges. CPU is percent of one
logical core, RSS is process resident MiB, and GPU/other-process memory is not
included. These software-renderer results cannot establish hardware-GPU or macOS
rankings or universal minimum resource use.

Capture all three interfaces at the common seeded 180-tick workload state outside
measured trials. The identical workload is a 100,000-row fixed-height virtualized
list and streaming text, not every new layout/media/native integration feature.

```sh
python3 docs/results/gpui-parity-workers4/build-orchestration.py
python3 docs/results/gpui-parity-workers4/run.py \
  --build-manifest docs/results/gpui-parity-workers4/lumine-build-manifest.json \
  --loader /tmp/lumine-ci-loader-check/build/loader
python3 docs/results/gpui-parity-workers4/capture.py
```

The build helper retains this checkout's absolute path and refuses an existing
manifest or measured CSV. Use a fresh directory and adapt paths on another host.
