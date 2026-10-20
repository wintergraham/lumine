These 4,608 expected rows are actual source-only Canvas RGBA8 readbacks, recorded
before implementing the native constant-fill correction. Each row is foreground
RGB, authored alpha byte, opaque destination RGB, actual source output RGB.
The corpus covers all 256 alphas, three authored source colors, and six backgrounds.

Chromium: 154.0.8037.57, revision 73c14f6228d7cd537c855007e8f88678969cc0eb.
Skia dependency: 2466dcf3937437e217e7f284afe0e1aae15891ce.
Actual backend: Linux headless CPU/software raster and compositing. The source
Canvas, original CSS constant fills, screenshots and repeated readbacks agreed.
No native output was available to the source observation.

Full immutable JSON, source PNGs/repeats, exact original source CSS, pinned Skia
code and executable/source hashes are retained in Ask's handoff archive under
`artifacts/native-react-compilation/source-flat-paint-linux-r1/`. Its original
artifact-index.json SHA256 is
`c8cc65ea978457b01abee21533bf690da2277cc57000915b0b688b1f361ff35f`.
The TSV header also binds its exact source JSON hash. This corpus does not qualify
GPU source backends, Metal source backends, edges, transparent destinations,
nonuniform backdrops, layers, arbitrary paint, or production source parity.
