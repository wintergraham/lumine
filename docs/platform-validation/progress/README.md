Native declarative progress validation
=====================================

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 cargo build -p lumine-desktop --example progress
python3 scripts/progress_smoke.py /tmp/lumine-target/debug/examples/progress --output docs/platform-validation/progress
```

The script owns its Xvfb/Openbox session. It drives the example's state button
through seven states and checks native screenshot pixels across the middle of
the progress control. Pixel extents must be contiguous from the content's left
edge; padding remains the track color. No clicks target the progress indicator.

| Stage | Outer width | Value | Visible fill pixels |
| --- | ---: | ---: | ---: |
| 0 | 400 | 0 | 0 |
| 1 | 400 | .25 | 98 |
| 2 | 400 | 1 | 392 |
| 3 | 600 | 1 | 592 |
| 4 | 600 | .25 | 148 |
| 5 | 200 | .25 | 48 |
| 6 | 200 | NaN → 0 | 0 |

`progress-0.png` through `progress-6.png` record those stages. The expanded
quarter-fill screenshot was visually inspected. `result.json` stores pixel
measurements; `progress.log` records normalized model values and final progress
accessibility semantics. The fill inherits its color from the parent container.

Client coordinates come from `xwininfo`'s absolute client position so window
manager decoration offsets do not contaminate pixel measurements. Dependencies
include Xvfb, Openbox, xdotool, xwininfo, and Pillow.

This validates native Linux X11 software-rendered presentation, not macOS,
hardware GPU performance, screen-reader interaction, or layout work counts.
