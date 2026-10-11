# Latest Linux comparison — preregistered 2026-09-25

Revision: `9d118f8ac8e42b4d5d127ca4c24224985781cef4` (latest main fetched by the parent before this run).

Fresh release builds of lumine public component workload, GPUI0.2.2, and QuickGUI811d6e2816d5229711f59683c4c9dfbb6fc74133. The shared workload source changed since the prior comparison, so both references are rebuilt. Every source file used by the patched `vendor/wgpu-hal` crate is hashed and archived. No old performance data substitutes for this measurement.

36 trials: idle, streaming, scrolling, combined; each framework three times; rotated framework order. Each requests20 seconds, excludes the first5 seconds from process CPU/RSS sampling, and samples every50ms. Common private Xvfb/Openbox and Mesa software Vulkan with LP_NUM_THREADS=4 per Mesa pool. Same private loader for all frameworks. No repository builds during measurement; periodic host process observations record unrelated activity. No CPU affinity or thermal isolation.

Gate: every active trial delivers58–61 logical updates per requested second; idle reports0. This checks model updates, not presented frames. Failed trials and raw evidence remain; no silent retry, omission or changed gate. Failed series prohibits equivalent-work speed claims; any retry must use a separate directory with explicit reason.

All adapters use the shared100,000-row virtualized workload (28px rows,400px viewport,2-row overscan), bounded streaming text, common geometry/font/colors. Capture all interfaces at seeded180ticks outside measurements. The changed shared workload file adds separate animation/busy modules; the four-mode stream/list algorithm is unchanged. Those additional workloads are outside this preregistered comparison; all freshly built adapters link the same current shared source.

Report all repeats, medians/ranges. CPU is percent of one logical core; RSS is resident process MiB, not GPU/other-process memory. Software-renderer Linux results cannot establish hardware GPU/macOS/universal framework rankings. QuickGUI may create multiple Mesa pools despite the same per-pool worker setting.
