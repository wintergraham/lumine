# IME cancellation after external model updates

Replacing an editor's model during preedit cleared the editor's composition but
left the desktop host suppressing ordinary keys. The same stale native session
could also commit its old candidate into the replacement text.

`TextEditor::composition_cancel_revision()` now records explicit local
cancellation of an existing preedit: selection/navigation, local edits, changed
model text, successful undo/redo and explicit cancellation. Native empty preedit
and native commit do not advance it. Equal model writes and unsuccessful history
operations keep their prior behavior.

The host observes this revision before native events, before the visibility gate,
after layout preparation and after accessibility actions. Cancellation in the
current session releases keyboard suppression and resets native IME through the
existing ordered Disabled/Enabled guard. Focus changes establish a new baseline.
Ordinary empty-preedit→Commit and a committed value echoed through the model do
not reset the session.

The native probe creates an owned X11/XIM IBus/libpinyin session. A timer observes
real preedit and replaces the model without changing focus or input engine. It
checks that the old candidate cannot commit, ordinary editing resumes, and fresh
Unicode composition works. The original IME and live AT-SPI suites are rerun on
the same archived build.

```sh
cargo build -p lumine-desktop --example external_ime --locked
python3 scripts/external_ime_smoke.py target/debug/examples/external_ime \
  --output /tmp/lumine-external-ime
```

This revision identifies local cancellation of existing preedit, not every model
change or a native protocol epoch. Pinned winit's Wayland reset notifications are
local and its text-input completion does not expose serials to reject arbitrary
late server commits after reset. A model change after native empty preedit but
before its commit is outside this cancellation-revision guarantee. Native macOS
and Wayland IME validation remain outstanding.

Sources, scripts, hashes, exact commands, native snapshots and validation logs
are archived alongside this record. Native rendering uses Mesa llvmpipe.

The final build passes **482 tests** (two opt-in tests ignored),
**29 doctests**, strict workspace Clippy, formatting and the macOS ARM64
cross-check. All three native smoke suites pass. All 147 source hashes
match both the archive and workspace after execution.
