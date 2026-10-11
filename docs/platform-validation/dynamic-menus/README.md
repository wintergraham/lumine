# Dynamic application menu validation

Two actual native windows in owned Linux sessions. X11 and Wayland pass enabled/disabled shortcuts, checked state callback, replacement unregistering old commands, and updated command delivery to both existing windows. Windows are arranged side by side so both can present; this is a menu validation, not an occlusion claim.

Seven unit tests cover rendered keyboard traversal, checked semantic state projecting to AccessKit MenuItemCheckBox, focused typed-action override, binding disposal/disabled suppression, unique accelerators, state updates preserving shortcuts, and reactive model replacement. Native-only strict Clippy passes. macOS cross-check passes; AppKit runtime remains unverified here.

`covered-failure` retains the earlier overlapping-window stall. A separate minimal covered-window fixture investigates that lifecycle issue; it is not counted as a passing menu test. Initial focus-race automation attempts remain in /tmp/lumine-dynamic-menu-wayland*. Exact binary hash is retained. Owned source snapshots follow checks, while the wider workspace was concurrently developing; this is not a hermetic full-workspace provenance claim.

Reproduce with `scripts/native_menu_smoke.py /tmp/lumine-target/debug/examples/dynamic_menus --output <dir>` and add `--wayland` for native Wayland. Use `LD_LIBRARY_PATH=/tmp/lumine-ci-loader-check/build/loader` on this builder.
