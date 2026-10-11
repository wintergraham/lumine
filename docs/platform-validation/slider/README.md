Native declarative slider validation
===================================

Run from the repository root:

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 cargo build -p lumine-desktop --example slider
python3 scripts/slider_smoke.py /tmp/lumine-target/debug/examples/slider --output docs/platform-validation/slider
```

The script owns and cleans up its Xvfb display and Openbox window manager. Six
native button-triggered snapshots verify:

1. Pointer dragging to approximately 75% on the original 420px control.
2. Tab focus followed by Home and Right sets the model to 1.
3. Tab focus followed by End and Left sets the model to 99.
4. Reactive width expansion to 580px updates pointer mapping (75% drag again).
5. Disabled pointer and keyboard input leave the model unchanged.
6. An external write still updates the disabled model to 25.

The close callback logs final bounds and accessibility semantics. `slider.log`
contains the assertions' source values; `result.json` records validated behavior.
The visually inspected `slider.png` shows the expanded track, thumb at 25%,
disabled opacity, and the focused report button.

This is Linux X11 software-rendered validation, not native macOS, hardware GPU
performance evidence, or a screen-reader interaction test.
