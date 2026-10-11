# Held-key focus transfer on X11

The smoke test owns an Xvfb server, Openbox and a two-window lumine process. It
disables X server key repeat for the test so holding a key introduces exactly one
real key press. It then verifies:

- Holding `x` in the source editor while activating the target window does not
  insert the synthetic focus-gain press into the target editor. A real `b` works.
- Holding Space in the source editor while activating the target's focused button
  does not activate that button on release. A subsequent genuine Space activates
  the button once.

The final model is source `"x "`, target `"b"`, and one target action. The log,
screenshot and results are adjacent; results include hashes of the executable and
test script. This is actual X11 input through xdotool, not direct scene dispatch.
It does not establish macOS behavior or hardware GPU coverage.

```sh
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 \
  cargo build -p lumine-desktop --example held_keys --offline
LD_LIBRARY_PATH=/tmp/lumine-ci-loader-check/build/loader \
  python3 scripts/held_keys_smoke.py /tmp/lumine-target/debug/examples/held_keys \
  --output docs/platform-validation/held-keys
```

The Vulkan loader override is the same loader used by the Linux native validation
suite. The script inherits it and otherwise creates its own isolated display.
The archived run used the host's synthetic-event filter; no pre-fix run is claimed.

Winit 0.30.13 documents synthetic focus-transfer key events in `src/event.rs`
(`WindowEvent::KeyboardInput::is_synthetic`). Its X11 event processor synthesizes
held presses/releases through its ordinary key processor. lumine ignores these
reconciliation events rather than treating them as new typing or actions.

The final run includes a pinned `test-runner.py`. A complete Rust source and Cargo
manifest snapshot is archived at `../live-dpi-pointer/source.tar.gz`, with a
per-file source manifest alongside it and the archive hash in each results file.
