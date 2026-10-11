use std::sync::Arc;
use lumine::{
    image::{EffectChain, EffectStage, ImageData, ShaderInstance},
    scene::*,
};
use lumine_gpu::GpuRenderer;

fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
fn input(width: u32, height: u32) -> Arc<ImageData> {
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&[
                (x * 17 + y * 7) as u8,
                (x * 3 + y * 23) as u8,
                (180 + x * 5) as u8,
                255,
            ]);
        }
    }
    Arc::new(ImageData::new(width, height, pixels).unwrap())
}

#[test]
fn ordered_chains_reuse_prefixes_cascade_changes_resize_and_preserve_snapshots() {
    let source = input(16, 16);
    let blur = EffectStage::Blur { radius: 1.5 };
    let dither = EffectStage::Dither {
        levels: 4,
        cell_size: 1,
    };
    let mut chain = EffectChain::new();
    let original = chain.render(source.clone(), &[blur, dither]).unwrap();
    assert_eq!(
        original.id(),
        chain.render(source.clone(), &[blur, dither]).unwrap().id()
    );
    let mut reverse = EffectChain::new();
    let reversed = reverse.render(source.clone(), &[dither, blur]).unwrap();
    assert_ne!(
        original.pixels(),
        reversed.pixels(),
        "stage order must affect the image"
    );
    let mut scene = Scene::new(32., 16.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let node = scene.append(
        scene.root(),
        NodeKind::Image(original.clone()),
        fixed(16., 16.),
    );
    let second = scene.append(scene.root(), NodeKind::Image(reversed), fixed(16., 16.));
    scene.set_transform(second, Transform { x: 16., y: 0. });
    let mut gpu = GpuRenderer::new(32, 16).unwrap();
    let damage = scene.flush().damage;
    let stats = gpu.render(&scene, &damage).unwrap();
    assert_eq!(stats.shader_dispatches, 6);
    let ordered = gpu.readback().unwrap();
    let left: Vec<_> = ordered
        .as_chunks::<{ 32 * 4 }>()
        .0
        .iter()
        .flat_map(|row| &row[..16 * 4])
        .copied()
        .collect();
    let right: Vec<_> = ordered
        .as_chunks::<{ 32 * 4 }>()
        .0
        .iter()
        .flat_map(|row| &row[16 * 4..])
        .copied()
        .collect();
    assert_ne!(left, right);
    // Asymmetric colors catch BGRA source upload / RGBA filter output mistakes.
    let difference: usize = left
        .iter()
        .zip(original.pixels())
        .map(|(a, b)| a.abs_diff(*b) as usize)
        .sum();
    assert!(
        difference < left.len() * 4,
        "GPU and lazy software fallback diverged: mean {}",
        difference as f32 / left.len() as f32
    );

    let changed = chain
        .render(
            source.clone(),
            &[
                blur,
                EffectStage::Dither {
                    levels: 8,
                    cell_size: 1,
                },
            ],
        )
        .unwrap();
    scene.set_kind(node, NodeKind::Image(changed.clone()));
    let damage = scene.flush().damage;
    let stats = gpu.render(&scene, &damage).unwrap();
    assert_eq!(
        stats.shader_dispatches, 1,
        "unchanged blur prefix must stay cached"
    );
    assert_eq!(stats.shader_resource_allocations, 0);
    let changed_pixels = gpu.readback().unwrap();
    assert_ne!(changed_pixels, ordered);

    scene.set_kind(second, NodeKind::Image(original.clone()));
    let damage = scene.flush().damage;
    let stats = gpu.render(&scene, &damage).unwrap();
    assert_eq!(
        stats.shader_dispatches, 1,
        "old and current snapshot share blur and own separate dither outputs"
    );
    assert_eq!(stats.shader_resource_allocations, 1);
    let coexist = gpu.readback().unwrap();
    for (row, old) in coexist
        .as_chunks::<{ 32 * 4 }>()
        .0
        .iter()
        .zip(left.as_chunks::<{ 16 * 4 }>().0)
    {
        assert_eq!(&row[16 * 4..], old);
    }

    let wider_blur = chain
        .render(source, &[EffectStage::Blur { radius: 2.5 }, dither])
        .unwrap();
    scene.set_kind(node, NodeKind::Image(wider_blur));
    let damage = scene.flush().damage;
    assert_eq!(
        gpu.render(&scene, &damage).unwrap().shader_dispatches,
        3,
        "blur change invalidates its suffix"
    );
    gpu.readback().unwrap();
    let resized = chain.render(input(8, 8), &[blur, dither]).unwrap();
    scene.set_kind(node, NodeKind::Image(resized));
    let damage = scene.flush().damage;
    let stats = gpu.render(&scene, &damage).unwrap();
    assert_eq!(stats.shader_dispatches, 3);
    assert_eq!(stats.shader_resource_allocations, 3);
    gpu.readback().unwrap();
    assert_eq!(gpu.render(&scene, &[]).unwrap().shader_dispatches, 0);
}

#[test]
fn chains_filter_procedural_sources_on_gpu_and_preserve_premultiplied_alpha() {
    const FILL: &str = "@group(0) @binding(0) var<storage,read> p:array<f32>; @group(0) @binding(1) var out:texture_storage_2d<rgba8unorm,write>; @compute @workgroup_size(1) fn main(@builtin(global_invocation_id) i:vec3<u32>) {if all(i.xy<textureDimensions(out)) {textureStore(out,vec2<i32>(i.xy),vec4(p[0],p[1],p[2],p[3]));}}";
    let mut shader = ShaderInstance::new(FILL);
    let source = shader
        .render(8, 8, &[0.5, 0.125, 0.25, 0.5], [8, 8], || {
            panic!("GPU chains must not read procedural CPU fallback")
        })
        .unwrap();
    let mut chain = EffectChain::new();
    let image = chain
        .render(
            source,
            &[
                EffectStage::Blur { radius: 2. },
                EffectStage::Dither {
                    levels: 5,
                    cell_size: 1,
                },
            ],
        )
        .unwrap();
    let mut scene = Scene::new(8., 8.);
    scene.append(scene.root(), NodeKind::Image(image), fixed(8., 8.));
    let damage = scene.flush().damage;
    let mut gpu = GpuRenderer::new(8, 8).unwrap();
    assert_eq!(gpu.render(&scene, &damage).unwrap().shader_dispatches, 4);
    for pixel in gpu.readback().unwrap().as_chunks::<4>().0 {
        assert_eq!(pixel[3], 128);
        assert!(
            pixel[0].abs_diff(128) <= 1 && pixel[1].abs_diff(32) <= 1 && pixel[2].abs_diff(64) <= 1,
            "premultiplied color changed: {pixel:?}"
        );
    }
}

#[test]
fn chain_validates_settings_and_keeps_transparent_color_clean() {
    let source = Arc::new(ImageData::new(1, 1, vec![255, 0, 200, 0]).unwrap());
    let mut chain = EffectChain::new();
    assert!(
        chain
            .render(source.clone(), &[EffectStage::Blur { radius: f32::NAN }])
            .is_err()
    );
    assert!(
        chain
            .render(
                source.clone(),
                &[EffectStage::Dither {
                    levels: 1,
                    cell_size: 1
                }]
            )
            .is_err()
    );
    assert_eq!(chain.render(source.clone(), &[]).unwrap().id(), source.id());
    let filtered = chain
        .render(
            source,
            &[
                EffectStage::Blur { radius: 2. },
                EffectStage::Dither {
                    levels: 2,
                    cell_size: 1,
                },
            ],
        )
        .unwrap();
    assert_eq!(filtered.pixels(), &[0, 0, 0, 0]);
}

#[test]
fn nested_filtered_inputs_share_the_eight_stage_limit() {
    let mut input = Arc::new(ImageData::new(1, 1, vec![128, 64, 32, 255]).unwrap());
    let stage = EffectStage::Dither {
        levels: 256,
        cell_size: 1,
    };
    for _ in 0..8 {
        input = EffectChain::new().render(input, &[stage]).unwrap();
    }
    assert_eq!(input.pixels(), [128, 64, 32, 255]);
    let mut ninth = EffectChain::new();
    assert!(
        ninth
            .render(input.clone(), &[stage])
            .unwrap_err()
            .contains("eight")
    );
    assert_eq!(ninth.render(input.clone(), &[]).unwrap().id(), input.id());
}

#[test]
fn recursive_chain_upload_obeys_image_budget_and_recovers_after_removal() {
    const FILL: &str = "@group(0) @binding(0) var<storage,read> p:array<f32>; @group(0) @binding(1) var out:texture_storage_2d<rgba8unorm,write>; @compute @workgroup_size(1) fn main() {textureStore(out,vec2<i32>(0),vec4(p[0]));}";
    let mut shader = ShaderInstance::new(FILL);
    let source = shader
        .render(3200, 3200, &[0.], [1, 1], || {
            panic!("budget rejection must not evaluate fallback")
        })
        .unwrap();
    let mut chain = EffectChain::new();
    let image = chain
        .render(
            source,
            &[EffectStage::Dither {
                levels: 2,
                cell_size: 1,
            }],
        )
        .unwrap();
    let mut scene = Scene::new(4., 4.);
    let node = scene.append(scene.root(), NodeKind::Image(image), fixed(4., 4.));
    let mut gpu = GpuRenderer::new(4, 4).unwrap();
    let damage = scene.flush().damage;
    assert!(
        gpu.render(&scene, &damage)
            .unwrap_err()
            .0
            .contains("64 MiB")
    );
    let cache = gpu.debug_cache_stats();
    assert!(cache.image_bytes <= 64 * 1024 * 1024);
    assert_eq!(cache.effect_chain_bytes, 0);
    scene.set_kind(node, NodeKind::Rect(Color(255, 0, 0, 255)));
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    assert_eq!(gpu.debug_cache_stats().image_bytes, 0);
    assert_eq!(gpu.readback().unwrap()[..4], [255, 0, 0, 255]);
}

#[test]
fn a_rejected_chain_preserves_previously_encoded_procedural_uploads() {
    const FILL: &str = "@group(0) @binding(0) var<storage,read> p:array<f32>; @group(0) @binding(1) var out:texture_storage_2d<rgba8unorm,write>; @compute @workgroup_size(1) fn main(@builtin(global_invocation_id) i:vec3<u32>) {if all(i.xy<textureDimensions(out)) {textureStore(out,vec2<i32>(i.xy),vec4(p[0],p[1],p[2],1.));}}";
    let mut good = ShaderInstance::new(FILL);
    let red = good
        .render(4, 4, &[1., 0., 0.], [4, 4], || panic!("GPU only"))
        .unwrap();
    let mut source = ShaderInstance::new(FILL);
    let input = source
        .render(2048, 2048, &[0., 1., 0.], [1, 1], || panic!("GPU only"))
        .unwrap();
    let mut chain = EffectChain::new();
    // Source/output fit the image budget, but four intermediates plus their
    // parameter buffers exceed the chain's independent 64 MiB budget.
    let rejected = chain
        .render(
            input,
            &[
                EffectStage::Blur { radius: 1. },
                EffectStage::Blur { radius: 1. },
            ],
        )
        .unwrap();
    let mut scene = Scene::new(4., 4.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    scene.append(scene.root(), NodeKind::Image(red), fixed(4., 4.));
    let bad = scene.append(scene.root(), NodeKind::Image(rejected), fixed(4., 4.));
    let damage = scene.flush().damage;
    let mut gpu = GpuRenderer::new(4, 4).unwrap();
    let error = gpu.render(&scene, &damage).unwrap_err();
    assert!(
        error.0.contains("intermediates"),
        "must exercise chain rejection after recursive upload: {error}"
    );
    scene.set_kind(bad, NodeKind::Rect(Color(0, 0, 0, 0)));
    let damage = scene.flush().damage;
    let stats = gpu.render(&scene, &damage).unwrap();
    assert_eq!(
        stats.image_uploads, 0,
        "good image ID is already cached after the failed frame"
    );
    let recovered = gpu.readback().unwrap();
    assert!(
        recovered
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [255, 0, 0, 255])
    );
    let mut fresh = GpuRenderer::new_with_context(4, 4, &gpu.context()).unwrap();
    fresh.render(&scene, &[Rect::new(0., 0., 4., 4.)]).unwrap();
    assert_eq!(recovered, fresh.readback().unwrap());
}

#[test]
fn resizing_a_chain_counts_released_intermediates_against_its_cache_budget() {
    const FILL: &str = "@group(0) @binding(0) var<storage,read> p:array<f32>; @group(0) @binding(1) var out:texture_storage_2d<rgba8unorm,write>; @compute @workgroup_size(1) fn main() {textureStore(out,vec2<i32>(0),vec4(p[0],0.,0.,1.));}";
    let mut shader = ShaderInstance::new(FILL);
    let mut chain = EffectChain::new();
    let stages = [EffectStage::Blur { radius: 0.1 }];
    let input = shader
        .render(2048, 2048, &[1.], [1, 1], || panic!("GPU only"))
        .unwrap();
    let image = chain.render(input, &stages).unwrap();
    let mut scene = Scene::new(4., 4.);
    let node = scene.append(scene.root(), NodeKind::Image(image), fixed(4., 4.));
    let mut gpu = GpuRenderer::new(4, 4).unwrap();
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    gpu.readback().unwrap();
    let before = gpu.debug_cache_stats().effect_chain_bytes;
    assert!(before > 32 * 1024 * 1024);
    let input = shader
        .render(2304, 2304, &[0.5], [1, 1], || panic!("GPU only"))
        .unwrap();
    scene.set_kind(node, NodeKind::Image(chain.render(input, &stages).unwrap()));
    let damage = scene.flush().damage;
    let stats = gpu
        .render(&scene, &damage)
        .expect("old unique intermediates are released while resizing");
    assert_eq!(stats.shader_dispatches, 3);
    assert_eq!(stats.shader_resource_allocations, 3);
    let after = gpu.debug_cache_stats().effect_chain_bytes;
    assert!(after > before && after < 64 * 1024 * 1024);
    gpu.readback().unwrap();
}
