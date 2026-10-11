Native component menu validation
================================

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 cargo build -p lumine-desktop --example menus
python3 scripts/menus_smoke.py /tmp/lumine-target/debug/examples/menus --output docs/platform-validation/menus
```

The example uses component children, fluent styles, a signal-controlled menu,
and leaf menu items. The owned Xvfb/Openbox smoke opens the menu, clicks a disabled
item, navigates enabled items with ArrowDown/Home/End, activates with Enter and
pointer, uses printable `m` typeahead to focus Middle, dismisses with Escape,
and verifies Tab closes the menu and advances focus to the next outside control. Read-only reports query actual focused
semantic labels. Callback logs verify that open is already false when selection
handlers run. The disabled item must never activate.

The recorded run used the isolated Vulkan Loader 1.4.345 at
`/tmp/lumine-ci-loader-check/build/loader`, with default wgpu debug/validation flags preserved. The Khronos Vulkan
validation layer was not installed.
Stage 3 was visually inspected: Middle is focused and Unavailable is muted.

Ten screenshots and `menus.log` retain the native interaction sequence;
`result.json` records asserted behavior. The script inherits caller environment
and cleans up its owned application, window manager, and display server.

This is Linux X11 software-rendered validation, not macOS, hardware GPU, or
screen-reader validation. Dependencies include Xvfb, Openbox, xdotool, xwininfo,
and Pillow.

The final integrated build passed 322 tests with one opt-in GPU stress test ignored,
24 doctests, strict Clippy, formatting, and macOS ARM64 all-target compilation.
Two GPU regressions compare incremental and full repaint through menu focus,
dismissal, and long-menu scrolling; focused rows stay inside padded clipping and
navigation/scroll translation reports zero layout work. Core tests cover dynamic
menu shrink, reactive gap changes, keyed ownership, Tab exit and Unicode typeahead.
Native AccessKit consumers check roles/actions, trigger expansion, disabled states,
focus restoration, actions opening dialogs and owner removal during activation.

Check logs, source archive, binary/loader hashes and `metadata.json` retain the
integration evidence. The final native run repeated all ten stages after the
repeated-letter typeahead correction. Submenu-trigger behavior and side placement
remain follow-up implementation work, not part of these passing checks.
