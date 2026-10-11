# Real lumine burst diagnostic

The freshly rebuilt `native_ime` application delivered final `ni hao` preedit and exactly one `你好` commit in all four observations: two independent sessions, each with a cold and warm burst. The original stuck-final-preedit failure was not reproduced. No production changes were made.

This adapts the independently archived blocking-probe harness to the existing lumine fixture: no probe CLI arguments, actual `lumine native IME` title, wait for `MODEL`, click `(100,82)`, enable Pinyin, then use the original unsynchronized `xdotool type --clearmodifiers --delay 90 nihao`. No primer or character acknowledgement is used. The first editor stays focused; Escape/Ctrl+A/BackSpace resets its model between bursts. The two-second observation period and subsequent Space collect preedit and commit evidence separately. Every process/configuration is private, the application has DISPLAY unset, and client, input-method, and server protocol traces are retained. No injected blocking or synthetic IME events are used.

## Server ordering

Session 2 cold delivered only `n` and `ni hao`. Its server trace confirms rejection of three intermediate transactions, with successful final recovery:

| Event | Server timestamp (ms) | `run/lumine-delay0-2/sway.log` line |
| --- | --- | --- |
| `n` input-method `commit(2)` | 635799.044 | 3017 |
| Server's third input-method `done()` | 635805.724 | 3045 |
| `ni` stale `commit(2)` | 635808.541 | 3048 |
| `ni h` stale `commit(2)` | 635808.628 | 3061 |
| `ni ha` stale `commit(2)` | 635808.644 | 3064 |
| `ni hao` matching `commit(3)` | 635844.642 | 3089 |
| `你好` matching `commit(4)` | 637891.594 | 3125 |

The matching wlroots implementation discards commits with a stale input-method serial. `server-serial-analysis.json` counts issued input-method done events from the complete server traces: session 1 has no mismatches, session 2 has these three. Every warm burst delivered all five preedit states. Periodic model/display logs additionally retain the resulting editor text. This narrows the unresolved question to the timing that leaves the **final** preedit stale; it does not establish lost Unicode commits or justify a cursor/render-order fix.

## Build and replay

```
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 cargo build -p lumine-desktop --example native_ime --locked
LD_LIBRARY_PATH=/tmp/lumine-ci-loader-check/build/loader /usr/bin/python3 docs/platform-validation/wayland-ime-native-burst/sources/wayland_native_burst.py /tmp/lumine-target/debug/examples/native_ime --sessions 2 --bursts 2 --output docs/platform-validation/wayland-ime-native-burst/run
```

`compiled-sources.tar.gz` contains workspace crates, Cargo files, and external include_bytes/include_str inputs. Source hashes before/after the build and after the run are retained. External included assets were additionally hashed before/after a post-run locked verification build, whose binary remained identical to the tested binary. `binary-provenance.json`, the run's `provenance.json`, build logs, tool versions, archived harness/helper, and artifact manifest bind these results to the tested inputs. `run/result.json` retains all individual observations without converting failures into successes. The harness exits successfully when collection completes; consumers must inspect each `outcome` and `delivery_failures` field.

This is a small diagnostic sample. Cold means a new Fcitx process and data directory, not a cold OS cache; server tracing and archive instrumentation can affect scheduling. The earlier failed-initial evidence remains valid and separate.
