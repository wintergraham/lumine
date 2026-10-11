# Variable-height retained component lists

The integrated source-frozen build passes **584 tests**, **30 documentation
examples**, strict Clippy, formatting and the macOS ARM64 all-target cross-check.
Two existing opt-in/native tests remain ignored by the ordinary suite. The
[manifest](source-manifest.json), [archive](source.tar.gz), [metadata](metadata.json)
and [audit](audit.json) identify all 171 captured inputs and reproduce the tested
code. MacOS cross-checking is not native Metal/AppKit execution.

`variable_virtual_list` composes owned keyed row views using GPUI-style fluent
viewport styles. `VariableHeights` supplies indexed allocated heights. Point
changes preserve the first visible index and intra-row offset; explicit changed
scroll requests take precedence. Keyboard paging uses prefix geometry, markers
follow allocated height, and scrollbars follow the updated extent. Natural text
measurement is not inferred. Positional heights must be reordered with data.

The isolated X11 fixture passes **13 native stages** with 100,000 heterogeneous
rows. At most **14 rows** are mounted; only **69 constructors** run across jumps,
streaming growth, keyboard navigation, resize and wheel scrolling. Updating a
visible row and four asynchronous growth chunks retains its mounted components.
Increasing a row above the viewport moves offset 54008 to 54040 while preserving
the visible anchor. Geometry and screenshot pixels are checked against an
independent Python prefix oracle. The existing fixed-height million-row keyboard
fixture also passes after sharing the updated focus marker behavior.

GPU tests compare damage-driven rendering byte-for-byte with full redraw through
height/anchor changes, focus-marker growth and shrinkage, clipping, scrolling,
large jumps and viewport resizing. Native and GPU runs use Mesa llvmpipe with the
private patched Vulkan loader, not a hardware GPU.

## Core index cost

The [release microbenchmark](core-benchmark.jsonl) checks equal-height
fixed/variable scan count and checksum parity, heterogeneous prefixes, 10,000
point updates and 10,000 successive appends. One source-matched run measured:

| Initial rows | 10k point updates | 10k appends | 10k variable range + geometry scans | Same fixed-height scans | Logical index payload |
| --- | ---: | ---: | ---: | ---: | ---: |
| 100,000 | 0.455 ms | 0.271 ms | 4.916 ms | 0.608 ms | 2,000,016 bytes |
| 1,000,000 | 1.484 ms | 0.296 ms | 9.758 ms | 0.516 ms | 20,000,016 bytes |

Point updates/searches are logarithmic; visible geometry is O(log n + visible
rows). Appending builds only new Fenwick entries rather than rebuilding existing
rows. Payload excludes spare vector capacity, allocator overhead and process RSS.
These short shared-host microbenchmarks have no confidence intervals and are not
GUI CPU/RSS rankings. The constant-space fixed-height path remains faster when
all rows have the same height. Historical GPUI/QuickGUI process comparisons are
separate; rejected recent trials remain rejected.

## Reproduction

```sh
cargo test --workspace --all-targets --locked
cargo test --workspace --doc --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo check --workspace --all-targets --target aarch64-apple-darwin --locked
cargo build -p lumine-desktop --example variable_virtual_list --example virtual_keyboard --locked
python3 scripts/variable_virtual_list_smoke.py target/debug/examples/variable_virtual_list --output /tmp/lumine-variable-list
python3 scripts/virtual_keyboard_smoke.py target/debug/examples/virtual_keyboard --output /tmp/lumine-fixed-list
cargo run --release -p lumine --example variable_list_bench --locked
```

The native scripts own their Xvfb/Openbox processes and clean up their test
sessions. See metadata for the exact loader and Cargo environment used here.
