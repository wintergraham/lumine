# Live native Wayland DPI transitions

One running lumine window survives output scale **1 → 2 → 1** in an owned Sway
session. The client connects through Wayland with `DISPLAY` removed. Sway uses
its X11 backend inside a private Xvfb/Openbox display; physical pointer and
keyboard events enter through that display. Every process and IPC socket belongs
to the runner and is cleaned up when it exits.

The physical output stays 1200×800 while the logical client viewport changes
1200×800 → 600×400 → 1200×800. The editor remains 400×44 logical pixels. Clicking
its left edge at correspondingly scaled physical coordinates and typing inserts
`A`, `B`, then `C` at offset zero: the final model is `CBA0123456789`. After each
insertion the selection is offset 1 and the logical caret is at (37.6, 62), with
size 1×23. Pixel assertions verify that the editor's solid interior spans
398 → 796 → 398 physical pixels. All three screenshots remain 1200×800.

The protocol log independently records fractional scale notifications
120 → 240 → 120 and matching logical viewport destinations. `results.json`
records the compositor output state, one client PID, models, selection, caret
and pixel measurements. This exercises an actual live platform event transition,
not a forced initial scale or a direct call to a renderer resize function.

Reproduce from the repository root (requires Sway, Xvfb, Openbox, xdotool,
xwininfo, grim and Python Pillow):

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo build -p lumine-desktop --example dpi_transition
python3 scripts/dpi_transition_smoke.py /tmp/lumine-target/debug/examples/dpi_transition --output docs/platform-validation/dpi-transition
```

Sway's runtime output command is implemented in its
[output scale handler](https://github.com/swaywm/sway/blob/master/sway/commands/output/scale.c).
The nested compositor uses pixman and the application uses Mesa software Vulkan.
This validates native Wayland scaling, hit testing and rendering, without claiming
physical monitor hotplug, fractional non-integer scaling, hardware GPU performance
or native macOS validation.

`metadata.json` records the binary hash, archive hash, compiler and commands;
`source.tar.gz` preserves 115 Rust/shader/build inputs. Their SHA-256 manifest
was checked unchanged across the final build and native run. The runner modules
are copied alongside the evidence. The archived host includes the authoritative
native scale cache and backend-specific pointer conversion fix.
