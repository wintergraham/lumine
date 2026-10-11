Native declarative image validation
==================================

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 cargo build -p lumine-desktop --example images
python3 scripts/images_smoke.py /tmp/lumine-target/debug/examples/images --output docs/platform-validation/images
```

An owned Xvfb/Openbox session displays two reactive images and one static yellow
swatch. A native button changes immutable image sources and explicit allocation.
The script checks the presented pixels horizontally and vertically, including
four-pixel padding and the static swatch, at every stage:

| Stage | Source | Intrinsic content | Allocated content |
| --- | --- | --- | --- |
| 0 | Red | 80 × 40 | 192 × 72 |
| 1 | Blue, same size | 80 × 40 | 192 × 72 |
| 2 | Green, larger | 120 × 60 | 192 × 72 |
| 3 | Green, same source | 120 × 60 | 292 × 72 |
| 4 | Original red source | 80 × 40 | 192 × 72 |

`images-0.png` through `images-4.png` retain the native screenshots. Stage 3 was
visually inspected. `result.json` records pixel measurements and `images.log`
records source transitions, final bounds, and accessible image labels/roles.
The script cleans up its own application, window manager, and display server.

This validates Linux X11 software-rendered presentation, not native macOS,
hardware GPU performance, or screen-reader interaction. Dependencies include
Xvfb, Openbox, xdotool, xwininfo, and Pillow.
