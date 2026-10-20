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

The code is an unqualified candidate until actual GPU checks pass. Transparent
destinations, isolated opacity/layers, partial rounded ancestor clips, small
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
