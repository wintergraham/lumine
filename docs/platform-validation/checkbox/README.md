Native declarative checkbox validation
=====================================

Run from the repository root:

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 cargo build -p lumine-desktop --example checkbox
python3 scripts/checkbox_smoke.py /tmp/lumine-target/debug/examples/checkbox --output docs/platform-validation/checkbox
```

The script creates and cleans up its own Xvfb display and Openbox window manager.
It clicks the label to check the model, presses Space to uncheck, disables the
control and attempts a suppressed click, then writes the model through a separate
button. The final model is checked and disabled with exactly two activations.
The close callback records the control bounds and checkbox semantics in
`checkbox.log`; `result.json` records asserted behaviors.

`checkbox.png` was visually inspected: the checked indicator, inherited 22px
label, disabled styling, and focused external-update button render correctly.
This is Linux X11 software-rendered validation. It does not establish native
macOS behavior, hardware GPU performance, or screen-reader interaction.
