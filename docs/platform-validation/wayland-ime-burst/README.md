# Direct winit input-method burst diagnostic

A direct winit/softbuffer window, independent of lumine editors and rendering,
received every final preedit and Unicode commit in this bounded experiment:
**12/12 baseline bursts** and **10/10 cursor-delay sweep bursts**. Some intermediate
preedit states did not arrive, but the final state did. This does not reproduce
or resolve the stuck final preedit observed in the
[real lumine failure](../wayland-ime/failed-initial/README.md).

The baseline alternates moving/fixed candidate geometry across two fresh sessions
per mode, with one cold and two warm bursts per session. The sweep uses moving
geometry with requested delays of 0, 16, 40 and 80 ms, plus a fixed-geometry
control, each with one cold and one warm burst. Delayed geometry coalesces the
latest target while retaining the first deadline. Initial focus geometry is
immediate. Fixed geometry is set once rather than recommitting identical values.

Each burst injects `nihao` with xdotool's requested delay of 90 ms, observes for
two seconds, and then commits with Space. Recorded wall times describe the
actual injection interval; the option is not assumed to produce precisely
90 ms between each character. Escape/F8 resets between bursts without switching
input methods or moving focus. Both native protocol streams and application
logs are retained, including actual rectangle-request/application timestamps.

[Baseline results](baseline/result.json) and [delay-sweep results](delay-sweep/result.json)
are diagnostic outcomes, not broad input-reliability or performance evidence.
Each directory contains the exact harness snapshot, executable hash, logs,
source manifest and source archive. Both archives preserve 162 inputs; the
baseline substitutes the original probe/harness snapshots rather than the later
live versions. Binary hashes were checked against the frozen baseline and
current probe executables. The probe builds, passes focused strict Clippy, and
cross-checks for macOS ARM64; native macOS execution is not established.

## Protocol investigation

The failed lumine trace contains full engine-side preedit requests but only the
first client-side preedit. Thus the missing events did not reach the editor
handler. Serial mismatch is a supported explanation, not a complete experimental
attribution of the original failure.

In matching installed sources, Fcitx 5.1.19's ordinary `done` callback advances
its serial without re-sending the current preedit. Its preedit update sends
`commit(serial_)`. [Fcitx source](https://github.com/fcitx/fcitx5/blob/5.1.19/src/frontend/waylandim/waylandimserverv2.cpp#L168)
Sway 1.11 relays client state commits to the input method and emits `done`,
including cursor-only state changes. [Sway source](https://github.com/swaywm/sway/blob/1.11/sway/input/text_input.c#L213)
wlroots 0.19.2 advances its serial when sending `done`; mismatched input-method
commits discard pending state without emitting the commit signal.
[wlroots source](https://gitlab.freedesktop.org/wlroots/wlroots/-/blob/0.19.2/types/wlr_input_method_v2.c#L87)
The [protocol](https://github.com/fcitx/fcitx5/blob/5.1.19/src/lib/fcitx-wayland/input-method-v2/input-method-unstable-v2.xml#L298)
constrains state replacement by the issued-done count.

No production workaround was added. Suppressing or delaying candidate geometry
may place it incorrectly, and replaying committed text or deletion can duplicate
operations because there is no per-commit success acknowledgement. Re-publishing
replaceable preedit state after an updated serial is an upstream investigation
option, but focus transitions and feedback loops require validation. This
experiment does not establish a safe general retry mechanism.
