Native overlay scrollbar validation
===================================

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 cargo build -p lumine-desktop --example scrollbars
python3 scripts/scrollbars_smoke.py /tmp/lumine-target/debug/examples/scrollbars --output docs/platform-validation/scrollbars
```

The script owns an Xvfb/Openbox session. It clicks the vertical track to page
144px, drags the thumb outside the viewport to verify capture and the 176px
maximum, then sends Home, End, PageUp and PageDown to the focused scrollbar.
A horizontal thumb drag reaches the 416px maximum. Shrinking both content trees
to two children clamps their offsets to zero and hides both bars.

The read-only report key records both offsets without changing focus. Native
screenshots check visible accent or focused text-colored thumb pixels at each stage and no thumb
pixels and restored track background after overflow disappears. Stage 7 was
visually inspected. Model reports assert each expected offset.
Screenshots, `scrollbars.log`, and `result.json` retain evidence.

Dependencies: Xvfb, Openbox, xdotool, xwininfo, and Pillow. This is Linux X11
software-rendered validation, not macOS, hardware GPU, or screen-reader evidence.
