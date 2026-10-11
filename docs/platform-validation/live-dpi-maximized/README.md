# Maximized X11 live-DPI transition

This is the same owned-XSETTINGS stationary-pointer regression as
[`../live-dpi-pointer`](../live-dpi-pointer), run with `--maximized`. The runner
double-clicks the native title bar and verifies the window manager's maximized
state before changing `Xft/DPI` from 96 to 192.

Openbox keeps the physical window at 1400×981 rather than doubling its size. The
host nevertheless receives the needed native resize notification and updates its
logical viewport from 1400×981 to 700×490.5. The stationary click and wheel both
reach the correct left control. No extra deferred-resize host change was needed
for this observed window-manager refusal path.

```sh
LD_LIBRARY_PATH=/tmp/lumine-ci-loader-check/build/loader \
  python3 scripts/live_dpi_pointer_smoke.py \
  /tmp/lumine-target/debug/examples/live_dpi_pointer --maximized \
  --output docs/platform-validation/live-dpi-maximized
```

The source archive in `../live-dpi-pointer/source.tar.gz` covers this run too;
both result files identify the same executable. This validates Openbox under
Xvfb, not every tiling/window-manager policy or physical monitor migration.
