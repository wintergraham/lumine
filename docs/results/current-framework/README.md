# Current public-framework release comparison

The current public component-based lumine application used less sampled CPU and
RSS than the pinned GPUI and QuickGUI adapters in every active mode in this
software-rendered Linux run. All three had effectively idle CPU usage when idle.
This is an observed result for these applications and backend policies, not proof
of minimum possible resource use or hardware GPU superiority.

The lumine build includes the expanded component/widget API, native input and live
DPI fixes, wrapped editors and explicit line-height support. The shared workload
and both reference adapters were unchanged: their eight source/manifest hashes
and frozen release binary hashes match the earlier reference build proof. All
three render the same requested 960×720 layout, content, colors, streaming text
and virtual-list ranges (17 initial rows, at most 19 at interior offsets).
[Source audit](../component-comparison-audit.md) describes the architecture.
Separate screenshots at the same seeded 180-update state were visually inspected:
[lumine](lumine.png), [GPUI](gpui.png), [QuickGUI](quickgui.png). Font rasterization and
baseline details are not pixel-identical.

Twenty-four GUI processes completed: three frameworks, four modes and two
repeats, rotating framework order between repeats. Each requested ten seconds of
model activity; CPU/RSS sampling excludes the first three seconds after process
spawn. Actual sampled intervals were **6.990–7.104 seconds**, and total observed
process lifetimes were **10.087–10.221 seconds**. Active trials reported
**596–599 model updates** across their requested ten-second workload; idle trials
reported zero. These counters cover the full workload, not just post-warmup
samples, and are not presentation timestamps or evidence of displayed frame rate.

Each process ran alone on an owned Xvfb 1100×820 display with Openbox, the explicit
Mesa llvmpipe Vulkan ICD and the same private patched Vulkan loader. Capture-time
`/proc/maps` files confirm that loader in all three processes. Team builds and
tests were paused during sampling; source and binary hashes were unchanged
through measurement and capture. The host was shared and had no CPU pinning or
thermal isolation. Two short repeats show observed spread, not statistical
confidence. CPU percentages include software GPU rendering and can exceed one
core's 100%.

The memory difference includes backend initialization policy: lumine initializes
primary graphics backends, while the unchanged QuickGUI reference also retains
unused EGL resources despite the Vulkan environment setting. Capture maps show
EGL in QuickGUI and none in lumine/GPUI. This is not an isolated measurement of
component architecture overhead. Hardware GPU and native macOS comparisons
remain separate validation work.

| Mode | Framework | CPU median [range], % one core | Mean RSS median [range], MiB | Full-workload updates [range] |
| --- | --- | ---: | ---: | ---: |
| idle | lumine | 0.00 [0.00–0.00] | 100.40 [100.13–100.66] | 0–0 |
| idle | gpui | 0.14 [0.14–0.14] | 108.36 [108.20–108.52] | 0–0 |
| idle | quickgui | 0.00 [0.00–0.00] | 132.18 [131.79–132.57] | 0–0 |
| stream | lumine | 136.52 [135.87–137.17] | 105.58 [104.78–106.38] | 599–599 |
| stream | gpui | 312.59 [310.61–314.58] | 110.97 [110.82–111.12] | 596–599 |
| stream | quickgui | 238.80 [238.59–239.02] | 136.58 [136.51–136.64] | 598–599 |
| scroll | lumine | 169.02 [169.01–169.03] | 106.17 [106.00–106.35] | 599–599 |
| scroll | gpui | 291.77 [290.79–292.75] | 109.07 [108.57–109.57] | 598–599 |
| scroll | quickgui | 228.18 [227.86–228.50] | 138.25 [138.11–138.40] | 599–599 |
| both | lumine | 204.94 [203.72–206.16] | 106.99 [106.94–107.04] | 596–598 |
| both | gpui | 312.70 [310.97–314.43] | 109.62 [109.22–110.03] | 599–599 |
| both | quickgui | 240.58 [240.27–240.89] | 140.34 [139.74–140.93] | 599–599 |

The [artifact audit](audit.json) recomputes all summaries from **3,386 raw
samples**, verifies all 24 trials, positive RSS, monotonic timestamps/counters,
successful exits and log-matched tick counts, and checks **156 archived source
files** plus all measured binary hashes. [CSV](current.csv),
[summary](summary.json), [sampler metadata](current.csv.metadata.json),
[measurement environment](measurement-environment.json), [preflight](preflight.json),
[capture audit](capture-audit.json),
[lumine build proof](lumine-build-manifest.json),
[reference build proof](reference-build-manifest.json),
[source manifest](source.json), [source archive](source.tar.gz), per-trial logs
and raw JSON preserve the evidence. `run.py` and `capture.py` preserve the
procedures; the README was written after sampling and is not in the source archive.

Recheck the archived artifacts independently of later workspace edits:

```sh
python3 scripts/audit_comparison.py docs/results/current-framework/current.csv \
  --source-manifest docs/results/current-framework/source.json \
  --source-archive docs/results/current-framework/source.tar.gz \
  --output /tmp/current-framework-audit.json
```

For a fresh series, build and freeze matching binaries and a new lumine build
manifest, then run `run.py --build-manifest PATH --preflight-only` before sampling.
The runner refuses to overwrite an existing CSV. Its default binary directory is
`/tmp/lumine-current-framework-bin`; specify the private patched loader directory
with `--loader`. Reusing this exact lumine build proof after source changes should
fail its preflight, rather than silently associating old binaries with new code.
