Native horizontal scrolling validation
======================================

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 cargo build -p lumine-desktop --example horizontal_scroll
python3 scripts/horizontal_scroll_smoke.py /tmp/lumine-target/debug/examples/horizontal_scroll --output docs/platform-validation/horizontal-scroll
```

The owned Xvfb/Openbox session presents eight 80px buttons in a padded 240px
horizontal viewport (224px inner width). Native Tab reaches the eighth item,
revealing it at offset 416; Shift+Tab returns to the first at offset zero.
Two X11 button-7 wheel events then move horizontally to offset 112 without
changing the first item's focus or snapping back. After keyboard focus returns
to the last item, clicking the resize button grows the viewport to 320px and
clamps the offset to the new maximum of 336.

Read-only `r` reports record offset and focused target identity. Pixel samples
assert 80 visible focus-colored columns for both revealed items, then zero when
manual scrolling hides the focused item and when the resize button takes focus.
No application code writes scroll offsets or invokes reveal. Stage 1 was
visually inspected; screenshots, native logs, and `result.json` retain evidence.
All owned processes are cleaned up.

This validates Linux X11 software rendering and button-7 horizontal input. It
does not validate macOS trackpads, hardware GPUs, or screen-reader interaction.
