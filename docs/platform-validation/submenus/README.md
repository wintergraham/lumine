Native cascading menu validation
================================

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 cargo build -p lumine-desktop --example submenus
LD_LIBRARY_PATH=/tmp/lumine-ci-loader-check/build/loader python3 scripts/submenus_smoke.py /tmp/lumine-target/debug/examples/submenus --output docs/platform-validation/submenus
```

The owned Xvfb/Openbox session opens a parent menu, moves to its submenu trigger,
and opens the child with Right. Resizing the window from 700px to 600px verifies
side placement flips from the trigger's right to its left. Left closes only the
child and restores the trigger's focus; Enter reopens it, Escape closes only the
child, and a pointer click reopens it. Activating the child action must close the
entire menu chain before invoking its callback, then restore root-anchor focus.
With the child reopened, a single click on the parent action must close both
menus and invoke that action exactly once. A final click outside both menus
closes the chain without activating the background button under the pointer.

Read-only reports include both open signals, actual focused semantic labels, and
panel bounds. `submenus.log`, twelve screenshots, and `result.json` retain evidence.
The final run passed all twelve stages with Vulkan Loader 1.4.345 and normal
GPU validation enabled. The right- and left-placed screenshots were inspected:
the trigger spans the menu width, its trailing chevron is visible, and the child
action has a visible keyboard-focus outline. Strict example Clippy also passes.
The script inherits caller loader configuration and cleans up its application,
window manager, and display server.

Dependencies: Xvfb, Openbox, xdotool, xwininfo, and Pillow. This is Linux X11
software-rendered validation, not macOS, hardware GPU, or screen-reader evidence.

The integrated workspace passes 336 tests with one opt-in GPU stress test ignored,
24 doctests, strict Clippy, formatting and macOS ARM64 all-target compilation.
The GPU submenu regression compares partial/full repaint through opening, side
flipping after anchor translation, Left dismissal and root dismissal; flipping
reports zero layout work. Native AccessKit consumers verify logical submenu
ownership, expansion and focus, with safe fallback for invalid/cyclic parent
metadata. Regressions cover disabled triggers, keyed removal, sibling switching,
one-gesture parent action routing, batching and reentrant owner disposal.

Check logs, `metadata.json`, and a Rust source archive preserve this integration.
Default wgpu debug/validation flags were retained; the Khronos Vulkan validation
layer was not installed. This remains software-GPU Linux evidence, not a native
macOS or screen-reader validation result.
