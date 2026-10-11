# lumine patch to cosmic-text 0.19.0

Source: crates.io cosmic-text 0.19.0, upstream commit
`c24886c2471e5606587c46090cd25dbbf209186b`. MIT/Apache licenses are unchanged.
The runtime source is copied locally; unused upstream integration-test and bench
targets are omitted from the vendored manifest.

`src/shape.rs` adds `Shaping::AdvancedUnwrapped`. It uses the existing advanced
shaper but shapes a whole bidi span rather than splitting at potential soft line
breaks. Existing `Advanced` shaping and wrapped layout retain their behavior.
This preserves kerning across a slash when text is unwrapped: Geist's `/s` pair
adjusts the slash advance by -40 font units. Splitting `add/send` at the slash
otherwise shifts the suffix by 0.52 pixels at 13px.

lumine selects this mode for unconstrained text and separates both prepared-cache
entries and streaming-line donors by mode. A small licensed test fixture is a
400-weight subset of source-matching Geist 1.401 from upstream geist-font commit
`af6dae4551cc8aed58a0e7e715d929bee3e3fabd`, retaining `add/send.` glyphs and their
layout tables. Its SIL license is alongside the fixture.

`src/swash.rs` negates the cache key's fractional Y offset for both outline and
pixel-font rendering. Cache coordinates use screen Y down, while Swash renders
outlines with a bottom-left origin (font Y up), and consumers place the bitmap
using `baseline - placement.top`. A positive half-pixel phase previously moved
ink up by half a pixel rather than down, causing a one-pixel disagreement at
fractional text origins. Upstream `LayoutGlyph::physical` normally truncates Y,
which masks this issue; lumine retains final node/DPI phases in its cache keys.
The lumine-gpu text regression checks the alpha-weighted screen centroid across
quarter-pixel phases, including pixel-font rounding.
