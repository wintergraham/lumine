# Motion performance — 2026-10-03

Measured on Windows x86-64, AMD Ryzen 7 7800X3D, Rust release profile. Each case
delivers 1,000 synthetic presentation frames and keeps the complete active set
running throughout. These are single-run mean CPU timings, not statistical
medians or end-to-end frame times.

The measurement includes `FrameClock`, `LocalExecutor`, animation sampling,
batched signal writes, and optional simple reactive consumers. It excludes
scene/layout, text, GPU rendering, native presentation, and application effects.

| Animation / consumers | 1 value (µs/frame) | 100 values | 1,000 values |
|---|---:|---:|---:|
| Linear / none | 0.322 | 2.561 | 25.061 |
| Linear / one per value | 0.337 | 8.042 | 105.319 |
| Cubic Bezier / none | 0.377 | 5.805 | 60.218 |
| Cubic Bezier / one per value | 0.413 | 10.937 | 153.807 |
| Analytical spring / none | 0.327 | 3.809 | 29.116 |
| Analytical spring / one per value | 0.383 | 8.396 | 113.612 |

Every case asserted exactly 1,000 frame-waiter deliveries and 1,000 executor
polls, regardless of value count. Consumer cases asserted exactly one consumer
run per value per frame. Correctness tests separately verify a consumer reading
multiple values runs only once per frame and observes coherent samples.

The benchmark uses 60-second tweens and deliberately slow springs (stiffness
0.01, damping 0.001, mass 1) so it measures active work rather than early
settlement. Bezier uses controls `(0.2, 0, 0, 1)`.

Starting 1,000 transitions individually took 6.35–7.38 ms in these cases;
retargeting them inside one `Runtime::batch` took 102–124 µs. These are different
operations and are evidence to batch bursts, not a before/after speedup claim.
Immediate policy subscriptions are retained so a pause before the executor's
first poll still freezes the correct delay and phase.

Reproduce from the lumine workspace:

```sh
cargo test -p lumine --release motion_release_benchmark --lib -- --ignored --nocapture
```

Idle, all-paused, disposed, and delay-only work is covered by deterministic
frame-demand assertions. This timing data does not establish performance of
expensive procedural images or layout animation; measure those in the app.

## Compiled typed timelines

The follow-up timeline benchmark uses one scalar transport, five compiled linear
keyframes per `Vec2` clip, and one simple reactive consumer per clip. Every case
delivers 1,000 synthetic frames over an active 60-second timeline. Same machine
and release profile, single-run mean CPU cost:

| Parallel clips | Scalar channels | CPU µs/frame |
|---:|---:|---:|
| 1 | 2 | 0.489 |
| 32 | 64 | 3.082 |
| 128 | 256 | 12.020 |

Each case asserts exactly one driver delivery per frame. This includes transport,
curve search/interpolation, batched output writes and simple consumers. It excludes
layout, shader dispatches, blur, text and native presentation. The scalar benchmark
above is a different workload, so these numbers are not a claimed speedup ratio.

```sh
cargo test -p lumine --release compiled_timeline_release_benchmark --lib -- --ignored --nocapture
```

Regression tests verify that projected layout motion changes only paint after its
initial reflow, scroll translations do not restart projection, snapshots/derived
values request no frames, and paused/idle transports release display demand.
