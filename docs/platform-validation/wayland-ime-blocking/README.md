# Blocking preedit diagnostic

All 12 observations delivered final `ni hao` preedit and exactly one `你好` commit. This standalone winit/softbuffer diagnostic did **not** reproduce the previous lumine final-preedit failure. No production fix follows from this result.

| Preedit-handler configuration | Moving cold/warm | Fixed cold/warm |
| --- | --- | --- |
| No blocking, present before cursor request | Delivered / delivered | Delivered / delivered |
| Block 80 ms, present, then cursor request | Delivered / delivered | Delivered / delivered |
| Cursor request, block 80 ms, then present | Delivered / delivered | Delivered / delivered |

Every configuration enables `--present-on-ime`; the timer-based cursor delay is zero. Each moving/fixed pair uses independent private Sway/Fcitx processes, followed by two bursts in the same focus and engine. Cold means a fresh Fcitx process/data directory, not a cold operating-system cache. Each burst uses unchanged `xdotool type --clearmodifiers --delay 90 nihao`, with no per-character acknowledgement. The recorded injection wall time is approximately 230 ms; the nominal xdotool delay must not be interpreted as a measured 90 ms gap between received key presses. The harness observes preedit for two seconds before sending Space, then records the Unicode commit.

## Server evidence

The moving cold baseline received only `n` and `ni hao`; missing intermediate states were real stale input-method transactions:

- `baseline-0/moving-delay0-1/sway.log:486`: server receives `n`, `commit(2)` at 325935.646 ms.
- Line 517: server issues its third input-method `done()` at 325940.257 ms.
- Lines 520, 533, 536: server receives `ni`, `ni h`, `ni ha` with stale `commit(2)` at 325944.474, 325944.560, 325944.577 ms.
- Line 559: final `ni hao` arrives with matching `commit(3)` at 325972.991 ms.
- Line 592: Unicode commit arrives with matching `commit(4)` at 328019.777 ms.

The matching wlroots 0.19.2 implementation discards input-method commits whose serial differs from its current serial. This trace therefore supports rejection of those three intermediate transactions, not loss of the final preedit or committed text. `server-serial-analysis.json` records all server-side input-method commit comparisons: three mismatches in that session, zero in the other five. The analysis counts issued input-method `done()` events from the start of each complete server log; the separate text-input-v3 serial is not used as an input-method serial.

Blocking in an IME callback and presenting a white softbuffer surface do not reproduce the full lumine layout/render/GPU batching path. The experiment changes timing and connection flush behavior, and has only one cold/warm pair per configuration. Zero final failures in this matrix is not evidence that the earlier failure was fixed or cannot recur. Moving application cursor synchronization ahead of GPU work remains an unproven timing hypothesis.

## Reproduction and provenance

`commands.json` contains the exact run commands. `sources/probe` contains the standalone Cargo package and its pruned lock; all 201 registry packages match the archived root lock by name, version, source, and checksum (`lock-verification.json`). `build.log`, `tool-versions.json`, and `source-manifest.json` record the build and hashes. All three run provenance files match the same archived harness and binary hashes. `sources/platform_smoke.py` is the exact shared process-cleanup helper, adjacent to the archived executable harness for portable imports.

`summary.json` aggregates the twelve outcomes. Every condition retains client, input-method, and compositor protocol logs, per-trial slices before/after Space, application logs, screenshots, private configuration, and provenance. `pre-experiment-review.md` is the earlier hypothesis review, preserved as written before this experiment; its references to an unbuilt draft describe that earlier point in time.
