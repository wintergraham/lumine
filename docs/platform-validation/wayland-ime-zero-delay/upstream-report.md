# Draft upstream report: stale input-method serial leaves final Pinyin preedit behind

Not submitted externally.

A direct winit/softbuffer application, without lumine dependencies, reproduces a final preedit that remains `n` after typing the complete `nihao` burst. The input method produced `ni hao`, but its final transaction reached the compositor after the compositor advanced its input-method serial. The same symptom and ordering occur in the real lumine application. Pressing Space subsequently commits `你好` correctly in both cases; **lost committed text was not observed**.

## Versions and conditions

- winit 0.30.13, wayland-client 0.31.15, softbuffer 0.4.8.
- Fcitx5 5.1.19 (Ubuntu package 5.1.19-1); fcitx5-pinyin 5.1.12-1.
- Sway 1.11, libwlroots-0.19 0.19.2-1; libwayland client/server 1.24.0-2.
- Private Sway session using its X11 backend, pixman renderer, owned Xvfb/Openbox host; native Wayland client has DISPLAY unset. Private D-Bus, XDG paths, and Pinyin profile. Complete versions are in `tool-versions.json`.
- Standalone moving cursor rectangle follows preedit length, deduplicates identical rectangles, and presents a white softbuffer surface on each preedit. No blocking delay or scheduled cursor delay; `--present-on-ime` is enabled. This fixture has no lumine dependency.
- One fresh session per application; two bursts in each session. First/cold burst delivered all preedits. Second/warm burst reproduced the failure in both applications. Cold means a fresh Fcitx process/data directory, not a cold OS cache.

## Expected and actual

Expected: after input-method transactions settle, the application receives final `ni hao` preedit. Actual: after the warm burst, it receives only `n` and remains at that preedit throughout the two-second observation interval. The input method emitted `n`, `ni`, `ni h`, `ni ha`, `ni hao`. Space, sent after the observation, commits exactly `你好`.

The burst uses `xdotool type --clearmodifiers --delay 0 nihao` into the Sway host window; input passes through the native Wayland input-method protocol. There are no synthesized application IME events, character acknowledgements, or primer. Harness observation completes normally while recording `delivery_failure` for both warm trials; exit status alone is not the oracle.

## Standalone reproduction

From the repository root, use the archived standalone package and harness:

```
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 cargo build --manifest-path docs/platform-validation/wayland-ime-blocking/sources/probe/Cargo.toml --offline --locked
LD_LIBRARY_PATH=/tmp/lumine-ci-loader-check/build/loader /usr/bin/python3 docs/platform-validation/wayland-ime-zero-delay/sources/standalone_zero.py /tmp/lumine-target/debug/lumine-ime-blocking-probe --sessions 1 --bursts 2 --key-delay-ms 0 --present-on-ime --output /tmp/wayland-zero-standalone
```

The loader path is specific to this machine; standalone softbuffer does not itself require the lumine Vulkan loader. The archived executable was built from the identical `/tmp/lumine-ime-blocking-probe` package before this run. Source, lock, hash, and build evidence are in `../wayland-ime-blocking`; zero-delay harness copies and exact run provenance are here. Installed Xvfb/Openbox/Sway/Fcitx5/Pinyin/xdotool/grim are required. The helper is adjacent to the harness.

For the lumine comparison:

```
LD_LIBRARY_PATH=/tmp/lumine-ci-loader-check/build/loader /usr/bin/python3 docs/platform-validation/wayland-ime-zero-delay/sources/native_zero.py /tmp/lumine-target/debug/examples/native_ime --sessions 1 --bursts 2 --key-delay-ms 0 --output /tmp/wayland-zero-native
```

`../wayland-ime-native-burst` archives the rebuilt lumine binary provenance, source/assets, and locked build commands. Binary hashes for these runs match those preceding archives (`inputs.json`).

## Server ordering

These are server dispatch/send timestamps, not merely client-side marshal timestamps. Input-method serials are counted independently of text-input-v3 serials.

| Event | Standalone timestamp, line in `standalone/moving-delay0-1/sway.log` | lumine timestamp, line in `native/lumine-delay0-1/sway.log` |
| --- | --- | --- |
| Server receives `n` commit | 760615.776, line 683: `commit(8)` | 752796.412, line 3210: `commit(5)` |
| Cursor-state update leads to next IM `done()` | 760618.107, line 720: ninth done | 752800.965, line 3249: sixth done |
| Server receives final `ni hao` commit | 760627.192, line 746: stale `commit(8)` | 752805.578, line 3274: stale `commit(5)` |
| Later Space/Unicode commit | 762615.436, line 768: matching `commit(9)` | 754797.275, line 3298: matching `commit(6)` |

The `ni`, `ni h`, and `ni ha` transactions between `n` and the final preedit also use stale serials. `server-serial-analysis.json` retains every transaction and comparison; the analyzer source is included. Per-trial client, engine, server, and application log slices retain the exact before-Space and after-Space observations.

## Mechanism and limits

Copies of the previously inspected version-matched upstream source are under `upstream-source/`:

- `wlroots-0.19.2.c:87`: `im_commit` resets pending state and returns when the submitted serial differs from `current_serial`; sending input-method done increments that serial near line 499.
- `sway-1.11.c`: relays client text-input state commits to input-method done, including cursor updates.
- `fcitx-5.1.19.cpp`: ordinary done increments the engine serial; its normal done path does not republish the current preedit. Preedit publication commits using the engine's current serial.

This evidence establishes stale-serial rejection of final preedit in this stack and reproduces the symptom without lumine. It does not identify a uniquely responsible project, prove which protocol-level repair is appropriate, or establish failure on other compositors/input methods. No production workaround was applied, and no upstream issue was submitted. Replaying committed text or deletion transactions would require separate safety analysis; these runs do not justify it.

The nominal 90 ms burst matrix is archived separately: real lumine four final deliveries; blocking standalone twelve final deliveries. Zero-delay is a distinct stress condition, not a replacement for those results. Only one zero-delay cold/warm pair per application was run, and server instrumentation can affect scheduling. Blocking-handler/present experiments do not exactly model GPU rendering or event-loop batching. Moving cursor updates ahead of rendering remains an unproven timing workaround; it cannot explain away this reproduction in an application with no lumine renderer.
