Read-only review of the preserved Wayland IME failure

Status: diagnostic assessment only. No repository edits, builds, tests, native applications, or compositor sessions were performed for this review. The companion `/tmp/lumine-wayland-ime-blocking-probe.rs` is an **UNBUILT, UNEXECUTED diagnostic draft**. Rustfmt parsed/formatted that temporary copy; that is not a compile or runtime check.

The most specific demonstrated gap is a stuck final composition preview during a queued Pinyin burst. The failed harness stopped before pressing Space, so this artifact does **not** demonstrate loss of a committed Unicode string. It does demonstrate that the final engine preedit did not reach the application protocol connection. The passing paced tests and 22 direct-winit burst observations do not resolve that gap.

Exact preserved chronology (timestamps are the logs' milliseconds, not compositor receipt timestamps):

| Log and line | Timestamp | Observation |
| --- | ---: | --- |
| `docs/platform-validation/wayland-ime/failed-initial/native/fcitx.log:166` | 2181241.007 | First `n` keypress callback; all five keys and releases are recorded through line 207. |
| Same, lines 167–168 | 2181634.365 / 2181634.439 | Engine requests preedit `n`, then input-method `commit(2)`. About 393 ms elapsed inside the first-key processing interval. |
| `native/protocol.log:2789–2790` | 2181648.113 / 2181648.166 | Application receives `n` and text-input `done(2)`. |
| `native/fcitx.log:182–183` | 2181662.756 / 2181662.783 | Engine requests `ni` and input-method `commit(2)`. |
| `native/protocol.log:2792–2796` | 2181693.585–2181693.615 | Mesa callback, attach/damage/frame, and `wl_surface.commit`. |
| `native/protocol.log:2797–2798` | 2181693.762 / 2181693.774 | Application requests cursor rectangle `(48,70,1,31)` and text-input commit, about 45.66 ms after receiving `n`. |
| `native/fcitx.log:195–206` | 2181723.769–2181732.446 | Engine requests `ni h`, `ni ha`, `ni hao`; each uses input-method commit serial 2. |
| `native/fcitx.log:218` | 2181738.630 | Engine dispatches its third input-method `done`, after submitting the final preedit. |

No later preedit appears on the failed application protocol stream. Importantly, `ni` was marshalled before the application's cursor commit yet was also absent. Sender-side WAYLAND_DEBUG lines record request construction, not wire flush or server dispatch. Therefore the two client traces do not establish the exact cross-connection ordering at Sway. The serial explanation is supported by matching source, but a compositor-side trace remains the missing causal evidence.

Relevant inspected sources:

- `crates/lumine-desktop/src/application.rs`, `Host::draw` around lines 500–590: settle UI geometry, synchronize cancellation, flush scene, render/present, drop scene borrow, then update candidate rectangle. The same function retains caret deduplication and X11 baseline/Wayland rectangle coordinate distinctions.
- The same file, `WindowManager::about_to_wait` and `Host::update`: ordinary drawing follows the native event batch; `RedrawRequested` also draws. It is **not** called after every IME callback. A blocking preedit-handler probe stresses related main-thread behavior but does not exactly reproduce this batching.
- `crates/lumine/src/scene.rs:1306`, `Scene::prepare_layout`, and `widgets.rs`, `Ui::try_prepare_frame`: authoritative layout/world geometry is ready before GPU rendering; render completion does not compute the caret's logical rectangle.
- Local winit 0.30.13 registry source `src/platform_impl/linux/wayland/window/state.rs:1038`: `set_ime_cursor_area` requests `set_cursor_rectangle` and `text_input.commit`. It does not explicitly flush. `event_loop/mod.rs:284` flushes the connection before polling. Mesa/softbuffer may also affect shared-connection flushing; the current evidence does not identify each flush boundary.
- Local wayland-protocols 0.32.12 `protocols/unstable/text-input/text-input-unstable-v3.xml:257`: rectangle coordinates are surface-local and applied by text-input commit, independently of `wl_surface` presentation.
- `/tmp/lumine-ime-upstream-research/sway-1.11.c:213`: client state commits relay IM state and send `done`, including cursor-only changes.
- `/tmp/lumine-ime-upstream-research/wlroots-0.19.2.c:87` and `:499`: IM commit with a noncurrent serial discards pending state and returns; sending IM `done` increments current_serial.
- `/tmp/lumine-ime-upstream-research/fcitx-5.1.19.cpp:168` and `:645–691`: ordinary `done` increments the engine serial without republishing current preedit; preedit updates call `commit(serial_)`.
- `/tmp/lumine-ime-upstream-research/protocol.xml:298`: input-method commit serial is checked against the number of issued IM done events. IM serials and application text-input done serials are separate counters, even though both happen to be 2 at this point.

Moving candidate synchronization before GPU work is semantically valid **after** successful `try_prepare_frame` and the second cancellation/permission synchronization, outside scene borrows, preserving coordinate conversion, focus gating, and deduplication. It can remove render/present latency from request construction. It is not a proven serial fix: constructing the commit sooner does not guarantee flushing it sooner, timely Fcitx dispatch of the next done, or acceptance of queued serial-2 commits. It might improve or worsen the race depending on ordering. Treat it as a testable latency/refactoring option, not recovery logic or a correctness guarantee.

The existing raw probe differs materially: it presents white initially and services IME state quickly; its 40/80 ms cursor-delay control is a nonblocking, coalesced deadline that continues dispatching events. Those successes do not exclude a synchronous main-thread/presentation-order interaction in the real host.

The staged diagnostic copy preserves all existing flags and existing log formats. New controls:

- `--preedit-block-ms 0|40|80` (default 0): synchronous thread sleep during focused preedit callbacks, including empty preedit updates. This deliberately stalls event dispatch/return to the outer flush loop.
- `--preedit-block-position before-cursor|after-cursor` (default before-cursor).
- `--present-on-ime` (default off): white softbuffer presentation after the blocking interval. Before mode executes block → optional present → cursor request; after mode executes cursor request → block → optional present. Initial redraw remains intact. This exercises surface request ordering, not GPU rendering cost.
- `CONFIG`, `BLOCK_BEGIN/END`, and `PRESENT_BEGIN/END` log configured and observed timings. `RECT_REQUEST` and `RECT` retain their existing meanings. With nonzero `--cursor-delay-ms`, “after-cursor” means after scheduling/requesting the rectangle, not necessarily after its eventual application. Use cursor delay 0 for the primary ordering experiment.

Next actionable experiment, only after the performance run: retain the unchanged 90 ms requested burst injection and cold/warm conditions; compare moving and fixed geometry under zero/40/80 ms synchronous stalls, before/after order, first without and then with surface presentation. Capture Sway server-side protocol dispatch in addition to both client streams, so IM done issuance and rejected commit ordering can be reconstructed. Score final preedit and committed Unicode separately; missing intermediate preedits alone are not failure. If this reproduces the original symptom, validate a controlled real-host pre-GPU/post-GPU A/B with layout/render/present/cursor/flush timestamps before making any production change. Do not replay committed text/deletions as a workaround; there is no per-transaction success acknowledgement.

Source provenance for the temporary diagnostic:

- Derived from `crates/lumine-desktop/examples/wayland_ime_probe.rs`, SHA256 `faf47184d39e68085fa418683eb405e6a184c6ef931704e91ff2b8a6c1a791ab`.
- Staged copy `/tmp/lumine-wayland-ime-blocking-probe.rs`, SHA256 `498d4825cf3adf7e317efbd9ce67725d64a420ad5a7103fd8bd96d93db63ad8e`.
- No executable corresponds to the staged copy yet. No outcome is claimed for the proposed experiment.
