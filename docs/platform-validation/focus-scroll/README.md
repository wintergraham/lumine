Native focus reveal validation
==============================

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 cargo build -p lumine-desktop --example focus_scroll
python3 scripts/focus_scroll_smoke.py /tmp/lumine-target/debug/examples/focus_scroll --output docs/platform-validation/focus-scroll
```

The script owns an Xvfb/Openbox session and performs native keyboard/wheel input:

1. Eight Tab presses focus the last of eight 40px buttons. The framework scrolls
   to offset 176, fully exposing that button in the 144px content viewport.
2. Seven Shift+Tab presses return to the first button and offset zero.
3. Two wheel-down events move to offset 112 while the first button remains
   focused. Pressing the read-only report key does not snap the scroll back.

At each stage, pressing `r` logs the offset and the keyboard event's focused
target without moving focus. The application never calls reveal or writes the
scroll offset. Native screenshots assert exactly 40 visible focus-colored rows
at the bottom/top for stages 1/2 and zero after manually scrolling the focused
first button out of view. Stage 1 was visually inspected.

`focus_scroll.log` records target identity and offsets; `result.json` records
pixel checks; `focus-scroll-1.png` through `focus-scroll-3.png` retain screenshots.
All owned processes are cleaned up. This is Linux X11 software-rendered evidence;
it does not cover nested reveal, macOS, hardware GPUs, or screen readers.
