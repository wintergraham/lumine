# Live Linux accessibility validation

The `atspi` example uses ordinary retained components and fluent styles. The
runner creates a private D-Bus session, accessibility bus, Xvfb and Openbox, then
uses the system GI AT-SPI client to inspect and operate the running application.
It does not substitute internal semantics or application logs for bus assertions.

The checks cover control names and roles, focus actions/events, editor Text reads,
caret/selection actions, native typing reflected in accessible text, button
activation and readable reactive status, slider values, checkbox state, and
input disable/reenable. It also disables accessibility, changes the application
through native input, reenables the bridge, and checks fresh values and actions.

Editor selection actions map AccessKit grapheme positions to committed UTF-8
byte offsets and use ordinary event routing, including modal, disabled and
cancellation guards. Unchanged text reuses cached TextRun metadata; selection-only
updates change the parent node. Static labels expose their text as AccessKit
values and TextRun children, without editor selection actions.

Run from the repository root:

```sh
cargo build -p lumine-desktop --example atspi --locked
/usr/bin/python3 scripts/atspi_smoke.py target/debug/examples/atspi \
  --output /tmp/lumine-atspi
```

Dependencies include Xvfb, Openbox, xdotool, ImageMagick, D-Bus, `at-spi2-core`,
`python3-gi` and `gir1.2-atspi-2.0`. The script requires a Python interpreter with
those system GI bindings. It uses memory-only GSettings and temporary config/cache
directories, and shuts down only its owned processes. An early development run
without this isolation persisted desktop accessibility preferences through dconf;
that runner is not the archived version.

This is live bus-client validation, not an Orca usability assessment or native
macOS accessibility validation. The pinned AccessKit Unix bridge does not expose
EditableText; text replacement in this test uses native keyboard input. TextRun
metadata describes hard lines, not soft-wrapped visual rows, and does not provide
per-character screen geometry. AccessKit character byte lengths are limited to
255: documents containing a single larger grapheme retain their value but omit
TextRun projection rather than publishing truncated text. Accessibility toggling
does not prove recovery after an accessibility-bus process crash.

`source.tar.gz` and `source-manifest.json` preserve the build inputs;
`metadata.json` records source and binary hashes and exact commands. The native
results, initial/final trees, events, screenshot and logs are stored alongside
the full workspace validation logs.

The recorded integrated run passes **434 tests** (two opt-in tests ignored),
**28 doctests**, strict all-target workspace Clippy, formatting and the macOS
ARM64 cross-check. The final native tree contains `Grace`, `Count: 3`, a checked
checkbox and slider value `65`. All archived source hashes were independently
checked against both the workspace and archive after the run.
