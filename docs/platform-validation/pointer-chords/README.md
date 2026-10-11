# Pointer button chords during dragging

Pointer capture now records the button that initiated it. Releasing a different
button preserves capture, and primary pressed/activation state survives unrelated
releases. Matching release, pointer cancellation, blur, removal and explicit
release retain their cleanup behavior. Capture requested outside a down event
keeps its existing button or defaults to primary when there is none.

Editors and both slider APIs now end primary dragging only on primary release
or cancellation. Existing disabled and focus cleanup is retained. Scrollbars
already checked primary releases in their local handler; the dispatcher fix now
keeps their capture intact as well.

Core tests cover primary and custom secondary capture, outside movement, primary
activation, prevented release, cancellation, editor selection, both slider APIs,
and scrollbars. The native example uses ordinary composed editor/slider controls
and periodic model snapshots. Its runner sends real primary down, secondary
down/up, further motion, primary up, and further motion on an owned Xvfb/Openbox
desktop. Selection and slider values must advance after the unrelated release
and remain unchanged after primary release.

```sh
cargo build -p lumine-desktop --example pointer_chords --locked
python3 scripts/pointer_chords_smoke.py target/debug/examples/pointer_chords \
  --output /tmp/lumine-pointer-chords
```

The runner isolates settings and owns its display/window-manager processes.
Source inputs and runner are archived with manifest and binary hashes; native
logs, snapshots and screenshot accompany integrated check logs. This is Linux
X11 input validation using Mesa llvmpipe, not macOS or Wayland native evidence.

The final build passes **465 tests** (two opt-in tests ignored),
**29 doctests**, strict workspace Clippy, formatting and the macOS ARM64
cross-check. The native chord smoke passes. All 139 archived source hashes
match the archive and workspace after execution.
