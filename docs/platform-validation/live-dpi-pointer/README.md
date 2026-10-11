# Live X11 DPI and stationary pointer

The isolated smoke starts Xvfb, Openbox, a minimal XSETTINGS provider and the
`live_dpi_pointer` example. The provider changes standard `Xft/DPI` from 96 to 192.
Winit receives the native property notification, emits `ScaleFactorChanged`, and
requests the larger physical window. The test confirms 500×200 → 1000×400 physical
size with the logical viewport still 500×200.

The pointer stays at the same screen coordinates throughout the transition. Its
physical client coordinate (300,60) becomes logical (150,30), over the left control.
The following stationary click and wheel reach only that control. No pointer move,
application input dispatch or fabricated scale event intervenes.

Adjacent results include executable/script hashes and before/after geometry. The
log and screenshot are from the fixed host. The independently archived failing
run is in `../live-dpi-pointer-before`.

```sh
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 \
  cargo build -p lumine-desktop --example live_dpi_pointer --offline
LD_LIBRARY_PATH=/tmp/lumine-ci-loader-check/build/loader \
  python3 scripts/live_dpi_pointer_smoke.py \
  /tmp/lumine-target/debug/examples/live_dpi_pointer \
  --output docs/platform-validation/live-dpi-pointer
```

Plain xrdb updates were not observed by Winit in this Xvfb/Openbox session, so the
script uses the standard XSETTINGS owner/property notification path instead. It
owns that provider and closes it with its display. This does not claim physical
monitor migration, hardware GPU, Wayland or macOS DPI validation. In particular,
Wayland and macOS pointer positions originate in logical coordinates; the host's
cached-coordinate adjustment is deliberately restricted to X11.

The final run includes a pinned `test-runner.py`. A complete Rust source and Cargo
manifest snapshot is archived at `../live-dpi-pointer/source.tar.gz`, with a
per-file source manifest alongside it and the archive hash in each results file.

The separate [maximized run](../live-dpi-maximized/README.md) verifies a window
manager that keeps physical size fixed: the logical viewport changes with DPI,
and stationary pointer input still reaches the correct control.
