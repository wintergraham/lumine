# Four-worker software-renderer comparison

All 36 trials passed the unchanged delivered-work gate: active trials completed 1,188–1,199 logical updates per 20 requested seconds (59.4–59.95 updates/second). The independent audit recomputed 10,808 samples, checked the complete inventory and successful exits, and verified 228 archived source inputs. The 169 current-framework build inputs and 8 pinned-reference build inputs matched the archive and current files at independent audit time; frozen executable hashes match their build proofs. No failed trials were removed from this series.

Each value below is the median across three repeats, followed by the observed minimum–maximum. CPU is percent of one logical core (100% = one core); RSS is MiB (2^20 bytes). Mean RSS is the per-trial sample mean, and peak RSS is the largest sampled value. Exact unrounded values are in [summary.json](summary.json), with [CSV](current.csv), per-trial raw JSON/logs, [audit](audit.json), and [acceptance](acceptance.json).

| Mode | Framework | CPU % median [min–max] | Mean RSS MiB median [min–max] | Peak RSS MiB median [min–max] |
|---|---|---:|---:|---:|
| idle | lumine | 0.000 [0.000–0.000] | 101.262 [100.680–101.453] | 101.262 [100.680–101.453] |
| idle | gpui | 0.133 [0.133–0.133] | 108.344 [107.622–108.594] | 108.344 [107.656–108.594] |
| idle | quickgui | 0.066 [0.000–0.066] | 131.231 [131.203–131.613] | 131.254 [131.203–131.613] |
| stream | lumine | 96.232 [95.876–97.642] | 106.730 [106.141–106.731] | 106.805 [106.141–106.898] |
| stream | gpui | 211.973 [211.488–214.636] | 110.434 [110.321–110.460] | 111.066 [110.953–111.074] |
| stream | quickgui | 169.387 [169.094–171.311] | 136.424 [136.124–137.299] | 137.719 [137.312–138.332] |
| scroll | lumine | 121.981 [119.193–123.870] | 105.881 [105.727–106.516] | 105.969 [105.727–106.516] |
| scroll | gpui | 208.591 [206.291–210.675] | 109.180 [108.561–109.397] | 109.180 [108.625–109.434] |
| scroll | quickgui | 162.430 [160.967–167.766] | 138.729 [138.485–139.553] | 139.188 [138.516–139.656] |
| both | lumine | 150.517 [150.428–151.144] | 106.798 [105.543–107.762] | 106.965 [105.543–107.762] |
| both | gpui | 216.408 [214.499–216.650] | 110.352 [109.440–111.784] | 110.352 [110.242–112.523] |
| both | quickgui | 174.434 [170.546–176.740] | 139.777 [139.404–140.439] | 140.691 [139.828–141.680] |

Within this software-renderer condition, lumine had lower observed median process CPU and RSS in each mode. Idle zero means no process CPU-tick increase at the sampler's resolution. These measurements do not establish lowest resource use across other applications, platforms, renderers, or hardware.

The requested process duration was 20 seconds; actual process lifetime ranged 20.067585–20.233132 seconds. Excluding the first 5 seconds after launch produced sampled windows of 14.974967–15.152093 seconds. The update gate divides final logical ticks by requested duration; it does not measure presentation cadence, latency, or missed displayed frames.

`LP_NUM_THREADS=4` was applied identically to all adapters and recorded in [measurement-environment.json](measurement-environment.json) and trial metadata. It limits the relevant Mesa worker pool, not total process threads; frameworks may create other pools or driver contexts. This is a distinct preregistered condition, not an isolated causal ablation of the thread setting. Backend initialization differences remain, including the pinned QuickGUI adapter's EGL mapping despite Vulkan rendering.

Unrelated builds were observed during sampling in [host-observations.jsonl](host-observations.jsonl). The host had no CPU pinning or thermal isolation. Three repeats show observed spread, not confidence intervals; passing the workload gate does not eliminate interference. Process RSS includes mapped/shared resident pages, and excludes separate GPU memory and other processes. Xvfb/Mesa software-driver measurements cannot establish hardware-GPU or macOS rankings.

The shared workload uses fixed-height virtualized rows. This framework revision also supports measured-height lists, but those are not exercised by this cross-framework workload. Component ownership, accessibility, host input, and timer overhead remain included.

Historical evidence remains intact: the [first longer refresh](../refined-framework/README.md) stopped amid disk exhaustion and a failed process; the [complete retry](../refined-framework-retry/README.md) failed its update-rate gate and is rejected for comparison. This successful series neither repairs those trials nor justifies pooling or subtracting their measurements. The [earlier current-framework series](../current-framework/README.md) uses a different protocol and revision.

---

# Preserved preregistered protocol

Prepared protocol; no measurements or performance conclusions yet.

This series compares the current lumine public component workload with the unchanged, pinned GPUI 0.2.2 and QuickGUI adapters. All three processes receive `LP_NUM_THREADS=4`. Mesa documents this variable for configuring llvmpipe worker threads in its [environment-variable reference](https://docs.mesa3d.org/envvars.html#llvmpipe-driver-environment-variables). This condition aims to reduce software-driver oversubscription; it does not guarantee equal work completion, equal total process thread counts, or a speedup. Results cannot establish hardware GPU or macOS rankings.

The protocol retains 36 trials: three frameworks, idle/stream/scroll/both modes, and three repeats with rotated framework order. Each requests 20 seconds, with the first 5 seconds excluded from sampled CPU and memory statistics. Requested duration, actual process lifetime, and sampled duration are distinct. Existing acceptance remains unchanged: every active trial must report 58–61 logical workload updates per requested second. These are model updates, not presented frames. A failed gate prevents equivalent-work performance claims.

Use a private Xvfb/Openbox desktop, Mesa software rendering, and the private patched Vulkan loader. `run.py --loader PATH` records the resolved loader path and hash; captures reuse that loader. The common four-worker setting is recorded in preflight, measurement environment, trial metadata, and capture environment. The same setting applies to every adapter, while each adapter retains its existing backend initialization behavior.

After source validation freezes, run `/tmp/lumine-freeze-measured-workers4.py` to build the current release `component_workload`, verify unchanged pinned reference sources/binaries, and copy all binaries into `/tmp/lumine-measured-framework-workers4-bin`. It records exact source hashes, build command/environment, compiler identity, and binary hash. Then run:

```sh
python3 docs/results/measured-framework-workers4/run.py \
  --build-manifest docs/results/measured-framework-workers4/lumine-build-manifest.json \
  --loader /tmp/lumine-ci-loader-check/build/loader
python3 docs/results/measured-framework-workers4/capture.py
```

The runner archives sources, verifies frozen binaries, refuses to overwrite an existing series, audits the complete trial inventory, and enforces the existing update-rate gate. Failed trials retain raw samples, exit information, and logs through the shared comparison harness. Do not discard failed trials, weaken gates, or select favorable repeats. Any retry needs a separate directory and an explanation; historical result directories remain unchanged. Screenshot captures use the common seeded 180-tick state, outside the measured trials.

The workload remains the shared fixed-height virtualized-list and streaming-text comparison. The directory name identifies the framework revision after measured-height list support; it does not imply that the cross-framework workload benchmarks variable-height virtualization.

## Reproducing the frozen series

[build-orchestration.py](build-orchestration.py) is an exact supplemental copy of
the original host-specific `/tmp/lumine-freeze-measured-workers4.py`. It was added
after measurement and is **not** part of the 228-file frozen source archive. It
contains the original absolute checkout/build paths and refuses an existing
build manifest or measured CSV; it is provenance, not a portable one-command
installer. Running it requires a fresh output directory and adapting those paths.

The frozen runner verifies every build-manifest input, including README files.
Reproduction therefore requires restoring the [archived source snapshot](source.tar.gz)
and its build inputs rather than running against an evolving checkout. Source
equality was checked at audit time; subsequent documentation, CI, and portable
smoke-script additions do not belong to these measured binaries. Do not overwrite
this result directory. For a new revision, build fresh adapters and write a new
series using the portable commands in [benchmarking.md](../../benchmarking.md).
