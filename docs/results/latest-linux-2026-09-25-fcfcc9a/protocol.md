# Latest Linux comparison rerun — preregistered 2026-09-25

Revision: `fcfcc9ac79dd3950f44ea098a661cae103bc9b8e` (latest main fetched by the parent before this run).

Fresh release builds of lumine public component workload, GPUI0.2.2, and QuickGUI811d6e2816d5229711f59683c4c9dfbb6fc74133. All three adapters receive release verification builds from this revision; no prior frozen binary is reused. Every source file used by the patched `vendor/wgpu-hal` crate is hashed and archived. No old performance data substitutes for this measurement.

36 trials: idle, streaming, scrolling, combined; each framework three times; rotated framework order. Each requests20 seconds, excludes the first5 seconds from process CPU/RSS sampling, and samples every50ms. Common private Xvfb/Openbox and Mesa software Vulkan with LP_NUM_THREADS=4 per Mesa pool. Same private loader for all frameworks. No repository builds during measurement; periodic host process observations record unrelated activity. No CPU affinity or thermal isolation.

Gate: every active trial delivers58–61 logical updates per requested second; idle reports0. This checks model updates, not presented frames. Failed trials and raw evidence remain; no silent retry, omission or changed gate. Failed series prohibits equivalent-work speed claims; any retry must use a separate directory with explicit reason.

All adapters use the shared100,000-row virtualized workload (28px rows,400px viewport,2-row overscan), bounded streaming text, common geometry/font/colors. Capture all interfaces at seeded180ticks outside measurements. The shared workload also contains separate animation/busy modules. Those additional workloads are outside this preregistered comparison; all freshly built adapters link the same current shared source.

Report all repeats, medians/ranges. CPU is percent of one logical core; RSS is resident process MiB, not GPU/other-process memory. Software-renderer Linux results cannot establish hardware GPU/macOS/universal framework rankings. QuickGUI may create multiple Mesa pools despite the same per-pool worker setting.

## Preregistered launch-condition deviation

2026-09-25T20:29:36.293682+00:00: The original clean-start check rejected unrelated builds, and repeated waits could not find an idle build window. Before any trial, the launch check was changed to warn and preserve the process snapshot instead of refusing. This is an explicitly contended shared-host series with unrelated builds active from the start. The original rejection and wait observations remain. No CPU/RSS sampling, workload, repeat, duration, source/binary verification, or 58–61 logical updates acceptance rules changed. All36trials will remain, including failures; no selective retries. Comparisons with earlier series cannot isolate code changes from host contention.
