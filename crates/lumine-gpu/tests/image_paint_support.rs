//! Exact full-support A/Bs. Scissor checks use the production batching iterator.
use lumine::{
    affine::Affine,
    image::{EffectChain, EffectStage, ImageData, ImageSampling},
    scene::*,
};
use lumine_gpu::GpuRenderer;
use std::sync::Arc;

const W: f32 = 240.;
const H: f32 = 160.;
fn fixed(w: f32, h: f32) -> Style {
    Style {
        width: Some(w),
        height: Some(h),
        ..Default::default()
    }
}
fn fixture(center: (f32, f32), radius: f32) -> Arc<ImageData> {
    let (w, h) = (121_u32, 79_u32);
    let (cols, rows) = (45., 29.);
    let mut bytes = vec![0; (w * h * 4) as usize];
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            // Fractional cells stretched from a ceil-sized logical canvas.
            let cx = ((x as f32 + 0.5) * cols / w as f32).floor();
            let cy = ((y as f32 + 0.5) * rows / h as f32).floor();
            let dx = (cx + 0.5) / cols - center.0;
            let dy = (cy + 0.5) / rows - center.1;
            if dx.hypot(dy) < radius && (cx as u32 + cy as u32) % 3 != 0 {
                bytes[((y * w + x) * 4) as usize..((y * w + x + 1) * 4) as usize]
                    .copy_from_slice(&[190, 70, 210, 137]);
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    assert!(x1 > x0 && y1 > y0);
    Arc::new(
        ImageData::new(w, h, bytes)
            .unwrap()
            .sampled(ImageSampling::Nearest)
            .with_paint_support(Some(Rect::new(
                x0 as f32 / w as f32,
                y0 as f32 / h as f32,
                (x1 - x0) as f32 / w as f32,
                (y1 - y0) as f32 / h as f32,
            ))),
    )
}
fn tree(image: Arc<ImageData>, complex: bool) -> (Scene, NodeId) {
    let mut scene = Scene::new(W, H);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let outer = scene.append(
        scene.root(),
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad: QuadStyle {
                fill: Color(0, 0, 0, 0),
                radius: 13.,
                ..Default::default()
            },
        },
        Style {
            clip: complex,
            ..fixed(190., 130.)
        },
    );
    scene.set_transform(outer, Transform { x: 8.25, y: 7.5 });
    if complex {
        scene.set_paint_transform(
            outer,
            Affine::rotation(0.13).then(Affine {
                c: 0.22,
                ..Affine::IDENTITY
            }),
        );
    }
    let inner = scene.append(
        outer,
        NodeKind::Container(Layout::Overlay),
        Style {
            clip: complex,
            fade_edges: if complex { [23., 19.] } else { [0.; 2] },
            ..fixed(177.5, 119.25)
        },
    );
    let node = scene.append(inner, NodeKind::Image(image), fixed(151.25, 103.75));
    scene.set_transform(node, Transform { x: 0.5, y: 1.5 });
    scene.set_effects(
        node,
        Effects {
            opacity: 0.63,
            edge_fade: if complex { 8. } else { 0. },
            ..Default::default()
        },
    );
    (scene, node)
}
fn area(scissors: &[[u32; 4]]) -> u64 {
    scissors
        .iter()
        .map(|s| u64::from(s[2]) * u64::from(s[3]))
        .sum()
}
fn compare_pixels(actual: &[u8], full: &[u8]) {
    assert_eq!(actual.len(), full.len());
    let mismatch = actual
        .chunks_exact(4)
        .zip(full.chunks_exact(4))
        .enumerate()
        .find(|(_, (a, b))| a != b);
    assert!(mismatch.is_none(), "first pixel mismatch: {mismatch:?}");
}

#[test]
fn support_scissors_preserve_nearest_ties_affines_nested_rounded_clips_and_fades() {
    for scale in [1., 2.] {
        let mut gpu = GpuRenderer::new((W * scale) as u32, (H * scale) as u32).unwrap();
        let mut full =
            GpuRenderer::new_with_context((W * scale) as u32, (H * scale) as u32, &gpu.context())
                .unwrap();
        for renderer in [&mut gpu, &mut full] {
            renderer.set_scale_factor(scale);
            renderer.set_background(Color(17, 23, 31, 255));
            renderer.debug_enable_single_pass();
        }
        for complex in [false, true] {
            for own in [
                Affine::IDENTITY,
                Affine::rotation(0.37).then(Affine {
                    b: -0.18,
                    c: 0.24,
                    ..Affine::IDENTITY
                }),
            ] {
                let image = Arc::new(fixture((0.39, 0.52), 0.17).transformed(own));
                let baseline = Arc::new(image.with_paint_support(None));
                let (mut scene, _) = tree(image.clone(), complex);
                let (mut reference, _) = tree(baseline, complex);
                scene.flush();
                reference.flush();
                let damage = [Rect::new(0., 0., W, H)];
                gpu.render(&scene, &damage).unwrap();
                full.render(&reference, &damage).unwrap();
                let reduced = gpu.debug_pending_image_scissors(image.id());
                let original = full.debug_pending_image_scissors(image.id());
                assert!(
                    area(&reduced) > 0 && area(&reduced) * 2 < area(&original),
                    "scale={scale}, complex={complex}, own={own:?}: {reduced:?} vs {original:?}"
                );
                compare_pixels(
                    &gpu.debug_present_offscreen().unwrap(),
                    &full.debug_present_offscreen().unwrap(),
                );
            }
        }
    }
}

#[test]
fn support_metadata_invalidates_prepared_geometry_without_upload_or_layout() {
    let image = fixture((0.45, 0.5), 0.16);
    let (mut scene, node) = tree(Arc::new(image.with_paint_support(None)), false);
    let mut gpu = GpuRenderer::new(W as u32, H as u32).unwrap();
    gpu.set_background(Color(17, 23, 31, 255));
    gpu.debug_enable_single_pass();
    let damage = scene.flush().damage;
    gpu.render(&scene, &damage).unwrap();
    let before = area(&gpu.debug_pending_image_scissors(image.id()));
    let original = gpu.debug_present_offscreen().unwrap();
    scene.set_kind(node, NodeKind::Image(image.clone()));
    let report = scene.flush();
    assert_eq!(report.layout_nodes, 0);
    assert!(!report.damage.is_empty());
    let stats = gpu.render(&scene, &report.damage).unwrap();
    assert_eq!(stats.image_uploads, 0);
    assert!(area(&gpu.debug_pending_image_scissors(image.id())) < before);
    compare_pixels(&gpu.debug_present_offscreen().unwrap(), &original);
}

#[test]
fn moving_resizing_and_replacing_support_clear_old_pixels_with_partial_damage() {
    for scale in [1., 2.] {
        let mut gpu = GpuRenderer::new((W * scale) as u32, (H * scale) as u32).unwrap();
        let mut full =
            GpuRenderer::new_with_context((W * scale) as u32, (H * scale) as u32, &gpu.context())
                .unwrap();
        gpu.set_scale_factor(scale);
        full.set_scale_factor(scale);
        let first = fixture((0.2, 0.45), 0.14);
        let (mut scene, node) = tree(first.clone(), false);
        let (mut reference, ref_node) = tree(Arc::new(first.with_paint_support(None)), false);
        for step in 0..4 {
            if step > 0 {
                let image = fixture((0.2 + step as f32 * 0.19, 0.45), 0.14 - step as f32 * 0.02);
                scene.set_kind(node, NodeKind::Image(image.clone()));
                reference.set_kind(
                    ref_node,
                    NodeKind::Image(Arc::new(image.with_paint_support(None))),
                );
                for (tree, id) in [(&mut scene, node), (&mut reference, ref_node)] {
                    tree.set_style(
                        id,
                        fixed(151.25 - step as f32 * 11.5, 103.75 - step as f32 * 7.25),
                    );
                    tree.set_transform(
                        id,
                        Transform {
                            x: -8.5 + step as f32 * 4.5,
                            y: 1.5,
                        },
                    );
                }
            }
            let report = scene.flush();
            reference.flush();
            if step > 0 {
                assert!(!report.damage.is_empty());
                assert!(report.damage.iter().all(|r| r.width * r.height < W * H));
            }
            gpu.render(&scene, &report.damage).unwrap();
            full.render(&reference, &[Rect::new(0., 0., W, H)]).unwrap();
            compare_pixels(&gpu.readback().unwrap(), &full.readback().unwrap());
        }
    }
}

#[test]
fn linear_filters_and_backdrop_blur_keep_full_draw_and_filter_regions() {
    let nearest = fixture((0.48, 0.5), 0.14);
    let filtered = EffectChain::new()
        .render(nearest.clone(), &[EffectStage::Blur { radius: 3. }])
        .unwrap();
    assert_eq!(filtered.paint_support(), None);
    let variants = [
        Arc::new(nearest.sampled(ImageSampling::Linear)),
        filtered,
        nearest.clone(),
    ];
    let mut gpu = GpuRenderer::new(W as u32, H as u32).unwrap();
    let mut full = GpuRenderer::new_with_context(W as u32, H as u32, &gpu.context()).unwrap();
    gpu.set_background(Color(17, 23, 31, 255));
    gpu.debug_enable_single_pass();
    full.set_background(Color(17, 23, 31, 255));
    full.debug_enable_single_pass();
    for (index, image) in variants.into_iter().enumerate() {
        let (mut scene, node) = tree(image.clone(), true);
        let (mut reference, ref_node) = tree(Arc::new(image.with_paint_support(None)), true);
        if index == 2 {
            for (tree, id) in [(&mut scene, node), (&mut reference, ref_node)] {
                tree.set_effects(
                    id,
                    Effects {
                        blur_radius: 4.,
                        ..Default::default()
                    },
                );
                let bg = tree.append(
                    tree.root(),
                    NodeKind::Rect(Color(210, 80, 30, 255)),
                    Style {
                        z_index: -1,
                        ..fixed(90., 120.)
                    },
                );
                tree.set_transform(bg, Transform { x: 30., y: 5. });
            }
        }
        scene.flush();
        reference.flush();
        let damage = [Rect::new(0., 0., W, H)];
        let optimized = gpu.render(&scene, &damage).unwrap();
        let baseline = full.render(&reference, &damage).unwrap();
        assert_eq!(optimized.filtered_pixels, baseline.filtered_pixels);
        if index == 2 {
            assert!(optimized.filtered_pixels > 0);
        }
        assert_eq!(
            gpu.debug_pending_image_scissors(image.id()),
            full.debug_pending_image_scissors(image.id())
        );
        compare_pixels(
            &gpu.debug_present_offscreen().unwrap(),
            &full.debug_present_offscreen().unwrap(),
        );
    }
}
