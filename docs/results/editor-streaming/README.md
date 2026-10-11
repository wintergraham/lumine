# External editor value streaming microbenchmark

The changed `TextEditor::set_text` retains small edit deltas instead of whole-document replacements. In this headless workload, the same 100-entry / 4 MiB history limits retain **100 undo steps instead of 7**. Every undo and redo verifies the entire expected committed text; all ten process runs pass.

This measures the public owned-string `set_text` path, including cloning the incoming model. It does not measure the newer borrowed widget binding, text shaping, layout, rendering, native event delivery, or competing frameworks. Prefix comparison remains linear in document length; this is not a constant-time streaming claim.

| Metric | Archived implementation | Current implementation |
|---|---:|---:|
| 1,000 updates, median milliseconds (range) | 42.647 (41.259–46.783) | 10.204 (9.821–10.985) |
| Live requested Rust allocation bytes before updates | 524,836 | 524,836 |
| Live requested Rust allocation bytes after updates | 4,775,648 | 1,060,788 |
| Peak requested Rust allocation bytes during updates | 5,574,020 | 1,326,936 |
| Resident memory after updates, median KiB (range) | 9,132 (9,128–9,168) | 3,396 (3,364–3,420) |
| Verified undo steps / redo steps | 7 / 7 | 100 / 100 |

Allocation counts and undo counts are identical across all five runs of each implementation. The allocator wrapper counts live requested allocation sizes, including string capacities, both document/model buffers, history storage, and other Rust allocations. These are not isolated history-payload measurements or allocator/RSS upper bounds. RSS includes allocator overhead and process mappings; timings and RSS are observations from this shared Linux runner, without CPU pinning or a statistical significance claim.

Each fresh process starts with a 256 KiB ASCII document and appends 1,000 four-byte UTF-8 tokens (` 好`). It supplies each complete updated value through `set_text`, then validates every available undo and redo after the timed region. The before/after order alternates over five repeats. There is no GUI, GPU, display server, or warm-up phase.

The before source is the exact `text_edit.rs` from [the initialization archive](../../platform-validation/initialization/source.tar.gz); the after source is the working implementation captured for this run. Each is compiled as the sole module of an otherwise identical temporary Rust crate with the same exact `unicode-segmentation` version, thin LTO, and one codegen unit. Both use the identical benchmark and counting allocator. This isolates the text editor module rather than building an old or new full GUI application.

[Raw trials and summaries](results.json), [metadata](metadata.json), [benchmark](benchmark.rs), [captured runner](run.py), [before source](before-text_edit.rs), [after source](after-text_edit.rs), and the two build logs/lockfiles are retained here. Metadata records timestamps, actual command, build environment, Rust/Cargo versions, platform, source/archive/runner hashes, and the temporary binary hashes. The runner rejects changes to either current source, benchmark, runner, or baseline archive before publishing results.

To run a new comparison from the repository root:

```sh
python3 scripts/editor_streaming_compare.py \
  --archive docs/platform-validation/initialization/source.tar.gz \
  --output /tmp/lumine-editor-streaming --repeats 5
```

The `run.py` copy is a provenance snapshot of the repository script, whose default source root is derived from its original `scripts/` location. Use the repository script for a fresh working-tree comparison. A future source change produces a new measurement; it does not change the archived result above.
