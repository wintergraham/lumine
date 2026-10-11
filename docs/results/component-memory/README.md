# Idle memory attribution and backend initialization

The first snapshots compare the frozen normalized-release binaries at idle, two seconds after process start, using `/proc/PID/smaps` and `smaps_rollup`. All processes exited successfully. These are single OS mapping snapshots, not allocator profiles or repeated whole-framework rankings. RSS, proportional set size (PSS), anonymous pages, and private pages measure different things; shared driver mappings make RSS particularly different from private memory.

The original lumine process had 104 threads and loaded both Vulkan and Gallium/EGL libraries even though its selected adapter was Vulkan. GPUI had 80 threads and no Gallium mapping. QuickGUI had 105 threads and loaded Gallium too. This identified an avoidable backend-initialization path in lumine: its wgpu instance enabled every backend and did not apply `WGPU_BACKEND`.

## Controlled backend ablation

Lumine now defaults to wgpu's primary backends (Vulkan on Linux, Metal on macOS) and applies the wgpu environment overrides. A fresh release binary was run three times with `WGPU_BACKEND=vulkan` and three times with `WGPU_BACKEND=vulkan,gl`, alternating order. Every other application/environment setting was identical. All six records have the same executable SHA-256, and the archived source hashes were verified. Processes were idle, bounded to three seconds, with a snapshot at two seconds; all exited successfully.

| Enabled backends | Median RSS [range], MiB | Threads | Gallium RSS, MiB |
| --- | ---: | ---: | ---: |
| vulkan | 106.96 [106.93–107.16] | 55 | 0.00 |
| vulkan,gl | 128.98 [128.95–129.00] | 104 | 9.05 |

Vulkan-only initialization saves approximately **22 MiB RSS and 49 threads** in this software-driver environment. Vulkan-only initialization avoids loading the GL stack. The ablation measures the cost of permitting GL initialization in this configuration. The thread reduction is driver/environment specific and is not a promise about real GPUs or macOS. Mapping data do not attribute every saved byte to a particular allocator object.

This is a same-binary backend ablation, not a replacement for the earlier normalized three-framework CPU/RSS series. It does not establish updated streaming CPU/RSS medians or a new framework ranking. Earlier benchmark artifacts remain valid for their frozen binaries, which initialized all available backends despite the recorded `WGPU_BACKEND` environment value.

[Initial mapping summaries](summary.json), raw framework `.smaps`/`.rollup` files, [ablation summaries](backend-ablation/summary.json), [source/binary manifest](backend-ablation/metadata.json), [source archive](backend-ablation/source.tar.gz), and raw per-selection snapshots/logs retain the evidence. `run.py` and `backend_ablation.py` preserve the procedures. Native Linux Vulkan and macOS Metal remain the supported default paths; availability of another wgpu backend does not imply that its native integration has been validated.
