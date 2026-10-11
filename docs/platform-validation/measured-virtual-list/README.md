# Naturally measured retained virtual rows

The combined build passes **598 tests**, **31 documentation examples**, strict
Clippy, formatting and the macOS ARM64 all-target cross-check. Two existing
opt-in/native tests remain ignored by the ordinary suite. The [source archive](source.tar.gz)
contains all 223 inputs in the [manifest](source-manifest.json), including
native smoke helpers. [Metadata](metadata.json) records commands, binary hashes,
loader/Cargo configuration and source immutability; [audit](audit.json) verifies
archive contents and test counts. This is not native macOS or hardware GPU evidence.

`measured_virtual_list` composes ordinary child views at the viewport width and
writes natural allocated heights into a positional `VariableHeights` cache.
It shares ownership, anchoring, scrollbars and keyboard navigation with explicit
variable-height lists. Mounted text wrapping, child insertion, padding, margins
and width changes update layout without reconstructing retained rows. Offscreen
heights remain estimates until revisited; exact unseen extent is not promised.

Geometric discovery handles severe overestimates within the bounded layout
feedback budget. A one-pixel minimum applies to the shared cache, including later
writes and growth, preventing tiny estimates or empty rows from materializing the
entire collection. Applying this floor scans the cache once. Independently sized
lists need separate caches. Genuine measurement/model cycles return the existing
layout feedback error and can recover after the dependency is repaired.

Thirteen integration tests cover both inaccurate-estimate extremes, natural
padding/margins, wrapped reflow, offscreen remount, retained keys, measurement
subscription cleanup, constructor rollback, reentrant callback writes and
feedback errors. They found and fixed keyboard destinations being displaced by
newly measured tall rows. A pending reveal now follows its target until settlement,
while yielding to newer pixel requests, programmatic focus and focus redirection.

The GPU test uses the renderer's actual font shaper. Streaming wrapped text,
growth above the anchor, viewport narrowing, scrolling and large jumps produce
byte-identical damage/full-redraw output; mounted rows meet without gaps or overlap.
This supplements the explicit-height focus-marker and geometry pixel tests.

The isolated X11 measured fixture passes **seven stages** with 100,000 rows:
initial layout, jump, four asynchronous child additions, resize, wheel, Home and
End. At most **11 rows** remain mounted in the captured stages; **70 row constructors**
run across the entire scenario, including temporary discovery. Row 1000 grows
naturally from **76 to 164 pixels**, its anchor stays fixed, and its streaming
stage constructs no replacement rows. An independent Python child-geometry oracle
checks wrapper/cache heights, row adjacency and solid screenshot pixels.

The explicit-height native fixture also passes all **13 stages**, and the
fixed-height million-row keyboard fixture passes again. Native rendering and GPU
readbacks use Mesa llvmpipe with the private patched Vulkan loader. Their correctness
checks are separate from process CPU/RSS benchmarking.

## Reproduction

```sh
cargo test --workspace --all-targets --locked
cargo test --workspace --doc --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo check --workspace --all-targets --target aarch64-apple-darwin --locked
cargo build -p lumine-desktop --example measured_virtual_list --example variable_virtual_list --example virtual_keyboard --locked
python3 scripts/measured_virtual_list_smoke.py target/debug/examples/measured_virtual_list --output /tmp/lumine-measured-list
python3 scripts/variable_virtual_list_smoke.py target/debug/examples/variable_virtual_list --output /tmp/lumine-variable-list
python3 scripts/virtual_keyboard_smoke.py target/debug/examples/virtual_keyboard --output /tmp/lumine-fixed-list
```

The scripts create and clean up their own Xvfb/Openbox sessions and do not connect
to the user's display, D-Bus or input-method session. See metadata for the exact
validation environment and the composition guide for cache/estimate semantics.
