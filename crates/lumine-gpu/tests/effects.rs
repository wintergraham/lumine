use lumine::scene::*;
use lumine_gpu::{BlurAlgorithm, GpuRenderer};
const FILL: &str = "@group(0) @binding(0) var<storage,read> p:array<f32>; @group(0) @binding(1) var out:texture_storage_2d<rgba8unorm,write>; @compute @workgroup_size(1) fn main(@builtin(global_invocation_id) i:vec3<u32>) {if all(i.xy<textureDimensions(out)) {textureStore(out,vec2<i32>(i.xy),vec4(p[0],p[1],p[2],1.));}}";

fn fixed(w: f32, h: f32) -> Style {
    Style {
        width: Some(w),
        height: Some(h),
        ..Default::default()
    }
}

#[test]
fn svg_atlas_cells_are_rebuilt_after_a_scale_round_trip() {
    use std::sync::Arc;
    use lumine::svg::SvgData;
    let mut scene = Scene::new(20., 8.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    for (x, color) in [(0., "red"), (10., "lime")] {
        let source = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><rect width='8' height='8' fill='{color}'/></svg>"
        );
        let svg = Arc::new(SvgData::new(source.as_bytes()).unwrap());
        let node = scene.append(scene.root(), NodeKind::Svg(svg), fixed(8., 8.));
        scene.set_transform(node, Transform { x, y: 0. });
    }
    let mut gpu = GpuRenderer::new(40, 16).unwrap();
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    let original = gpu.readback().unwrap();
    for scale in [2., 1.] {
        gpu.set_scale_factor(scale);
        gpu.render(&scene, &[]).unwrap();
        gpu.readback().unwrap();
    }
    gpu.render(&scene, &[]).unwrap();
    assert_eq!(
        original,
        gpu.readback().unwrap(),
        "old-size SVG raster IDs must not reuse overwritten cells"
    );
}

#[test]
fn edge_fade_preserves_side_edges_in_flat_and_isolated_drawing() {
    for isolated in [false, true] {
        let mut scene = Scene::new(32., 32.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let node = scene.append(
            scene.root(),
            NodeKind::Rect(Color(255, 0, 0, 255)),
            fixed(32., 32.),
        );
        scene.set_isolated(node, isolated);
        scene.set_effects(
            node,
            Effects {
                edge_fade: 8.,
                ..Default::default()
            },
        );
        let mut gpu = GpuRenderer::new(32, 32).unwrap();
        gpu.set_background(Color(0, 0, 0, 0));
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let pixels = gpu.readback().unwrap();
        assert_eq!(
            &pixels[(16 * 32) * 4..(16 * 32) * 4 + 4],
            &[255, 0, 0, 255],
            "isolated={isolated}"
        );
        assert!(pixels[16 * 4 + 3] < 20, "top fade, isolated={isolated}");
    }
}

#[test]
fn cached_backdrop_edge_fade_preserves_side_edges_for_both_algorithms() {
    for algorithm in [BlurAlgorithm::Gaussian, BlurAlgorithm::DualKawase] {
        let mut scene = Scene::new(64., 64.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        scene.append(
            scene.root(),
            NodeKind::Rect(Color(0, 0, 0, 255)),
            fixed(64., 64.),
        );
        for y in [0., 32.] {
            let stripe = scene.append(
                scene.root(),
                NodeKind::Rect(Color(255, 255, 255, 255)),
                fixed(64., 1.),
            );
            scene.set_transform(stripe, Transform { x: 0., y });
        }
        let panel = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            fixed(64., 64.),
        );
        scene.set_effects(
            panel,
            Effects {
                blur_radius: 4.,
                ..Default::default()
            },
        );
        let mut gpu = GpuRenderer::new(64, 64).unwrap();
        gpu.set_blur_algorithm(algorithm);
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let full = gpu.readback().unwrap();
        scene.set_effects(
            panel,
            Effects {
                blur_radius: 4.,
                edge_fade: 8.,
                ..Default::default()
            },
        );
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let faded = gpu.readback().unwrap();
        let left = (32 * 64) * 4;
        let top = 32 * 4;
        assert_eq!(
            &faded[left..left + 4],
            &full[left..left + 4],
            "algorithm={algorithm:?}"
        );
        assert!(
            faded[top] > full[top] + 20,
            "top blur fade, algorithm={algorithm:?}"
        );
    }
}

#[test]
fn persistent_shaders_reuse_resources_and_preserve_live_snapshots() {
    use lumine::image::ShaderInstance;
    let mut instance = ShaderInstance::new(FILL);
    let red = instance
        .render(4, 4, &[1., 0., 0.], [4, 4], || {
            [255, 0, 0, 255].repeat(16).into()
        })
        .unwrap();
    let mut scene = Scene::new(12., 4.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let node = scene.append(scene.root(), NodeKind::Image(red.clone()), fixed(4., 4.));
    let mut gpu = GpuRenderer::new(12, 4).unwrap();
    let report = scene.flush();
    assert_eq!(
        gpu.render(&scene, &report.damage)
            .unwrap()
            .shader_resource_allocations,
        1
    );
    gpu.readback().unwrap();
    let green = instance
        .render(4, 4, &[0., 1., 0.], [4, 4], || {
            [0, 255, 0, 255].repeat(16).into()
        })
        .unwrap();
    assert_ne!(red.id(), green.id());
    assert_eq!(red.pixels()[0], 255);
    assert_eq!(green.pixels()[1], 255);
    scene.set_kind(node, NodeKind::Image(green.clone()));
    let report = scene.flush();
    let stats = gpu.render(&scene, &report.damage).unwrap();
    assert_eq!(stats.shader_resource_allocations, 0);
    assert_eq!(stats.shader_dispatches, 1);
    assert_eq!(&gpu.readback().unwrap()[..4], &[0, 255, 0, 255]);
    let old = scene.append(scene.root(), NodeKind::Image(red.clone()), fixed(4., 4.));
    scene.set_transform(old, Transform { x: 8., y: 0. });
    let report = scene.flush();
    assert_eq!(
        gpu.render(&scene, &report.damage)
            .unwrap()
            .shader_resource_allocations,
        1
    );
    let pixels = gpu.readback().unwrap();
    assert_eq!(&pixels[..4], &[0, 255, 0, 255]);
    assert_eq!(&pixels[32..36], &[255, 0, 0, 255]);
    let same = instance
        .render(4, 4, &[0., 1., 0.], [4, 4], || {
            panic!("unchanged uniforms must retain lazy fallback")
        })
        .unwrap();
    assert_eq!(same.id(), green.id());
    let idle = gpu.render(&scene, &[]).unwrap();
    assert_eq!(idle.shader_dispatches, 0);
    assert_eq!(idle.shader_resource_allocations, 0);
}

#[test]
fn shader_parameter_budget_rejection_keeps_earlier_encoded_images_valid() {
    use lumine::image::ShaderInstance;
    let mut good = ShaderInstance::new(FILL);
    let red = good
        .render(1, 1, &[1., 0., 0.], [1, 1], || panic!("GPU only"))
        .unwrap();
    let mut bad = ShaderInstance::new(FILL);
    // Only CPU parameters are large: reject before allocating their GPU buffer.
    let oversized = bad
        .render(1, 1, &vec![0.; 64 * 1024 * 1024 / 4], [1, 1], || {
            panic!("GPU only")
        })
        .unwrap();
    let mut scene = Scene::new(1., 1.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    scene.append(scene.root(), NodeKind::Image(red), fixed(1., 1.));
    let node = scene.append(scene.root(), NodeKind::Image(oversized), fixed(1., 1.));
    let damage = scene.flush().damage;
    let mut gpu = GpuRenderer::new(1, 1).unwrap();
    assert!(
        gpu.render(&scene, &damage)
            .unwrap_err()
            .0
            .contains("procedural image resources")
    );
    assert!(gpu.debug_cache_stats().shader_resource_bytes < 64 * 1024 * 1024);
    scene.set_kind(node, NodeKind::Rect(Color(0, 0, 0, 0)));
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    assert_eq!(gpu.readback().unwrap(), [255, 0, 0, 255]);
}

#[test]
fn kawase_is_swappable_bounded_and_matches_fresh_after_damage() {
    let mut scene = Scene::new(96., 64.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    for x in 0..12 {
        let id = scene.append(
            scene.root(),
            NodeKind::Rect(if x % 2 == 0 {
                Color(240, 20, 40, 255)
            } else {
                Color(10, 50, 220, 255)
            }),
            fixed(8., 64.),
        );
        scene.set_transform(
            id,
            Transform {
                x: x as f32 * 8.,
                y: 0.,
            },
        );
    }
    let panel = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Overlay),
        fixed(43.5, 37.25),
    );
    scene.set_transform(panel, Transform { x: 16.25, y: 13.5 });
    scene.set_effects(
        panel,
        Effects {
            blur_radius: 8.,
            opacity: 0.8,
            edge_fade: 3.,
        },
    );
    let report = scene.flush();
    let mut gpu = GpuRenderer::new(120, 80).unwrap();
    gpu.set_scale_factor(1.25);
    gpu.render(&scene, &report.damage).unwrap();
    let gaussian = gpu.readback().unwrap();
    gpu.set_blur_algorithm(BlurAlgorithm::DualKawase);
    let stats = gpu.render(&scene, &[]).unwrap();
    let kawase = gpu.readback().unwrap();
    assert!(stats.blur_passes > 2);
    assert_ne!(gaussian, kawase);
    assert_eq!(
        &gaussian[..20 * 4],
        &kawase[..20 * 4],
        "filter must not alter the outside backdrop"
    );
    gpu.set_blur_algorithm(BlurAlgorithm::Gaussian);
    gpu.render(&scene, &[]).unwrap();
    assert_eq!(
        gaussian,
        gpu.readback().unwrap(),
        "rollback must restore exact Gaussian output"
    );
    gpu.set_blur_algorithm(BlurAlgorithm::DualKawase);
    gpu.render(&scene, &[]).unwrap();
    gpu.readback().unwrap();
    scene.set_kind(
        scene.children(scene.root())[3],
        NodeKind::Rect(Color(40, 230, 20, 255)),
    );
    let report = scene.flush();
    gpu.render(&scene, &report.damage).unwrap();
    let incremental = gpu.readback().unwrap();
    let mut fresh = GpuRenderer::new_with_context(120, 80, &gpu.context()).unwrap();
    fresh.set_scale_factor(1.25);
    fresh.set_blur_algorithm(BlurAlgorithm::DualKawase);
    fresh
        .render(&scene, &[Rect::new(0., 0., 96., 64.)])
        .unwrap();
    assert_eq!(incremental, fresh.readback().unwrap());
    gpu.trim();
    gpu.set_blur_algorithm(BlurAlgorithm::Gaussian);
    gpu.set_blur_algorithm(BlurAlgorithm::DualKawase);
    gpu.render(&scene, &[]).unwrap();
    assert_eq!(incremental, gpu.readback().unwrap());
}

#[test]
fn cached_filter_opacity_and_fade_invalidate_overlapping_downstream_filters() {
    for algorithm in [BlurAlgorithm::Gaussian, BlurAlgorithm::DualKawase] {
        let mut scene = Scene::new(64., 40.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        for x in 0..8 {
            let stripe = scene.append(
                scene.root(),
                NodeKind::Rect(if x % 2 == 0 {
                    Color(230, 20, 50, 255)
                } else {
                    Color(10, 80, 210, 255)
                }),
                fixed(8., 40.),
            );
            scene.set_transform(
                stripe,
                Transform {
                    x: x as f32 * 8.,
                    y: 0.,
                },
            );
        }
        let first = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            fixed(32., 24.),
        );
        scene.set_transform(first, Transform { x: 8., y: 4. });
        scene.set_effects(
            first,
            Effects {
                blur_radius: 6.,
                ..Default::default()
            },
        );
        let second = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            fixed(32., 24.),
        );
        scene.set_transform(second, Transform { x: 24., y: 8. });
        scene.set_effects(
            second,
            Effects {
                blur_radius: 6.,
                ..Default::default()
            },
        );
        let mut gpu = GpuRenderer::new(64, 40).unwrap();
        gpu.set_blur_algorithm(algorithm);
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let initial = gpu.readback().unwrap();
        scene.set_effects(
            first,
            Effects {
                blur_radius: 6.,
                opacity: 0.3,
                edge_fade: 4.,
            },
        );
        let damage = scene.flush().damage;
        let stats = gpu.render(&scene, &damage).unwrap();
        assert!(
            stats.blur_cache_hits >= 1,
            "first panel reuses its pure filtered pixels"
        );
        assert!(
            stats.filtered_pixels > 0,
            "second panel's source includes the changed first composite"
        );
        let changed = gpu.readback().unwrap();
        assert_ne!(initial, changed);
        let mut fresh = GpuRenderer::new_with_context(64, 40, &gpu.context()).unwrap();
        fresh.set_blur_algorithm(algorithm);
        fresh
            .render(&scene, &[Rect::new(0., 0., 64., 40.)])
            .unwrap();
        assert_eq!(
            changed,
            fresh.readback().unwrap(),
            "downstream cache must match a fresh render"
        );
        scene.set_effects(
            second,
            Effects {
                blur_radius: 6.,
                opacity: 0.4,
                edge_fade: 3.,
            },
        );
        let damage = scene.flush().damage;
        let stats = gpu.render(&scene, &damage).unwrap();
        assert!(stats.blur_cache_hits >= 1);
        assert_eq!(
            stats.filtered_pixels, 0,
            "a final panel's opacity/fade only changes compositing"
        );
        let changed = gpu.readback().unwrap();
        let mut fresh = GpuRenderer::new_with_context(64, 40, &gpu.context()).unwrap();
        fresh.set_blur_algorithm(algorithm);
        fresh
            .render(&scene, &[Rect::new(0., 0., 64., 40.)])
            .unwrap();
        assert_eq!(changed, fresh.readback().unwrap());
    }
}

#[test]
fn transformed_images_contribute_their_actual_paint_bounds_to_blur_cache() {
    use std::sync::Arc;
    use lumine::{affine::Affine, image::ImageData};
    for algorithm in [BlurAlgorithm::Gaussian, BlurAlgorithm::DualKawase] {
        let mut scene = Scene::new(64., 32.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let image = |rgb: [u8; 4]| {
            Arc::new(
                ImageData::new(12, 12, rgb.repeat(12 * 12))
                    .unwrap()
                    .transformed(Affine::translation(40., 10.)),
            )
        };
        let source = scene.append(
            scene.root(),
            NodeKind::Image(image([230, 20, 40, 255])),
            fixed(12., 12.),
        );
        let panel = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            fixed(12., 12.),
        );
        scene.set_transform(panel, Transform { x: 40., y: 10. });
        scene.set_effects(
            panel,
            Effects {
                blur_radius: 4.,
                ..Default::default()
            },
        );
        let mut gpu = GpuRenderer::new(64, 32).unwrap();
        gpu.set_blur_algorithm(algorithm);
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let old = gpu.readback().unwrap();
        scene.set_kind(source, NodeKind::Image(image([10, 70, 240, 255])));
        let damage = scene.flush().damage;
        let stats = gpu.render(&scene, &damage).unwrap();
        assert!(
            stats.filtered_pixels > 0,
            "image allocation misses panel, but transformed pixels overlap it"
        );
        let changed = gpu.readback().unwrap();
        assert_ne!(old, changed);
        let mut fresh = GpuRenderer::new_with_context(64, 32, &gpu.context()).unwrap();
        fresh.set_blur_algorithm(algorithm);
        fresh
            .render(&scene, &[Rect::new(0., 0., 64., 32.)])
            .unwrap();
        assert_eq!(changed, fresh.readback().unwrap());
    }
}

#[test]
fn foreground_damage_reuses_bounded_filter_but_background_damage_invalidates() {
    for algorithm in [BlurAlgorithm::Gaussian, BlurAlgorithm::DualKawase] {
        let mut scene = Scene::new(128., 80.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let background = scene.append(
            scene.root(),
            NodeKind::Rect(Color(200, 10, 50, 255)),
            fixed(128., 80.),
        );
        let panel = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            fixed(40., 40.),
        );
        scene.set_transform(panel, Transform { x: 10., y: 10. });
        scene.set_effects(
            panel,
            Effects {
                blur_radius: 12.,
                ..Default::default()
            },
        );
        let cursor = scene.append(
            scene.root(),
            NodeKind::Rect(Color(255, 255, 255, 255)),
            fixed(2., 12.),
        );
        scene.set_transform(cursor, Transform { x: 22., y: 22. });
        let mut gpu = GpuRenderer::new(128, 80).unwrap();
        gpu.set_blur_algorithm(algorithm);
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        gpu.readback().unwrap();
        scene.set_kind(cursor, NodeKind::Rect(Color(30, 255, 10, 255)));
        let damage = scene.flush().damage;
        let cached = gpu.render(&scene, &damage).unwrap();
        assert_eq!(cached.blur_cache_hits, 1);
        assert_eq!(cached.blur_passes, 1);
        assert_eq!(cached.filtered_pixels, 0);
        assert_eq!(cached.blur_texture_allocations, 0);
        let pixels = gpu.readback().unwrap();
        let mut fresh = GpuRenderer::new_with_context(128, 80, &gpu.context()).unwrap();
        fresh.set_blur_algorithm(algorithm);
        fresh.render(&scene, &[scene.bounds(scene.root())]).unwrap();
        assert_eq!(pixels, fresh.readback().unwrap());
        scene.set_transform(cursor, Transform { x: 104., y: 22. });
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        gpu.readback().unwrap();
        scene.set_kind(cursor, NodeKind::Rect(Color(0, 0, 0, 255)));
        let damage = scene.flush().damage;
        let distant = gpu.render(&scene, &damage).unwrap();
        assert_eq!(distant.blur_passes, 0);
        scene.set_kind(background, NodeKind::Rect(Color(10, 30, 230, 255)));
        let damage = scene.flush().damage;
        let changed = gpu.render(&scene, &damage).unwrap();
        assert_eq!(changed.blur_cache_hits, 0);
        assert!(changed.filtered_pixels > 0);
        assert_eq!(changed.blur_texture_allocations, 0);
    }
}

#[test]
fn inherited_fade_masks_attenuate_blur_and_invalidate_downstream_composites() {
    for algorithm in [BlurAlgorithm::Gaussian, BlurAlgorithm::DualKawase] {
        let mut scene = Scene::new(64., 48.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        for x in 0..32 {
            let stripe = scene.append(
                scene.root(),
                NodeKind::Rect(if x % 2 == 0 {
                    Color(240, 20, 20, 255)
                } else {
                    Color(0, 20, 20, 255)
                }),
                fixed(2., 48.),
            );
            scene.set_transform(
                stripe,
                Transform {
                    x: x as f32 * 2.,
                    y: 0.,
                },
            );
        }
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                fade_edges: [12., 12.],
                clip: true,
                ..fixed(40., 48.)
            },
        );
        let first = scene.append(
            parent,
            NodeKind::Container(Layout::Overlay),
            fixed(40., 48.),
        );
        scene.set_effects(
            first,
            Effects {
                blur_radius: 6.,
                ..Default::default()
            },
        );
        let second = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            fixed(32., 32.),
        );
        scene.set_transform(second, Transform { x: 24., y: 8. });
        scene.set_effects(
            second,
            Effects {
                blur_radius: 6.,
                ..Default::default()
            },
        );
        let mut gpu = GpuRenderer::new(64, 48).unwrap();
        gpu.set_blur_algorithm(algorithm);
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let masked = gpu.readback().unwrap();
        let pixel = |pixels: &[u8], x: usize, y: usize| pixels[(y * 64 + x) * 4];
        assert!(
            pixel(&masked, 8, 0).abs_diff(240) <= 1,
            "blur must fade almost completely at top edge"
        );
        assert!(
            pixel(&masked, 8, 24).abs_diff(240) > 50,
            "center remains fully blurred"
        );
        scene.set_style(
            parent,
            Style {
                clip: true,
                ..fixed(40., 48.)
            },
        );
        let damage = scene.flush().damage;
        let stats = gpu.render(&scene, &damage).unwrap();
        assert!(
            stats.blur_cache_hits >= 1,
            "mask change reuses first pure filter"
        );
        assert!(
            stats.filtered_pixels > 0,
            "downstream input includes changed mask composite"
        );
        let plain = gpu.readback().unwrap();
        assert_ne!(masked, plain);
        assert_eq!(pixel(&masked, 8, 24), pixel(&plain, 8, 24));
        assert!(pixel(&plain, 8, 0).abs_diff(240) > 50);
        let mut fresh = GpuRenderer::new_with_context(64, 48, &gpu.context()).unwrap();
        fresh.set_blur_algorithm(algorithm);
        fresh
            .render(&scene, &[Rect::new(0., 0., 64., 48.)])
            .unwrap();
        assert_eq!(plain, fresh.readback().unwrap());
    }
}

#[test]
fn full_4k_gaussian_filter_stays_cached_across_foreground_damage_and_releases_scratch() {
    let mut scene = Scene::new(3840., 2160.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    scene.append(
        scene.root(),
        NodeKind::Rect(Color(30, 80, 140, 255)),
        fixed(3840., 2160.),
    );
    let panel = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Overlay),
        fixed(3840., 2160.),
    );
    scene.set_effects(
        panel,
        Effects {
            blur_radius: 0.1,
            ..Default::default()
        },
    );
    let cursor = scene.append(
        scene.root(),
        NodeKind::Rect(Color(255, 255, 255, 255)),
        fixed(2., 12.),
    );
    scene.set_transform(cursor, Transform { x: 20., y: 20. });
    let mut gpu = GpuRenderer::new(3840, 2160).unwrap();
    let damage = scene.flush().damage;
    assert_eq!(
        gpu.render(&scene, &damage)
            .unwrap()
            .blur_texture_allocations,
        3
    );
    gpu.readback().unwrap();
    let first = gpu.debug_cache_stats();
    assert!(
        first.blur_cache_bytes > 60 * 1024 * 1024 && first.blur_cache_bytes <= 64 * 1024 * 1024
    );
    assert_eq!(first.blur_scratch_bytes, 3840 * 2176 * 4);

    scene.set_kind(cursor, NodeKind::Rect(Color(0, 255, 0, 255)));
    let damage = scene.flush().damage;
    let stats = gpu.render(&scene, &damage).unwrap();
    assert_eq!(stats.blur_cache_hits, 1);
    assert_eq!(stats.filtered_pixels, 0);
    assert_eq!(stats.blur_texture_allocations, 0);
    gpu.readback().unwrap();
    let second = gpu.debug_cache_stats();
    assert_eq!(first.blur_cache_bytes, second.blur_cache_bytes);
    assert_eq!(first.blur_scratch_bytes, second.blur_scratch_bytes);
    gpu.resize(64, 64);
    assert_eq!(gpu.debug_cache_stats().blur_scratch_bytes, 0);
    gpu.trim();
    let empty = gpu.debug_cache_stats();
    assert_eq!(empty.blur_cache_bytes, 0);
    assert_eq!(empty.blur_scratch_bytes, 0);
}

#[test]
fn shared_gaussian_scratch_handles_different_crop_extents_and_recycled_caches() {
    for widths in [[40., 120.], [120., 40.]] {
        let mut scene = Scene::new(192., 96.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let background = scene.append(
            scene.root(),
            NodeKind::Rect(Color(10, 80, 130, 255)),
            fixed(192., 96.),
        );
        for x in 0..16 {
            let stripe = scene.append(
                scene.root(),
                NodeKind::Rect(Color(220, 30, 80, 255)),
                fixed(6., 96.),
            );
            scene.set_transform(
                stripe,
                Transform {
                    x: x as f32 * 12.,
                    y: 0.,
                },
            );
        }
        let mut panels = Vec::new();
        for (i, width) in widths.into_iter().enumerate() {
            let panel = scene.append(
                scene.root(),
                NodeKind::Container(Layout::Overlay),
                fixed(width, 72.),
            );
            scene.set_transform(
                panel,
                Transform {
                    x: i as f32 * 56.,
                    y: 4.,
                },
            );
            scene.set_effects(
                panel,
                Effects {
                    blur_radius: 2.,
                    ..Default::default()
                },
            );
            panels.push(panel);
        }
        let mut gpu = GpuRenderer::new(192, 96).unwrap();
        let damage = scene.flush().damage;
        let stats = gpu.render(&scene, &damage).unwrap();
        assert_eq!(stats.blur_passes, if widths[0] > widths[1] { 7 } else { 6 });
        gpu.readback().unwrap();
        // Force a new small crop while recycling the formerly large raw/filtered
        // textures, then invalidate both sources with one background change.
        scene.set_style(panels[1], fixed(24., 24.));
        scene.set_kind(background, NodeKind::Rect(Color(40, 100, 160, 255)));
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let changed = gpu.readback().unwrap();
        let mut fresh = GpuRenderer::new_with_context(192, 96, &gpu.context()).unwrap();
        fresh.render(&scene, &[scene.bounds(scene.root())]).unwrap();
        assert_eq!(changed, fresh.readback().unwrap());
    }
}
