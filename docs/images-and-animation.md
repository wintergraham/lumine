# Image loading and animation

`ImageCache::load(key, fetch)` shares one pending application-supplied future among
requests for that key. Successful decoded images enter a bounded LRU cache;
failures can be retried. Dropping the last pending request drops its transport
future. The default pixel budget is 16 MiB, configurable up to 64 MiB; at most
256 keys are retained and keys are limited to 1024 UTF-8 bytes. Clearing a cache
cannot allow an older in-flight generation to replace a newer request.

`async_image(label, cache, key, fetch, loading_view, error_view)` wraps this in an
owned component with loading, success and error children. Use a shared cache
through a provider when multiple components load the same resources. Transport
and credentials belong to the application; the framework does not start hidden
network requests. Image decoding is available through `lumine_gpu::assets`.

`decode_gif` composites GIF offsets and Keep/Background/Previous disposal into
bounded immutable full frames. Background disposal clears to transparent for
GUI compositing. An absent repeat extension plays once; finite GIF repetitions
add to the first play. Zero-delay frames use 100 ms. Decoded pixels are limited
to 64 MiB and 4096 frames. Application-provided `Animation` data uses frame
intervals from 10 ms to one day and equal-sized frames.

`animated_image` and `animated_image_controlled` are ordinary styled image views.
They use a monotonic timeline, skip missed frames after a delayed wake, and retain
the final frame when finite playback ends. Each active player has one cancelable
sleep. Fully clipped/offscreen/zero-opacity allocations pause; host visibility,
minimization, occlusion and suspension also pause through `Ui::set_presented`.
Uncovered visibility resumes from the paused position. Overlap by another
ordinary sibling is not an occlusion test. Removal cancels the owned task.

Frames remain decoded, but GPU textures follow the renderer's live-image policy:
it need not retain every animation frame on the GPU. The
[animation example](../crates/lumine-desktop/examples/animated_image.rs) accepts a
GIF path and includes a pause/resume control. A `TaskRunner` provider is required
for asynchronous loading/playback; the desktop host supplies one.

For code-driven animation (spinners, transitions, springs), await the window's display-paced frame clock instead of a timer; see [animation frames](application.md#animation-frames).

## Retained procedural images and typed uniforms

Keep one `ShaderInstance` per independently animated surface. Its texture,
uniform buffer and bind groups survive parameter changes; unchanged dimensions,
uniforms and dispatch return the previous immutable image. Resize, shader changes
and uniform-layout changes reallocate resources. Simultaneously displayed old
and current snapshots retain separate outputs. The software fallback is lazy;
GPU rendering does not call it.

Implement `ShaderUniforms` to make the shader's f32 storage layout explicit.
Read motion signals inside `image_signal`, then pass the typed parameters to
`render`. Motion keeps its existing display cadence and ownership; constructing
an image snapshot does not create another animation task.

```rust,no_run
use std::{sync::Arc, time::Duration};
use lumine::{compose::prelude::*, image::{ShaderInstance, ShaderUniforms}};

#[derive(Clone, Copy)]
struct Tint { progress: f32, rgb: [f32; 3] }
impl ShaderUniforms for Tint {
    fn encode(&self) -> Vec<f32> {
        vec![self.progress, self.rgb[0], self.rgb[1], self.rgb[2]]
    }
}
const TINT: &str = r#"
@group(0) @binding(0) var<storage, read> p: array<f32>;
@group(0) @binding(1) var output: texture_storage_2d<rgba8unorm, write>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) i: vec3<u32>) {
    if any(i.xy >= textureDimensions(output)) { return; }
    let rgb = mix(vec3(0.12), vec3(p[1], p[2], p[3]), p[0]);
    textureStore(output, vec2<i32>(i.xy), vec4(rgb, 1.));
}
"#;

let view = component(|cx| {
    let tint = cx.motion_value(0.0);
    tint.animate_to(1.0, Transition::tween(Duration::from_millis(250), Easing::EaseOut));
    let progress = tint.signal();
    let mut shader = ShaderInstance::new(TINT);
    image_signal("Tint", move || {
        let params = Tint { progress: progress.get().clamp(0., 1.), rgb: [0.8, 0.3, 0.1] };
        shader.render(32, 32, &params, [4, 4], move || {
            let mut rgba = [0u8; 4];
            for c in 0..3 {
                rgba[c] = ((0.12 + params.progress * (params.rgb[c] - 0.12)) * 255.).round() as u8;
            }
            rgba[3] = 255;
            Arc::<[u8]>::from(rgba.repeat(32 * 32))
        }).expect("valid shader parameters")
    }).size(32., 32.)
});
```

Compute shaders write **premultiplied RGBA8**; software fallbacks return
**straight-alpha RGBA8**. Binding 0 is the read-only f32 storage buffer and
binding 1 is the `rgba8unorm` output. Dispatch dimensions are workgroup counts,
not pixel counts. The shader must bound-check its invocations.

## Ordered content effects

`EffectChain` applies a short sequence directly to an image's GPU texture,
including procedural inputs. `Blur` is a paired Gaussian filter with a standard
deviation in image pixels; `Dither` applies an ordered 4×4 Bayer threshold per
color channel and preserves alpha. Reversing their order changes the result.

```rust,no_run
use std::sync::Arc;
use lumine::image::{EffectChain, EffectStage, ImageData};

let input = Arc::new(ImageData::new(16, 16, [160, 90, 30, 255].repeat(16 * 16)).unwrap());
let mut chain = EffectChain::new();
let filtered = chain.render(input.clone(), &[
    EffectStage::Blur { radius: 3.0 },
    EffectStage::Dither { levels: 4, cell_size: 1 },
]).unwrap();
// Changing only the final stage reuses the cached blur and its intermediate.
let retuned = chain.render(input, &[
    EffectStage::Blur { radius: 3.0 },
    EffectStage::Dither { levels: 8, cell_size: 1 },
]).unwrap();
```

Keep the chain beside the shader instance inside `image_signal` to animate
stage parameters. The GPU retains intermediate textures, buffers and bind groups,
reuses unchanged prefix stages and dispatches only the changed suffix. Input
changes invalidate the entire chain. Live immutable snapshots can share a prefix
while retaining distinct later outputs. Software evaluation remains lazy and
filters premultiplied values before returning straight-alpha bytes; GPU filtering
does not read pixels back to the CPU.

Chains support at most eight stages including filtered inputs, blur radii in `(0, 64]`, dither levels in
`2..=256`, and cell sizes in `1..=4096` image pixels. Intermediate textures have
a 64 MiB cache budget. These are content-image effects; a scene's backdrop blur
continues to use its bounded background capture and renderer-wide
`BlurAlgorithm::{Gaussian, DualKawase}` selection. Gaussian caches retain the raw
and filtered crops; horizontal scratch is shared across panels. Matching texture
extents use three filter passes including compositing. Different extents need one
extra shader draw to preserve the raw crop. A cache hit uses only compositing.
Backdrop samples clamp to the
visible panel crop, preserving the existing edge behavior.
