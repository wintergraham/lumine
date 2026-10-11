Native Wayland input validation
===============================

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 cargo build -p lumine-desktop --example form
LD_LIBRARY_PATH=/tmp/lumine-ci-loader-check/build/loader python3 scripts/wayland_input_smoke.py /tmp/lumine-target/debug/examples/form --output docs/platform-validation/wayland-input
```

The script owns an Xvfb server, Openbox, and Weston 14 using its X11 backend,
pixman renderer, and kiosk shell. X11 pointer/key events enter Weston's seat;
the application receives native Wayland events. The form's environment removes
`DISPLAY`, sets `WAYLAND_DISPLAY` to the private compositor socket, and enables
`WAYLAND_DEBUG`. Weston does not enable Xwayland. The kiosk shell makes the
application fill the output, giving deterministic input coordinates and a real
Wayland configure when the host output is resized from 900×650 to 1000×700.

The assertions cover pointer focus, selection replacement, Tab focus traversal,
clipboard transfer between editors, multiline entry, disabled input suppression,
reactive editor widths, and native resize. `protocol.log` must contain actual
pointer buttons, keyboard keys, resized `xdg_toplevel.configure`, and clipboard
selection requests. `form.log` supplies final model and allocated-width evidence.
Screenshots retain initial and final rendering. All owned processes are cleaned
up even when an assertion fails; the private runtime directory is then removed.

The final run passes every assertion, including clipboard copy/paste. Protocol
evidence includes `wl_data_device.set_selection`, `wl_data_offer.receive`, and
`wl_data_source.send`: a real compositor clipboard transfer. The final screenshot
was inspected: both editors display the copied name, the notes include the second
line, disabled styling is visible, and editor widths expand to 580px within the
resized native surface.

The test initially exposed the host's X11-only clipboard configuration. The host
now selects smithay-clipboard on an actual Wayland display and retains its owned
display handle until the worker exits; X11/macOS use arboard. See the
[upstream safety contract](https://docs.rs/smithay-clipboard/0.7.3/smithay_clipboard/struct.Clipboard.html).
The smoke retains hard clipboard assertions and removes stale `result.json`
before each run, so unsupported behavior cannot silently produce a passing result.

`metadata.json` records the native binary, loader, and source-archive hashes,
commands, and tool versions. Its `source_files_newer_than_native_binary` list
explicitly identifies source edits after the tested binary build; the archive is
a captured workspace snapshot rather than an unqualified exact-build claim when
that list is nonempty.

Dependencies: Weston 14 with kiosk-shell, Xvfb, Openbox, xdotool, and Pillow.
This validates a native Wayland client on a nested software-rendered compositor;
it does not establish physical-seat, hardware-GPU, IME, or macOS support.
