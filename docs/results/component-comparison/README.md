# Public component release comparison

This series measures the public component-based lumine application against the actual pinned GPUI and QuickGUI adapters. It includes retained component/slot/provider ownership, reactive text, styled views, native input/accessibility integration and the public virtual list. It does not use the earlier direct-scene lumine adapter.

All three release binaries rendered the same requested 960×720 geometry, text and colors, with normalized viewport-derived row ranges (17 initially, 19 at interior offsets). Heading and body line heights, label positions and missed-deadline handling were aligned before freezing the binaries. [Source audit](../component-comparison-audit.md) explains architectural differences. Frozen-state native screenshots at 180 initial ticks were visually inspected: [lumine](lumine.png), [GPUI](gpui.png), [QuickGUI](quickgui.png). Font rasterization and baseline details are not pixel-identical.

Twenty-four processes completed successfully: four modes, three frameworks, two repeats, ten seconds each, excluding three seconds of startup warmup. Framework order rotated across repeats. A private Xvfb 1100×820 desktop with Openbox used the Mesa llvmpipe Vulkan ICD explicitly; CPU percentages include software GPU rendering. Team builds and tests were paused during sampling. This shared machine has no CPU pinning or thermal isolation; two short repeats show observed spread, not statistical confidence.

Active runs delivered **594–599 model updates** over ten requested seconds; all lumine active runs delivered 599. Idle runs reported zero. The audit recomputed every CSV summary from **3,395 raw samples**, checked positive RSS and monotonic timestamps/counters, verified all 70 archived source files and the three measured binary hashes, and matched the reference build manifests to the series source archive. Model updates are not measured presentation timestamps.

In this environment, lumine used less sampled active process CPU than both references. Its RSS was lower than QuickGUI and higher than GPUI in every mode. This does not establish minimum possible resource use, hardware GPU performance, macOS runtime behavior, or the fastest possible implementation in any framework.

| Mode | Framework | CPU median [range], % one core | Mean RSS median [range], MiB | Updates [range] |
| --- | --- | ---: | ---: | ---: |
| idle | lumine | 0.00 [0.00–0.00] | 124.53 [123.69–125.38] | 0–0 |
| idle | gpui | 0.14 [0.14–0.14] | 107.63 [106.75–108.51] | 0–0 |
| idle | quickgui | 0.00 [0.00–0.00] | 132.30 [131.95–132.65] | 0–0 |
| stream | lumine | 136.72 [136.38–137.05] | 127.04 [126.86–127.23] | 599–599 |
| stream | gpui | 312.22 [310.65–313.79] | 109.29 [108.91–109.68] | 597–599 |
| stream | quickgui | 239.68 [239.46–239.89] | 135.97 [135.66–136.27] | 599–599 |
| scroll | lumine | 169.28 [169.21–169.34] | 126.45 [126.28–126.61] | 599–599 |
| scroll | gpui | 287.47 [279.47–295.46] | 108.94 [108.82–109.06] | 594–599 |
| scroll | quickgui | 228.02 [227.80–228.25] | 138.80 [138.77–138.83] | 599–599 |
| both | lumine | 206.75 [206.54–206.97] | 127.48 [126.99–127.96] | 599–599 |
| both | gpui | 313.39 [311.24–315.53] | 109.55 [109.55–109.55] | 598–599 |
| both | quickgui | 239.12 [236.63–241.62] | 139.49 [138.95–140.03] | 595–599 |

[CSV](current.csv), [summary](summary.json), [audit](audit.json), [environment/binary hashes](current.csv.metadata.json), [source manifest](source.json), [source archive](source.tar.gz) and per-run CSV-prefixed logs/JSON retain the evidence. `run.py` and `capture.py` preserve the private-display procedures. The source archive predates the post-processing audit script and captures the code used by the measured binaries; subsequent workspace edits do not change those binaries.

Do not subtract the historical direct-scene CPU numbers from these numbers to infer isolated component overhead: the adapter behavior, row policy, typography details and sampling session changed. A controlled component-versus-direct-scene ablation would need freshly normalized paired binaries and a separate run. The earlier five-second component release smoke remains separate in `../component-release-smoke`.

Recheck the artifacts with:

```sh
python3 scripts/audit_comparison.py docs/results/component-comparison/current.csv \
  --source-manifest docs/results/component-comparison/source.json \
  --source-archive docs/results/component-comparison/source.tar.gz \
  --output /tmp/component-audit.json
```

Subsequent work found that these frozen lumine/QuickGUI binaries initialized unused GL resources despite the `WGPU_BACKEND=vulkan` environment setting. The Vulkan ICD and measured binaries remain exactly as archived. A later lumine default-backend fix is evaluated separately in the [memory ablation](../component-memory/README.md); its savings are not folded into this series.
