# Read-only editors

`text_input` and `text_area` support `.read_only(true)` and reactive
`.read_only_when(move || locked.get())`. Component refinements follow normal
outer-property precedence. These options apply to editor roots, not containers.
The low-level `EditorHandle` provides `set_read_only` and `is_read_only`.

Read-only editors remain focusable and selectable. Navigation, scrolling, copy
and external model writes remain available. User text, IME updates, paste,
deletion, undo/redo and accessibility value replacement cannot edit the text.
Cut copies without deleting. Direct `TextEditor` access remains an application
mutation API. Switching policy preserves selection/history and cancels existing
preedit; an equal policy write leaves layout, damage and semantics unchanged.

Native IME permission follows editability separately from logical focus. The
host updates permission before native events and hidden-window draw gates, and
releases keyboard suppression. An active composition crossing a target reset
must complete Disabled/Enabled before new composition is accepted; rapid
read-only/ editable transitions preserve that guard. Without active marked text,
macOS may emit no Disabled event, so an ordinary toggle does not arm the guard.
This remains a context-notification guard, not a Wayland server protocol epoch.

AccessKit exposes read-only state, removes SetValue and retains focus and text
selection actions. Core tests exercise both editor forms, wrapper refinements,
reactive ownership, cancelled edits and cleanup. The native clipboard/input
probe checks selectable text, copy-only cut, blocked paste/undo, reenabled input
and external model updates. A real IBus probe toggles read-only during preedit
without a simultaneous model change or focus/engine switch, then checks stale
candidate rejection, ordinary keys and fresh Unicode composition. The existing
external-model and ordinary native IME suites are rerun as regressions.

Sources, exact commands, native logs and source hashes are archived here.
Linux native checks use owned Xvfb/Openbox sessions and Mesa llvmpipe; native
macOS and hardware GPU behavior remain unverified.

The final build passes **497 workspace tests** (two opt-in tests ignored),
**29 doctests**, strict Clippy, formatting and the macOS ARM64 cross-check.
All four native checks pass. All 153 source hashes match both archive and
workspace after execution. Build concurrency is limited to one job in this run.

```sh
cargo build -p lumine-desktop --example read_only --example external_ime --locked
python3 scripts/read_only_smoke.py target/debug/examples/read_only \
  --output /tmp/lumine-read-only
python3 scripts/external_ime_smoke.py target/debug/examples/external_ime \
  --read-only-test --output /tmp/lumine-readonly-ime
```
