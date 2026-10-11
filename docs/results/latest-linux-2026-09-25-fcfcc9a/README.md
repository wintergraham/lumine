# Latest Linux attempt: rejected workload parity

Measured revision: `fcfcc9ac79dd3950f44ea098a661cae103bc9b8e`.

**Not accepted as an equivalent-work performance comparison.** All 36 trials completed with exit 0, but 19/27 active trials failed the unchanged 58–61 logical updates per requested second gate. Raw evidence is retained without dropped trials or retries. These CPU/RSS results must not replace the last accepted dashboard graphs or be interpreted as a framework speedup.

## Launch condition and shared-host limitations

The original clean-start gate rejected unrelated cargo/rustc builds. After repeated waits, the launch condition was explicitly amended before any trial to warn instead of refuse. Unrelated builds were active from the start and during sampling. The first rejection, process snapshot, timestamped wait log, and preregistered deviation remain in this directory. All actual sampling, workload, duration, repeat, source/binary verification, and update-rate acceptance rules remained unchanged.

This series used Linux Xvfb/Openbox and Mesa software Vulkan, LP_NUM_THREADS=4 per pool, with 20 requested seconds and 5 seconds excluded warmup per trial. It is neither hardware-GPU nor native macOS evidence. Updates count model progress, not presented frames. Contention plausibly contributes to missed updates, but this run does not establish causation or isolate code changes from host activity.

## Integrity checks

All 36 raw trial summaries and CSV rows were independently recomputed, as were 12 summary groups. 10,808 samples and 437 archived inputs verified. Exact rotated order, source archive/current source hashes, build inputs, all three binary hashes, and private Vulkan loader hash matched. `independent-audit.json` has status `rejected_workload_parity`; integrity verification passing does not mean parity passing. Original archived audit helper is unchanged; `rejected-audit.py` independently records gate violations instead of aborting at the first failure.

Sampling observations: 2026-09-25T20:29:38.523820+00:00 through 2026-09-25T20:41:49.765647+00:00.

Seeded 180-tick screenshots were captured before sampling and visually checked for matching streaming content and virtual list position. QuickGUI created two Mesa worker pools; the per-pool thread setting does not guarantee equal process worker counts.

## Active update rates

| Framework | Minimum | Maximum | Failed active trials |
| --- | ---: | ---: | ---: |
| lumine | 47.85 | 59.95 | 3/9 |
| gpui | 15.00 | 59.60 | 8/9 |
| quickgui | 19.45 | 58.75 | 8/9 |

## Rejected-series measurements

Descriptive process metrics only; unequal completed work prohibits performance rankings. CPU is percent of one logical core, RSS is resident process MiB and excludes GPU/other-process memory.

| Mode | Framework | CPU median [min–max] | RSS median [min–max], MiB |
| --- | --- | ---: | ---: |
| idle | lumine | 0.00 [0.00–0.00] | 107.60 [107.41–108.10] |
| idle | gpui | 0.32 [0.20–0.33] | 112.95 [112.46–112.98] |
| idle | quickgui | 0.06 [0.00–0.07] | 136.19 [134.33–136.64] |
| stream | lumine | 30.44 [28.52–31.59] | 112.75 [112.14–114.64] |
| stream | gpui | 143.41 [71.37–200.51] | 116.01 [113.11–116.08] |
| stream | quickgui | 167.41 [68.28–180.60] | 139.50 [139.03–141.02] |
| scroll | lumine | 15.28 [12.74–15.80] | 116.11 [115.55–116.36] |
| scroll | gpui | 160.81 [138.37–183.81] | 111.68 [111.37–112.74] |
| scroll | quickgui | 141.79 [138.26–147.72] | 143.48 [141.82–144.17] |
| both | lumine | 35.57 [32.36–35.58] | 115.95 [114.08–116.34] |
| both | gpui | 175.11 [101.26–201.06] | 113.87 [112.82–113.88] |
| both | quickgui | 180.21 [87.98–196.00] | 142.91 [142.09–144.47] |

## Exact failed trials

| Framework | Mode | Repeat (zero based) | Updates/requested second |
| --- | --- | ---: | ---: |
| lumine | stream | 0 | 57.75 |
| gpui | stream | 0 | 15.00 |
| quickgui | stream | 0 | 19.45 |
| lumine | scroll | 0 | 56.40 |
| gpui | scroll | 0 | 31.70 |
| quickgui | scroll | 0 | 41.00 |
| gpui | both | 0 | 30.45 |
| quickgui | both | 0 | 23.85 |
| gpui | stream | 1 | 48.45 |
| quickgui | stream | 1 | 51.50 |
| gpui | scroll | 1 | 51.45 |
| quickgui | scroll | 1 | 47.50 |
| gpui | both | 1 | 53.20 |
| quickgui | both | 1 | 54.95 |
| quickgui | stream | 2 | 50.60 |
| gpui | stream | 2 | 39.80 |
| quickgui | scroll | 2 | 43.65 |
| lumine | scroll | 2 | 47.85 |
| gpui | both | 2 | 56.30 |

## Prior accepted run

The prior accepted series is `../latest-linux-2026-09-25-rerun/`, measured at `ed0d277623e5dcbe74640fd80276bfd2fa2f0084`. That series passed all update gates. This new rejected attempt cannot establish improvement or regression versus it; the dashboard should retain those accepted graphs and disclose this newer failed attempt.

## Reproduce the audit

Run `python3 docs/results/latest-linux-2026-09-25-fcfcc9a/rejected-audit.py` while the frozen binaries and loader remain available. The script verifies frozen archive identity and additionally reports whether current repository inputs differ. `run.py` intentionally exits nonzero for this measured series because the parity gate failed.
