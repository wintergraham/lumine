# Native Chromium154 software paint derivation

The native subset uses exact pinned Skia `2466dcf3937437e217e7f284afe0e1aae15891ce`
analytic edge and convex AAA scan-conversion code. Original files, hashes and
BSD license are retained here. `scripts/prepare-source-skia154.py` verifies those
hashes and reproduces `crates/lumine-gpu/src/source_skia154.cpp` from those files
and the reviewed `compat.inc` / `bridge.inc` boundary. No PNG, expected pixels,
browser process, JavaScript, or fixture lookup enters the native implementation.

The measured source call path is SkBitmapDevice::drawRRect ->
skcpu::Draw::drawRRect -> SkPath::RRect -> SkScan::AntiFillPath -> AAAFillPath.
It uses analytic convex scan conversion after conic-to-quadratic subdivision,
1/4-device-pixel Y snapping, 1/16-device-pixel X snapping, and the original RLE
accumulation. Native SDF/fwidth coverage cannot supply those bytes. `bridge.inc`
constructs only monotonic rounded-rectangle conics and lines. It retains Skia's
0.25-device-pixel conic approximation and monotonic subdivision guards.
The pinned Chromium `skia/config/SkUserConfig.h` enables
`SK_SUPPORT_LEGACY_CONIC_CHOP`; the bridge uses that original scalar-float
subdivision branch. The generator verifies this configuration file as well as
the original geometry and scan-conversion sources.

The C ABI accepts finite bounded geometry only and returns at most 65,536
coverage/blit spans. Rust checks every span, rejects overlap/out-of-range output,
and caches only generic geometry operations (128 entries / 8 MiB maximum).
The exact SkARGB32 Color32 versus AntiH2 integer blit dispatch is evaluated in
`source_fill.rs`; coverage is applied after premultiplication. Known opaque
prior destinations are independently proven from actual prepared paint order,
per pixel where the destination changes within a row. Fully resolved runs are
emitted as ordinary opaque GPU quads; no special GPU output tolerance exists.

The matching Blink revision is `73c14f6228d7cd537c855007e8f88678969cc0eb`:
`core/paint/contoured_border_geometry.cc::PixelSnappedContouredBorder` uses
`ToPixelSnappedRect` before generating the source paint contour. The adapter
therefore snaps physical paint endpoints separately from retained layout
geometry. It preserves the independently observed fractional-layout geometry.

The C++ build uses the existing locked `cc` package, C++17 and explicit
`-ffp-contract=off` when the compiler supports that flag. Ask's existing locked
cc1.5.1 and Lumine's existing locked cc1.4.7 satisfy the same declared semver;
no dependency version/profile/feature update is required. Each lock only gains
`cc` under the lumine-gpu dependency list. Compiler and source bytes must be
bound by trusted build evidence before any later native qualification.

CPU-only replay of the actual C++ ABI and Rust blit implementation matched every
original source pixel for all ten supported single-primitive opaque fixture
cases, including opaque/near-opaque endpoints, square and fractional rectangles
and all rounded rims. Independently replaying the upstream source algorithm
also matched the original transparent badge cases; transparent native
composition is not implemented by this opaque-destination adapter.

The bounded clipping ABI uses the pinned `SkEdgeClipper::clipMonoQuad` and
`SkGeometry` quadratic roots/chop functions before analytic edge conversion,
then the original AA clip coverage merge. It accepts one axis-aligned,
uniform-radius ancestor with complete integer device-pixel bounds. Only after
every actual ancestor has been evaluated may the draw suppress its original
native geometry clip, avoiding duplicate native SDF coverage. Multiple clips,
transforms, fades, unequal corners and subpixel clip bounds fail closed.
Actual source-fill adapter CPU replay against the original clipped fixture
matches every pixel; a duplicate-ancestor case preserves the original child
quad and does not authorize clip suppression. The reusable C++ ABI/Rust replay
now covers eleven original cases with zero differing pixels. Actual GPU
verification remains required.

Additional source-only diagnostic captures exposed four low-coverage corner
pixels for an opaque physical-radius-16 shape. Investigation found that the
subset omitted the original `RunBasedAdditiveBlitter::flush` call to
`snapAlpha`. The generator now copies that exact function from the verified
original source, and dense accumulated rows apply it before the real blit.
Direct real-blitter paths retain their original dispatch. The prior failing
diagnostic is retained independently; all six supported opaque diagnostic
cases now match every source pixel in CPU replay. This is an upstream raster
operation, with no pixel exception or fitted correction. CPU replay still
does not establish GPU parity or general source qualification.

The opaque-backdrop code is unqualified until actual GPU checks pass. Transparent
destinations, isolated opacity/layers, unsupported ancestor clips, small
mask-backed source shapes (width<=32 and mask<=1024 bytes), unequal corner
radii, borders/filters/transforms and unknown destinations retain the existing
path. Those source differences remain strict failures. No production host v1
capability, actual target descriptor, qualification pin, or global enablement
is added. The full original 20-case source comparison remains required.

Existing renderer test expectations change deliberately: source diagnostic
rims are compared with original Chromium source samples, and a known partial
opaque preceding draw may be resolved per pixel. Unknown translucency remains
unsupported. Default rendering is independently compared byte-for-byte with
frozen actual native R2 pixels; those native bytes never supply source expected
paint or an allowance. No full GPU/Cargo build was run in the isolated worker.

## Separate known premultiplied target candidate

`SourcePaintPolicy::SkiaSoftwareSrgb8KnownPremultipliedTarget` is a distinct
native-host selection. It does not expand the opaque-backdrop policy and has
no production Host grant or qualification. It uses the same independently
derived source spans and clip preparation, retaining all four N32 target
channels through the original Color32 / AntiH2 integer operations. Original
`SkBlitter_ARGB32.cpp`, `SkColorPriv.h` and `SkColorData.h` are retained with
their pinned hashes and license to document that derivation.

Every previous destination pixel must come from an actual earlier source
resolved run or an independently proven opaque native core, in actual draw
order. Any overlapping unknown native translucency, layer, image, transform,
font or effect blocks the transaction. Background alpha initially supports
only zero or 255. Unknown alpha is never reconstructed from scene metadata or
expected PNGs. Arbitrary isolated opacity groups remain unsupported; no partial
descendant group is flattened.

Only after successful full resolution does a private Quad marker select the
existing basic pipeline with blending disabled. The ordinary shader retains
alpha and multiplies the emitted straight ratios back into the known source
premultiplied bytes. UI nodes and styles cannot request this marker. The
candidate requires shading splitting and evaluates every active clip before
suppressing geometry; unsupported state preserves the original paint.

`scripts/check-source-premultiplied-adapter.py` records a reproducible CPU-only
replay of the actual paint helpers and native C++ ABI, with explicit genuine
Scene/layout dependency hashes and compiler/source identities. All twenty
original full source frames match every pixel after the closed-leaf alpha
folding below. Negative transactions preserve original quads and destination
state for unknown native translucency, layer, image, transform, a partial
opacity closure including a hidden descendant, and inherited group opacity.
This CPU harness mirrors only the Draw/Quad fields consumed by the
helpers; it does not claim a GPU library ABI or observed native GPU behavior.
The separate actual GPU manual-scope test must still compare all twenty
original frames with zero pixel or geometry differences and preserve all
frozen native default controls. No CPU result grants source acceptance.

The last fixture sets `Effects.opacity` on a leaf; it does not explicitly set
the native isolated cache flag. Pinned Chromium
`PaintOpBuffer::PlaybackFoldingIterator` folds a complete
SaveLayerAlpha/single-supported-drawing-op/Restore sequence into paint alpha.
`ScopedRasterFlags` performs the alpha multiplication; `SkARGB32_Blitter`
quantizes SkColor before premultiplication. Those original Chromium files,
paint flag predicates and Chromium license are retained with exact hashes.

The candidate applies that rule only to a complete own-opacity leaf: actual
`Scene.children` must be empty, `Scene.layer_items(Some(id))` must enumerate
exactly the same one primitive and source owner, and every opacity/effect or
isolated ancestor is rejected. Clips, masks, transforms, borders, decorations,
filters and shadows are rejected. A failed transaction restores the original
quads without changing known destination state. Both cached and direct native
leaf representations use the same source rule; arbitrary groups stay native.

The separate nine-case independent opacity source cohort demonstrates why
this restriction is necessary: a half-opacity single draw has center
`[7,7,9,10]`, whereas the first disjoint two-draw group's center is
`[6,7,9,10]`; nested groups differ again. The candidate matches both supported
leaf source frames exactly and explicitly rejects all seven group cases.
The complete twenty-case GPU test remains pending, with no production grant.
