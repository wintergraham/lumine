# Reproduced live X11 DPI failure

An owned Xvfb/Openbox session runs `live_dpi_pointer` and a minimal script-local
XSETTINGS provider. The provider publishes the standard `Xft/DPI` setting and
changes it from 96 to 192. Winit receives the real property notification and
emits its native scale/resize events. No lumine input or scale event is injected.

The physical window doubles from 500×200 to 1000×400. The pointer remains
stationary over physical client coordinate (300,60), which should become logical
(150,30). A click and wheel should therefore reach the left control. Instead the
archived run sends both to the right control and reports a final logical viewport
of 1000×400, rather than retaining 500×200. The log and failing results record this.

This revealed two host issues: cached logical pointer coordinates were not
rescaled on X11, and subsequent resizing used a stale `Window::scale_factor()`
after Winit's live XSETTINGS notification. This is an X11-specific observation;
Wayland/macOS start with logical pointer coordinates and must preserve them.

The archived `test-runner.py` matches its result hash. The executable hash is in
`results.json`; a frozen copy was kept at `/tmp/lumine-live-dpi-pointer-before` during
this development session. The script imports the repository's `platform_smoke`
process-cleanup helper. The post-fix comparison is recorded separately.
