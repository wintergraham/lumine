# Retained image fitting

Image components now support fluent `object_fit` with `Fill`, `Contain`, `Cover`,
`None` and `ScaleDown`, plus `object_contain()` and `object_cover()` shortcuts.
Placement is centered within the allocated content box, excluding padding.
Oversized pixels are clipped to that content box. Outer decoration and intrinsic
sizing remain independent of fitting; clipping is rectangular.

A retained viewport and bitmap implement the modes using existing layout,
translation and clipping. Mode changes do not replace the component or image
source. Reactive style patches and interaction variants resolve the local fit
property through the existing style precedence, without inheritance.

The native example uses ordinary component children and style methods. Its
runner operates an owned Xvfb/Openbox window, clicks mode buttons, resizes the
window and checks actual rendered colors and placement for a nonsquare source.
The test records screenshots and results beside this document.

```sh
cargo build -p lumine-desktop --example image_fit --locked
python3 scripts/image_fit_smoke.py target/debug/examples/image_fit \
  --output /tmp/lumine-image-fit
```

GPU pixel and incremental/full-redraw comparisons exercise fit transitions,
clipping and retained image uploads. `GpuStats::image_uploads` counts uploads
performed by each render call, allowing fitting-only updates to be checked
independently of texture residency. Native rendering here uses Mesa llvmpipe;
this is not hardware GPU performance or native macOS runtime evidence.

`source.tar.gz` and `source-manifest.json` preserve build inputs. `metadata.json`
records exact commands and hashes; integrated check logs accompany the native
screenshots and results.

The recorded run passes **439 workspace tests** (two opt-in tests ignored),
**29 doctests**, strict workspace Clippy, formatting and the macOS ARM64
cross-check. Eight native stages pass **6,608 pixel checks**. All 134 archived
source hashes were checked against both the archive and current files. The
existing image smoke also passes, preserving default stretch, intrinsic resize,
same-size source replacement, padding, semantics and timed close behavior; its
logs/screenshots and runner are retained under `existing-images/`.
