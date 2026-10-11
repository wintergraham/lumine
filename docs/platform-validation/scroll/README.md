Native declarative scroll validation
===================================

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 cargo build -p lumine-desktop --example scroll
python3 scripts/scroll_smoke.py /tmp/lumine-target/debug/examples/scroll --output docs/platform-validation/scroll
```

The owned Xvfb/Openbox session exercises native wheel input and ordinary retained
children in a padded viewport. Five button-triggered model reports accompany
native screenshots:

| Stage | Operation | Content height | Viewport content height | Offset |
| --- | --- | ---: | ---: | ---: |
| 1 | Two native wheel-down events | 320 | 144 | 112 |
| 2 | External offset 10000 | 320 | 144 | 176 |
| 3 | Remove five keyed rows | 120 | 144 | 0 |
| 4 | External offset 10000 on short content | 120 | 144 | 0 |
| 5 | Restore rows, scroll bottom, enlarge viewport | 320 | 224 | 96 |

The script asserts each reported offset (stage 1 must be positive and bounded)
and every content pixel in a vertical sample away from label glyphs. Expected
row colors come from the reported offset; empty content and all four padding
edges must retain their background. This catches stale translations, padding
leaks, failed clamping, and stale removed-row paint.

`scroll-1.png` through `scroll-5.png` retain screenshots; the final resized view
was visually inspected. `scroll.log` contains reports and final semantics;
`result.json` contains checked pixel sample lengths. Application, window manager,
and display server are cleaned up by the script.

This is Linux X11 software-rendered validation. Nested scroll propagation,
macOS, hardware GPUs, and screen-reader interactions are outside this smoke's
scope. Dependencies include Xvfb, Openbox, xdotool, xwininfo, and Pillow.
