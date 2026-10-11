Native component overlay validation
===================================

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 cargo build -p lumine-desktop --example overlays
python3 scripts/overlays_smoke.py /tmp/lumine-target/debug/examples/overlays --output docs/platform-validation/overlays
```

The recorded run used the isolated upstream Vulkan Loader 1.4.345 through
`LD_LIBRARY_PATH=/tmp/lumine-ci-loader-check/build/loader`, preserving debug
and validation. The script inherits the caller's loader configuration and owns
its Xvfb/Openbox session.

Eleven native snapshots verify anchored popup opening, automatic following of
an anchor's compositor scroll, window-resize flipping, Escape dismissal with
focus restoration, modal Tab containment, nested popup focus ownership, and
outside-click dismissal for both modal and popup. Popup positions are asserted
at y=188, y=148 after a 40px anchor scroll, and y=8 after reducing window height
to 240px. Selected panel pixels verify native presentation after movement and
inside nested overlays. Stage 7 was visually inspected.

The example uses public component children and styles. Its `s` instrumentation
key writes only the scroll model; framework placement follows automatically.
Its read-only `r` handlers attach inside each focus scope because modal input
correctly stops propagating outside that scope. Reports include actual focused
control labels and scene bounds. No application code invokes reveal or writes
scene geometry.

`overlays.log`, eleven screenshots, and `result.json` retain the evidence. All
owned processes are cleaned up. This is Linux X11 software-rendered evidence,
not macOS, hardware GPU performance, or screen-reader validation. Dependencies
include Xvfb, Openbox, xdotool, xwininfo, and Pillow.

The final build passed all eleven stages again after the focus reentrancy and
logical disabled-state fixes. A separate owned Xvfb session reran the declarative
form smoke: pointer focus, selection replacement, clipboard, Tab, multiline input,
disabled input, reactive width and native resize passed. Its log and screenshot
are retained as `form-regression.*`.

The integrated workspace passes 307 tests with one opt-in GPU stress test ignored,
23 doctests, strict Clippy, formatting and the macOS ARM64 all-target cross-check.
Check logs, `metadata.json` and the Rust source archive record this integration.
The loader was built using the checked-in CI helper in a new private directory;
no system library was replaced. Khronos Vulkan validation layers were not installed,
so preserving validation flags is not evidence that that layer ran.
