# Committed editor text normalization

Single-line editors now strip line breaks consistently from initial values,
external model writes, typing, paste, accessibility replacements and IME commits.
Multiline editors canonicalize CRLF and lone CR to LF. Tabs remain intact;
ordinary text events still reject other control characters atomically. Native
preedit remains unmodified, preserving native cursor-offset semantics.

Normalization borrows already-canonical text and allocates once when conversion
is necessary. Initial normalization creates no undo entry. Canonical-equivalent
model writes preserve selection, preedit and history, and produce no layout or
damage. A real replacement retains the existing undo and IME-cancellation rules.

Canonical model writeback can invoke observers that unmount the editor. Legacy
editor construction and declarative tree construction defer reactive effects
until bindings and listeners have owners. Regression tests reproduce and prevent
both a retained binding after removal and a stale-node panic. Image mounting now
reserves its bitmap node while its initial source binding is deferred.

The native probe checks model/display agreement, retained editor nodes, ordinary
editing after replacement and multiline caret placement. The external-model IME
and original native IME suites run against the same source archive. Linux native
checks use owned Xvfb/Openbox sessions and Mesa llvmpipe. They do not establish
native macOS behavior or hardware GPU performance.

```sh
cargo build -p lumine-desktop --example model_text --locked
python3 scripts/model_text_smoke.py target/debug/examples/model_text \
  --output /tmp/lumine-model-text
```

The final build passes **489 tests** (two opt-in tests ignored), **29 doctests**,
strict workspace Clippy, formatting and the macOS ARM64 cross-check. All three
native suites pass. All 150 source hashes match both archive and workspace.
An initial native-example linker process terminated with SIGBUS; a rebuild with
`CARGO_BUILD_JOBS=1` succeeded. Both build logs are retained.
