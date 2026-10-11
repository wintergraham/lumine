# Native declarative event hooks

The owned Xvfb/Openbox run in `results.json` exercises real X11 keyboard events
through the desktop host. The intercepted editor blocks printable `x` and
Ctrl+C/X/V through its declarative `on_event` callback. Space still inserts text.
The ordinary editor then proves that blocked copy did not replace the clipboard,
and that ordinary copy, cut, paste and subsequent typing remain functional.

`event-hooks.log` records the ordinary model becoming empty after cut, returning
to `seed` after paste, and ending at `seed ok`. The intercepted model ends at
`ab ` with exactly one intercepted printable key and each clipboard shortcut.
`event-hooks.png` captures both fields before timed shutdown.

Reproduce from the repository root:

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo build -p lumine-desktop --example event_hooks
python3 scripts/event_hooks_smoke.py /tmp/lumine-target/debug/examples/event_hooks --output docs/platform-validation/event-hooks
```

This validates X11 keyboard/clipboard routing; it is not native macOS or Wayland
input evidence, nor a GPU performance measurement.

The final run rebuilt the example after the listener-ordering and editing-default
cancellation audit. `metadata.json` records its SHA-256, exact build/smoke commands
and compiler version. `source.tar.gz` contains 111 source/build-input files;
`source-manifest.json` records their hashes, all rechecked unchanged after the
build and smoke. Copies of both Python runner modules preserve the smoke harness.
This final archived run supersedes the earlier smoke performed before that audit.
