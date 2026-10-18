use lumine::{affine::Affine, scene::*};
use lumine_gpu::{BlurAlgorithm, GpuRenderer};

fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
fn overlay() -> Scene {
    let mut scene = Scene::new(96., 80.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    scene
}
fn pixel(pixels: &[u8], x: usize, y: usize) -> &[u8] {
    &pixels[(y * 96 + x) * 4..(y * 96 + x + 1) * 4]
}
fn render(gpu: &mut GpuRenderer, scene: &mut Scene) -> Vec<u8> {
    let report = scene.flush();
    gpu.render(scene, &report.damage).unwrap();
    gpu.readback().unwrap()
}
fn fresh(gpu: &GpuRenderer, scene: &Scene, algorithm: BlurAlgorithm) -> Vec<u8> {
    let mut reference = GpuRenderer::new_with_context(96, 80, &gpu.context()).unwrap();
    reference.set_background(Color(0, 0, 0, 0));
    reference.set_blur_algorithm(algorithm);
    reference
        .render(scene, &[Rect::new(0., 0., 96., 80.)])
        .unwrap();
    reference.readback().unwrap()
}

fn same_pixels(actual: &[u8], expected: &[u8]) {
    assert_eq!(actual.len(), expected.len());
    let mismatch = actual
        .chunks(4)
        .zip(expected.chunks(4))
        .enumerate()
        .find(|(_, (a, b))| a != b);
    assert!(mismatch.is_none(), "first mismatch: {mismatch:?}");
}

#[test]
fn rotated_subtree_clips_exactly_and_keeps_ordinary_draws_untransformed() {
    let mut scene = overlay();
    let parent = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Overlay),
        Style {
            clip: true,
            ..fixed(24., 16.)
        },
    );
    scene.append(
        parent,
        NodeKind::Rect(Color(255, 0, 0, 255)),
        fixed(60., 60.),
    );
    scene.set_paint_transform(
        parent,
        Affine::rotation(0.6).then(Affine::translation(35., 16.)),
    );
    let ordinary = scene.append(
        scene.root(),
        NodeKind::Rect(Color(0, 255, 0, 255)),
        fixed(8., 8.),
    );
    scene.set_transform(ordinary, Transform { x: 80., y: 65. });
    let mut gpu = GpuRenderer::new(96, 80).unwrap();
    gpu.set_background(Color(0, 0, 0, 0));
    let pixels = render(&mut gpu, &mut scene);
    let inverse = scene.paint_matrix(parent).inverse().unwrap();
    for y in 8..48 {
        for x in 20..70 {
            let (lx, ly) = inverse.point(x as f32 + 0.5, y as f32 + 0.5);
            let expected = (0. ..24.).contains(&lx) && (0. ..16.).contains(&ly);
            assert_eq!(
                pixel(&pixels, x, y)[3] > 0,
                expected,
                "({x},{y}) local=({lx},{ly})"
            );
        }
    }
    assert_eq!(pixel(&pixels, 83, 68), &[0, 255, 0, 255]);
    scene.set_paint_transform(
        parent,
        Affine::rotation(-0.35).then(Affine::translation(25., 30.)),
    );
    let moved = render(&mut gpu, &mut scene);
    same_pixels(&moved, &fresh(&gpu, &scene, BlurAlgorithm::Gaussian));
}

#[test]
fn affine_background_and_clip_changes_invalidate_blur_at_equal_bounds() {
    for algorithm in [BlurAlgorithm::Gaussian, BlurAlgorithm::DualKawase] {
        let mut scene = overlay();
        let background = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            fixed(40., 40.),
        );
        scene.append(
            background,
            NodeKind::Rect(Color(230, 20, 40, 255)),
            fixed(20., 40.),
        );
        let right = scene.append(
            background,
            NodeKind::Rect(Color(10, 80, 220, 255)),
            fixed(20., 40.),
        );
        scene.set_transform(right, Transform { x: 20., y: 0. });
        scene.set_transform(background, Transform { x: 20., y: 20. });
        let panel = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            fixed(40., 40.),
        );
        scene.set_transform(panel, Transform { x: 20., y: 20. });
        scene.set_effects(
            panel,
            Effects {
                blur_radius: 5.,
                ..Default::default()
            },
        );
        let mut gpu = GpuRenderer::new(96, 80).unwrap();
        gpu.set_background(Color(0, 0, 0, 0));
        gpu.set_blur_algorithm(algorithm);
        let before = render(&mut gpu, &mut scene);
        scene.set_paint_transform_origin(background, Affine::scale(-1., 1.), [0.5, 0.5]);
        let report = scene.flush();
        assert_eq!(report.layout_nodes, 0);
        let stats = gpu.render(&scene, &report.damage).unwrap();
        assert!(
            stats.filtered_pixels > 0,
            "{algorithm:?} must see changed affine source"
        );
        let after = gpu.readback().unwrap();
        assert_ne!(before, after);
        same_pixels(&after, &fresh(&gpu, &scene, algorithm));
    }
}

#[test]
fn rotated_backdrop_filters_keep_the_world_backdrop_and_exact_shape() {
    for algorithm in [BlurAlgorithm::Gaussian, BlurAlgorithm::DualKawase] {
        let mut scene = overlay();
        for x in 0..24 {
            let stripe = scene.append(
                scene.root(),
                NodeKind::Rect(if x % 2 == 0 {
                    Color(240, 30, 60, 255)
                } else {
                    Color(10, 80, 220, 255)
                }),
                fixed(4., 80.),
            );
            scene.set_transform(
                stripe,
                Transform {
                    x: x as f32 * 4.,
                    y: 0.,
                },
            );
        }
        let panel = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            fixed(32., 24.),
        );
        let transform = Affine::rotation(0.5).then(Affine::translation(35., 20.));
        scene.set_paint_transform(panel, transform);
        scene.set_effects(
            panel,
            Effects {
                blur_radius: 4.,
                ..Default::default()
            },
        );
        let mut gpu = GpuRenderer::new(96, 80).unwrap();
        gpu.set_background(Color(0, 0, 0, 0));
        gpu.set_blur_algorithm(algorithm);
        let pixels = render(&mut gpu, &mut scene);
        let inverse = transform.inverse().unwrap();
        let mut changed_inside = 0;
        for y in 0..80 {
            for x in 0..96 {
                let (lx, ly) = inverse.point(x as f32 + 0.5, y as f32 + 0.5);
                let original = if (x / 4) % 2 == 0 {
                    [240, 30, 60, 255]
                } else {
                    [10, 80, 220, 255]
                };
                if (0. ..32.).contains(&lx) && (0. ..24.).contains(&ly) {
                    changed_inside += usize::from(pixel(&pixels, x, y) != original);
                } else {
                    assert_eq!(
                        pixel(&pixels, x, y),
                        original,
                        "outside {algorithm:?} ({x},{y})"
                    );
                }
            }
        }
        assert!(
            changed_inside > 200,
            "filter must sample the opaque world backdrop"
        );
        scene.set_paint_transform(
            panel,
            Affine::rotation(-0.5).then(Affine::translation(22., 30.)),
        );
        let moved = render(&mut gpu, &mut scene);
        same_pixels(&moved, &fresh(&gpu, &scene, algorithm));
    }
}

#[test]
fn rank_one_singular_transform_neither_paints_nor_filters() {
    let mut scene = overlay();
    let panel = scene.append(
        scene.root(),
        NodeKind::Rect(Color(255, 0, 0, 255)),
        fixed(30., 20.),
    );
    scene.set_paint_transform(
        panel,
        Affine {
            a: 1.,
            b: 1.,
            c: 1.,
            d: 1.,
            tx: 10.,
            ty: 10.,
        },
    );
    scene.set_effects(
        panel,
        Effects {
            blur_radius: 8.,
            ..Default::default()
        },
    );
    let mut gpu = GpuRenderer::new(96, 80).unwrap();
    gpu.set_background(Color(0, 0, 0, 0));
    assert!(render(&mut gpu, &mut scene).iter().all(|byte| *byte == 0));
}

#[test]
fn affine_isolated_layers_and_single_pass_match_fresh_after_interruption() {
    let mut scene = overlay();
    let parent = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Overlay),
        fixed(40., 30.),
    );
    scene.set_transform(parent, Transform { x: 30., y: 20. });
    let layer = scene.append(
        parent,
        NodeKind::Container(Layout::Overlay),
        fixed(30., 20.),
    );
    scene.set_isolated(layer, true);
    let painted = scene.append(
        layer,
        NodeKind::Rect(Color(240, 30, 60, 255)),
        fixed(10., 20.),
    );
    let green = scene.append(
        scene.root(),
        NodeKind::Rect(Color(0, 255, 0, 255)),
        fixed(10., 10.),
    );
    scene.set_transform(green, Transform { x: 82., y: 65. });
    let mut gpu = GpuRenderer::new(96, 80).unwrap();
    gpu.set_background(Color(0, 0, 0, 0));
    for matrix in [
        Affine::rotation(0.3),
        Affine::scale(-1., 1.),
        Affine::rotation(-0.5),
    ] {
        scene.set_paint_transform_origin(parent, matrix, [0.5, 0.5]);
        let output = render(&mut gpu, &mut scene);
        same_pixels(&output, &fresh(&gpu, &scene, BlurAlgorithm::Gaussian));
        let (x, y) = scene.local_to_world(painted, 5., 10.);
        assert_eq!(
            pixel(&output, x.floor() as usize, y.floor() as usize),
            &[240, 30, 60, 255]
        );
        assert_eq!(pixel(&output, 86, 68), &[0, 255, 0, 255]);
    }
    gpu.set_background(Color(12, 18, 25, 255));
    gpu.debug_enable_single_pass();
    for angle in [0.7, -0.2, 0.] {
        scene.set_paint_transform_origin(parent, Affine::rotation(angle), [0.5, 0.5]);
        let report = scene.flush();
        gpu.render(&scene, &report.damage).unwrap();
        let presented = gpu.debug_present_offscreen().unwrap();
        let mut reference = GpuRenderer::new_with_context(96, 80, &gpu.context()).unwrap();
        reference.set_background(Color(12, 18, 25, 255));
        reference
            .render(&scene, &[Rect::new(0., 0., 96., 80.)])
            .unwrap();
        same_pixels(&presented, &reference.debug_present_offscreen().unwrap());
    }
}

#[test]
fn image_own_affine_and_node_affine_preserve_primitive_fades_at_each_dpi() {
    use std::sync::Arc;
    use lumine::image::ImageData;
    let source = Arc::new(
        ImageData::new(1, 1, vec![255; 4])
            .unwrap()
            .transformed(Affine::rotation(0.7).around(8., 8.)),
    );
    let mut scene = overlay();
    let node = scene.append(
        scene.root(),
        NodeKind::Image(source.clone()),
        fixed(16., 16.),
    );
    scene.set_paint_transform(
        node,
        Affine::rotation(-0.4).then(Affine::translation(35., 25.)),
    );
    scene.set_effects(
        node,
        Effects {
            edge_fade: 4.,
            ..Default::default()
        },
    );
    scene.flush();
    let item = scene
        .paint_items()
        .into_iter()
        .find(|item| item.id == node)
        .unwrap();
    let inverse = source
        .paint_transform(item.bounds)
        .then(item.transform)
        .inverse()
        .unwrap();
    for scale in [1_f32, 1.5, 2.] {
        let (w, h) = ((96. * scale) as u32, (80. * scale) as u32);
        let mut gpu = GpuRenderer::new(w, h).unwrap();
        gpu.set_background(Color(0, 0, 0, 0));
        gpu.set_scale_factor(scale);
        gpu.render(&scene, &[Rect::new(0., 0., 96., 80.)]).unwrap();
        let pixels = gpu.readback().unwrap();
        let mut checked = 0;
        for y in 0..h {
            for x in 0..w {
                let (lx, ly) = inverse.point((x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale);
                // Stay away from texture/triangle boundary coverage; verify the ramp itself.
                if lx > 1. && lx < 15. && ly > 1. && ly < 15. {
                    let expected = (ly.min(16. - ly) / 4.).min(1.) * 255.;
                    let actual = pixels[((y * w + x) * 4 + 3) as usize] as f32;
                    assert!(
                        (actual - expected).abs() <= 1.1,
                        "scale={scale} ({x},{y}) {actual} != {expected}"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 100);
    }
}

#[test]
fn animated_container_keeps_sibling_batching_and_reuses_geometry_buffers() {
    let mut scene = overlay();
    let parent = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Overlay),
        Style {
            clip: true,
            ..fixed(64., 48.)
        },
    );
    for n in 0..32 {
        let node = scene.append(
            parent,
            NodeKind::Rect(Color(90, 150, 220, 255)),
            fixed(6., 6.),
        );
        scene.set_transform(
            node,
            Transform {
                x: (n % 8) as f32 * 8.,
                y: (n / 8) as f32 * 10.,
            },
        );
    }
    let mut gpu = GpuRenderer::new(96, 80).unwrap();
    scene.set_paint_transform(
        parent,
        Affine::rotation(0.15).then(Affine::translation(12., 12.)),
    );
    render(&mut gpu, &mut scene);
    scene.set_paint_transform(
        parent,
        Affine::rotation(0.2).then(Affine::translation(13., 12.)),
    );
    let report = scene.flush();
    assert_eq!(report.layout_nodes, 0);
    let stats = gpu.render(&scene, &report.damage).unwrap();
    assert_eq!(stats.paint_geometry_buffer_allocations, 0);
    assert!(
        stats.draw_calls <= 3,
        "same subtree geometry must batch: {stats:?}"
    );
    gpu.readback().unwrap();
}

#[test]
fn disjoint_damage_never_batches_across_a_skipped_affine_draw() {
    let mut scene = overlay();
    let a = scene.append(
        scene.root(),
        NodeKind::Rect(Color(180, 10, 20, 100)),
        fixed(10., 10.),
    );
    let b = scene.append(
        scene.root(),
        NodeKind::Rect(Color(10, 20, 240, 255)),
        fixed(2., 2.),
    );
    scene.set_paint_transform_origin(
        b,
        Affine::scale(10., 10.).then(Affine::translation(50., 0.)),
        [0., 0.],
    );
    scene.append(
        scene.root(),
        NodeKind::Rect(Color(10, 200, 30, 100)),
        fixed(10., 10.),
    );
    let mut gpu = GpuRenderer::new(96, 80).unwrap();
    gpu.set_background(Color(0, 0, 0, 0));
    render(&mut gpu, &mut scene);
    scene.set_kind(a, NodeKind::Rect(Color(200, 20, 30, 100)));
    scene.set_kind(b, NodeKind::Rect(Color(20, 30, 220, 255)));
    scene.flush();
    // B's normalized local vertices lie inside the first region, but its
    // scaled world vertices belong exclusively to the second region.
    gpu.render(
        &scene,
        &[Rect::new(0., 0., 12., 12.), Rect::new(48., 0., 24., 24.)],
    )
    .unwrap();
    same_pixels(
        &gpu.readback().unwrap(),
        &fresh(&gpu, &scene, BlurAlgorithm::Gaussian),
    );
}

#[test]
fn magnified_rounded_edges_and_zero_blur_shadows_keep_pixel_sized_coverage() {
    for shadow in [false, true] {
        let mut scene = overlay();
        let node = scene.append(
            scene.root(),
            NodeKind::Quad(QuadStyle {
                fill: if shadow {
                    Color(0, 0, 0, 0)
                } else {
                    Color(255, 255, 255, 255)
                },
                radius: 0.4,
                shadow: shadow.then_some(BoxShadow {
                    color: Color(255, 255, 255, 255),
                    offset: Transform::default(),
                    blur_radius: 0.,
                    spread: 1.,
                }),
                ..Default::default()
            }),
            fixed(4., 4.),
        );
        scene.set_paint_transform_origin(
            node,
            Affine::scale(10., 10.).then(Affine::translation(25.25, 20.25)),
            [0., 0.],
        );
        let mut gpu = GpuRenderer::new(96, 80).unwrap();
        gpu.set_background(Color(0, 0, 0, 0));
        let pixels = render(&mut gpu, &mut scene);
        // Sample the spread shadow's outer edge, outside the original box.
        let edge_x = if shadow { 15 } else { 25 };
        assert_eq!(
            pixel(&pixels, edge_x + 1, 40)[3],
            255,
            "shadow={shadow}: one pixel inside must be opaque"
        );
        assert!(
            pixel(&pixels, edge_x, 40)[3] >= 180,
            "shadow={shadow}: physical coverage must not scale to ten pixels"
        );
    }
}

#[test]
fn isolated_sampling_damage_matches_fresh_at_fractional_dpi() {
    for scale in [0.5_f32, 1., 1.5, 2.] {
        let mut scene = overlay();
        let layer = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            fixed(30., 20.),
        );
        scene.set_isolated(layer, true);
        scene.append(
            layer,
            NodeKind::Rect(Color(240, 30, 60, 255)),
            fixed(10., 20.),
        );
        let (w, h) = ((96. * scale) as u32, (80. * scale) as u32);
        let mut gpu = GpuRenderer::new(w, h).unwrap();
        gpu.set_scale_factor(scale);
        gpu.set_background(Color(0, 0, 0, 0));
        for angle in [0.3, -0.5, 0.7, -0.2, 0.] {
            scene.set_paint_transform_origin(
                layer,
                Affine::rotation(angle).then(Affine::translation(30.3, 20.2)),
                [0.5, 0.5],
            );
            let output = render(&mut gpu, &mut scene);
            let mut reference = GpuRenderer::new_with_context(w, h, &gpu.context()).unwrap();
            reference.set_scale_factor(scale);
            reference.set_background(Color(0, 0, 0, 0));
            reference
                .render(&scene, &[Rect::new(0., 0., 96., 80.)])
                .unwrap();
            same_pixels(&output, &reference.readback().unwrap());
        }
    }
}
