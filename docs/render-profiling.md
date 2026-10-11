# Rendering performance tools

These tools live in lumine and work without Ask. Run from the lumine checkout with
Python 3.10+ and the normal Rust/platform build environment:

```sh
python scripts/render_bench.py --repeats 3 --output before.jsonl
# Apply a change, then build once and repeat the same workloads:
python scripts/render_bench.py --repeats 3 --output after.jsonl
python scripts/compare_render_bench.py before.jsonl after.jsonl --max-regression 10
python scripts/compare_render_bench.py before.jsonl after.jsonl --metric gpu.scroll_copy
```

The default matrix covers 1920x1350 @1.5, 3840x2160 @1.5 and 5120x2880 @2,
400 retained rows, 30 warmup frames and 240 measured frames per workload. The
13 public-component workloads cover idle, vertical/horizontal/diagonal/fractional
scrolling, text changes, paint changes, layout, backdrop blur, images, isolated
layers, a 100,000-row virtual list and resize. `--only fractional`, `--rows 1000`,
`--sizes 3840x2160@1.5`, `--frames 500` and `--hz 144` select targeted experiments.
`--profile dev` is useful when reproducing a development build; release is default.

The runner builds once, then executes the binary for every trial. JSONL records
adapter/backend/driver, dimensions/DPI, compiler/build environment, executable
and source hashes, trial settings, p50/p95/p99/max, frame-budget misses and work
counters. It refuses to overwrite evidence or accept sources changing during
compilation. The comparator compares medians of per-trial statistics, rejects
incompatible environments and can fail CI on a chosen regression percentage.
Changing the benchmark itself requires a new baseline. Existing artifacts remain
valid evidence for their recorded binary, even if the checkout changes afterward.

## What the timings measure

`input_layout`, `flush`, `encode_submit` and `accessibility` are CPU wall times.
`completed_frame` includes these stages and a GPU completion wait. It excludes
window presentation, vsync and readback. The default one-frame-in-flight mode is
a serial cost measurement, **not displayed FPS**. `--in-flight 3` instead reports
`cpu_frame` without the wait and end-to-end `render_throughput_hz`, including the
final queue drain. Do not compare those two timing modes.

GPU timestamps are optional adapter capabilities. `gpu.total` measures elapsed
GPU time from the first recorded scope to the last, including gaps between
submissions. Per-frame scope sums isolate `uploads`, `scroll_copy`, `repaint`,
`layer_repaint`, `blur_horizontal`, `deferred_repaint` and presentation work when
those passes occur. Vertical blur is part of its following repaint pass. Copy
and upload timings require encoder timestamp support; pass timings only require
timestamp queries. Missing GPU scopes are absent, not zero. Idle workloads have
no GPU work. `gpu_profiles_dropped` counts missing frames/scopes/results; treat
those measurements as incomplete. GPU profiling adds overhead, so keep it equally
enabled in both measurements.

Counters include damage and copied pixels, instances/draws, scroll copies and
phase hits, layer repaints, shaped text/glyph uploads, geometry/buffer allocation,
blur passes and cache memory. One copied pixel means one texture pixel transfer
(a read and a write), not bytes of memory bandwidth.

```sh
python scripts/render_bench.py --only text --raw --trace text.trace.json --output text.jsonl
python scripts/render_bench.py --only fractional --phase-cache 0 --output no-cache.jsonl
python scripts/render_bench.py --only fractional --phase-cache 1 --output cache.jsonl
python scripts/render_bench.py --split-shading 0 --output full-shader.jsonl
```

`--raw` preserves ordered samples. Import the trace into Chrome tracing or
Perfetto for CPU stage timelines; GPU timestamps use a separate clock and are
reported in JSONL, not placed at fictitious CPU timestamps. Trace one trial at
a time. Shader and scroll-cache switches are explicit ablations, recorded in
metadata. Run under the same power mode and background GPU load; compilation
must finish before timing. A short run is a smoke test, not a stable baseline.

## Profile an application

Enable `lumine-gpu`'s optional `benchmark` feature in a development dependency.
`lumine_gpu::benchmark::{Config, summary, emit, Trace}` supplies the same format and
configuration. An application's ignored lib test can use the standard runner:

```sh
python lumine/scripts/render_bench.py --manifest Cargo.toml --test catalogue_render_matrix --profile dev --repeats 3 --output components.jsonl
```

Tests control their own workloads and sizes. They must emit `RENDER_BENCH` records
with scene/workload, physical size/scale, adapter/budget and either
`completed_frame` or `cpu_frame`. The test adapter supports serial timings;
in-flight and trace options belong to the standard example.

Runtime GPU timing has no JSON dependency and is opt-in:

```rust
let supported = renderer.set_gpu_profiling(true);
// Normal render + presentation; offscreen callers can use renderer.submit().
for frame in renderer.take_gpu_profiles() { // nonblocking device poll
    for span in frame.spans {
        eprintln!("{}: {:.3} ms", span.label, span.duration_ms);
    }
}
let dropped = renderer.dropped_gpu_profiles();
```

`LUMINE_GPU_PROFILE=1` enables it at renderer construction. With
`LUMINE_GPU_STATS=1`, desktop logs also report GPU scope means, copy area, phase-cache
hits/memory and existing CPU/work counters. The collector never waits: it caps
pending readbacks at eight, completed results at 64 and scopes at 64 per frame,
recycles buffers and drops samples under backpressure. Poll/drain results
regularly. `wait_idle()` is available for tests; don't add it to normal frames.

## Choose the next optimization from evidence

| Expensive stage/counter | First things to inspect |
| --- | --- |
| Input/layout or flush | Rebuilding unchanged subtrees, layout dirtiness, virtualization |
| Encode CPU, draws or geometry | Batch fragmentation, repeated flattening, clipping and geometry caches |
| GPU repaint and damage | Overdraw, oversized damage unions, full repaints, shader complexity |
| GPU scroll copy | Pixel bandwidth, copy count, retained-buffer swaps and exposed-strip size |
| Text shaping/glyph upload | Changing strings, unstable fonts/sizes, avoidable atlas churn |
| Accessibility | Rebuilt ancestry/ownership, unstable semantic topology |
| Layer/blur work | Cache invalidation, changing content behind filters, oversized layers |
| Cache/allocation growth | Lifetime/eviction, repeated buffer growth, resize churn |

Check pixels against a fresh full repaint before accepting a retained-rendering
optimization (`cargo test -p lumine-gpu --test pixels -- --test-threads=1`). Keep
performance gates tied to the target adapter and environment; headless timing
does not establish presentation/input latency.

Current scroll optimization preserves disjoint damage, copies directly into the
next retained buffer, and uses one additional texture (at most 64 MiB) to reuse
matching pixel phases during half-physical-pixel scrolling. It never resamples
cached text. Other fractional phases and incompatible masks/backdrop filters
repaint. Paint/layout/effect changes invalidate reuse. `trim_caches()` releases
history, and `set_scroll_phase_cache(false)` disables it. This path is currently
disabled on macOS; Windows AMD measurements do not establish other-platform gains.
Accessibility similarly caches topology while refreshing bounds, values, focus
and inherited disabled state.

Large opaque square/rounded panel fills also partition into a plain interior and
an antialiased rim on device-pixel boundaries. Only the rim runs border/coverage
math; the interior skips blending and can occlude earlier draws. The split is
used on adapters with split shading and only when at least 32,768 visible damaged
interior pixels benefit, so tiny scroll repairs keep their existing batches.
Translucent fills, textures, shadows, transforms and fade masks keep their original
shading. Compare `--opaque-interiors 0` with `--opaque-interiors 1` to isolate this
optimization on the same binary; `--split-shading 0` disables it together with the
existing shader split. It adds geometry, not textures or another raster cache.
