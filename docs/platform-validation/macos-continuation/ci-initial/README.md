# Initial GitHub Actions inspection

Run: https://github.com/zeronsh/lumine/actions/runs/35795413672
Commit: `e51589408079f831c8b9ee433388e2edf46432a7`

`run-35795413672.log` is the unfiltered `gh run view --log` output. `run.json` records job conclusions and steps; `artifacts.json` confirms that no artifacts survived this run.

- macOS core tests fail at `editor_paging.rs:73`: the test sends Control+Z, while the editor correctly requires Command+Z on macOS. Paging assertions before the undo check pass. The fix uses the target platform command modifier, retaining undo/redo assertions; related read-only and streaming tests now also use realistic single-platform command modifiers.
- Linux desktop fixture linking fails inside rust-lld with SIGBUS for `line_height` and `animated_image`. The log does not establish an exact resource cause. CI now bounds concurrent Cargo jobs and omits dev/test debug symbols to reduce fixture linking memory and disk use; actual Linux rerun is required to validate this mitigation.
- macOS Metal subprocess fails, but its output was written exclusively to a retained file. Both GPU evidence upload steps fail because the account artifact storage quota was hit. No underlying Metal failure diagnosis can be established from this run. CI now prints retained macOS text logs to the job log even after failures, and sets a three-day artifact retention period. Existing account quota is not repaired by this change.
- The Ubuntu core job was canceled by the matrix fail-fast after macOS failed. The matrix now uses `fail-fast: false` to preserve independent platform results.

These failures do not prove parity or completion of native validation gates.

Local macOS verification: `cargo test -p lumine --test editor_paging --test editor_read_only --test editor_streaming_history --locked` passed (13 tests), with raw output in `focused-shortcut-tests.log`. This verifies command routing/model assertions, not native key injection or UI gates.
