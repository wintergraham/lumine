# Bounded text editing history

Lowering history limits with both undo and redo populated could leave a later
transfer over the configured destination limit. Undo/redo now trim the receiving
stack after each transfer. Eviction removes the oldest entry in that stack while
preserving the remaining delta chain.

The existing policy remains per stack: each independently allows at most the
configured entry count and inserted/deleted UTF-8 payload byte budget. Defaults
are 100 entries and 4 MiB per stack. Payload accounting excludes entry, container
and allocator metadata; it is not an RSS bound. Lowering either limit trims and
calls `shrink_to_fit` to release spare stack capacity.

Cached payload totals replace the prior whole-history byte scan on every edit.
Accounting updates take constant work plus the entries actually evicted; clearing
a branched redo history still drops those entries. No process CPU/RSS improvement
is claimed without a dedicated measurement.

Core regressions establish reduced-limit enforcement, byte-counter consistency,
redo branch clearing, oversized edit eviction and capacity reduction. The native
composed editor uses actual typing and shortcuts: type `abc`, undo to `ab`, reduce
limits to one entry, redo to `abc`, undo to `ab`, then verify a second undo is inert.

```sh
cargo build -p lumine-desktop --example history_limits --locked
python3 scripts/history_limits_smoke.py target/debug/examples/history_limits \
  --output /tmp/lumine-history-limits
```

Build inputs and runner are archived with hashes and commands. Native logs,
snapshots and integrated check logs accompany the record. X11 runs on owned
Xvfb/Openbox with isolated settings and Mesa llvmpipe. Native macOS behavior
remains unvalidated; this work does not resolve external model/IME cancellation.

The final build passes **478 tests** (two opt-in tests ignored),
**29 doctests**, strict workspace Clippy, formatting and the macOS ARM64
cross-check. The native history-limit smoke passes. All 144 source hashes
match both the archive and workspace after execution.
