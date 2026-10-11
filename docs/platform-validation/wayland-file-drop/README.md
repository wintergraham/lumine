# Native Wayland external file drops

`accepted/result.json` passes an actual GTK3 `text/uri-list` drag into a native Wayland lumine window: two files, including a percent-encoded space, arrive in one grouped callback at the accepting region. The client and GTK source have DISPLAY unset. The owned Sway compositor runs on an owned Xvfb/Openbox desktop with a private D-Bus session; no user settings are modified.

The receiver clears hover when the drag leaves the target, and releasing outside the target produces no second drop. A third gesture drops onto a non-accepting region: the UI does not accept it and the protocol finish count remains unchanged. Protocol completion waits for the live UI callback's acceptance; a missing owner, rejected region, disconnected reply, or bounded reply timeout destroys the offer without claiming success. See `accepted/application.log` for the real Wayland receive/action/finish trace and `accepted/source.log` for GTK source events.

`failed-escape-attempt` intentionally preserves the earlier failed test. In this GTK3/Sway setup, injecting Escape did not end the compositor/source drag; releasing the mouse still delivered another drop. This is not represented as successful receiver cancellation. Receiver leave cancellation is verified; keyboard cancellation also depends on source/compositor behavior.

The transport bounds encoded URI-list payloads to 1 MiB, local paths to 1,024, and nonblocking transfer reads to three seconds. Invalid data, overflow, disconnect, and timeout reject the complete transfer. `transport-unit.log` passes the URI-list decoder and real nonblocking Unix-stream stalled/oversized/disconnected tests. Those tests ran before the final UI acceptance acknowledgement addition; their decoder/polling implementation was unchanged. The final native run verifies acceptance and rejection through the acknowledgement path. `clippy.log` is a passing strict native-only desktop check.

Reproduce after building the drag_drop example:

```sh
LD_LIBRARY_PATH=/tmp/lumine-ci-loader-check/build/loader /usr/bin/python3 scripts/wayland_file_drop_smoke.py /tmp/lumine-target/debug/examples/drag_drop --output /tmp/lumine-wayland-files-accepted
```

The loader path is this validation environment's Vulkan loader; use the normal installed loader where appropriate. Required desktop tools include Sway, Xvfb, Openbox, xdotool, xwininfo, grim, dbus-daemon, Python GI, and GTK3. `sources` snapshots the relevant files after validation while the wider workspace was developing concurrently; this is not a hermetic full-workspace build claim. The binary hash records the captured executable. macOS runtime behavior is outside this Linux evidence packet.
