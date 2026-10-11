<!-- Historical record: lumine software backend, one 3-second repeat per mode. -->
# Historical software-renderer smoke measurements

`xvfb-llvmpipe-smoke.csv` contains actual native process samples from all three release binaries: four workload modes, one run each, 3 seconds per process, first second excluded. Every process exited successfully. Active workloads delivered 177–179 ticks; idle delivered zero. Deterministic screenshots were inspected to confirm both comparison frameworks actually displayed the intended UI (rather than measuring a black/unpresented window).

These are **environment smoke results, not evidence of native GPU performance or a general lumine advantage**. GPUI and QuickGUI ran GPU rendering through llvmpipe on CPU; lumine used its software renderer. This penalizes the GPU paths, and GPU memory/time is not measured. Only one short repeat was taken on a shared machine. No presentation-cadence or latency instrumentation is included. Warmup is short and timing includes platform startup differences. Root software font rendering also differs from native GPU text shaping.

CSV columns contain post-warmup sampled wall time, process-wide CPU seconds, CPU percentage relative to one core, mean/peak resident memory bytes, sample count, whole-run workload ticks, and exit code. See the adjacent metadata JSON for compiler, machine and binary hashes, and per-process logs for tick counts. Re-run `scripts/compare.py` on a physical GPU desktop for a meaningful comparison, with longer warmup, repeated trials, and independently verified presentation cadence.

The measured values below are software-GPU smoke data only. CPU is percent of one core; RSS is mean MiB after warmup.

| Mode | lumine CPU / RSS | GPUI CPU / RSS | QuickGUI CPU / RSS |
| --- | ---: | ---: | ---: |
| idle | 0.00% / 31.64 MiB | 12.13% / 108.04 MiB | 0.00% / 131.46 MiB |
| stream | 4.48% / 31.73 MiB | 305.03% / 109.32 MiB | 243.69% / 135.57 MiB |
| scroll | 6.47% / 31.88 MiB | 288.81% / 107.75 MiB | 228.28% / 138.23 MiB |
| both | 9.93% / 31.35 MiB | 309.63% / 109.46 MiB | 241.19% / 139.34 MiB |
