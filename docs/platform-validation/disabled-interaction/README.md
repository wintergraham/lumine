# Disabled interaction lifecycle

Disabling a view now publishes its input-disabled state and detaches owned
capture, focus and pending pointer/keyboard activation before running cleanup
callbacks. Captured descendants still receive PointerCancel, and formerly focused
descendants receive Blur. Cleanup cannot refocus a still-disabled descendant.
An uncaptured, nonfocusable press is also discarded, so a later release after
reenabling cannot activate it.

Callbacks may remove or reenable their owner. If a cancellation callback
reenables and refocuses the same control, the deferred old Blur is skipped so
the new focus state remains intact. Notifications read surviving
metadata after callbacks; Ui checks node lifetime again and uses the current
flag before updating semantics/effects. This fixes a reproduced stale-node panic
when a Blur or PointerCancel callback removes the control being disabled.
The low-level `InputDispatcher::set_options` remains metadata-only; use
`Ui::set_disabled` or component `disabled_when` for scene-aware cleanup.

The native probe also exposed composed slider presses suppressing default focus
without requesting focus themselves. Sliders now explicitly focus on a primary
press, allowing immediate keyboard interaction.

Core tests cover reentrant removal/reenabling, disabled refocus rejection, pending
press cancellation, style cleanup and fresh interactions after reenabling.
The native composed example disables an ancestor while Space, a primary button
or a slider drag is held. Focus/capture must clear, late release must not activate,
and new interactions after reenabling must work.

```sh
cargo build -p lumine-desktop --example disabled_interaction --locked
python3 scripts/disabled_interaction_smoke.py target/debug/examples/disabled_interaction \
  --output /tmp/lumine-disabled-interaction
```

Source and script hashes, exact commands, native logs/screenshots and integrated
checks accompany this record. X11 uses owned Xvfb/Openbox with Mesa llvmpipe and
isolated settings. Native macOS/Wayland behavior is not established by this run.

The final build passes **476 tests** (two opt-in tests ignored),
**29 doctests**, strict workspace Clippy, formatting and the macOS ARM64
cross-check. Native disabled-interaction checks pass. All 143 source hashes
match the archive and workspace after execution.
