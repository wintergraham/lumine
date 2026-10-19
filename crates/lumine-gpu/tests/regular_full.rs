//! Exact original/specialized shader comparison, including retained geometry.
use lumine::{
    affine::Affine,
    decoration::{Corners, Decoration},
    image::{EffectChain, EffectStage, ImageData, ImageSampling},
    layout::LayoutOptions,
    scene::*,
};
use lumine_gpu::GpuRenderer;
use std::sync::Arc;

fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}

#[test]
fn regular_full_matches_original_with_nested_clips_nearest_masks_and_mixed_shadows() {
    let mut scene = Scene::new(160., 120.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let panel = |radius| NodeKind::Panel {
        layout: Layout::Overlay,
        quad: QuadStyle {
            radius,
            ..Default::default()
        },
    };
    let outer = scene.append(
        scene.root(),
        panel(19.),
        Style {
            clip: true,
            fade_edges: [12., 9.],
            ..fixed(112., 92.)
        },
    );
    scene.set_paint_transform(
        outer,
        Affine::rotation(0.12).then(Affine::translation(20.5, 8.5)),
    );
    let inner = scene.append(
        outer,
        panel(13.),
        Style {
            clip: true,
            ..fixed(92., 78.)
        },
    );
    scene.set_transform(inner, Transform { x: 5.5, y: 4.5 });
    let horizontal = scene.append(
        inner,
        NodeKind::Container(Layout::Overlay),
        Style {
            fade_edges: [-11., -15.],
            ..fixed(92., 78.)
        },
    );
    let image = Arc::new(
        ImageData::new(
            3,
            2,
            vec![
                255, 40, 20, 255, 20, 255, 40, 128, 40, 20, 255, 255, 255, 255, 20, 255, 20, 255,
                255, 1, 255, 20, 255, 0,
            ],
        )
        .unwrap()
        .sampled(ImageSampling::Nearest)
        .transformed(Affine::rotation(0.18)),
    );
    let nearest = scene.append(horizontal, NodeKind::Image(image), fixed(104., 84.));
    scene.set_transform(nearest, Transform { x: -6.5, y: -4.5 });
    scene.set_effects(
        nearest,
        Effects {
            edge_fade: 6.,
            opacity: 0.83,
            ..Default::default()
        },
    );
    // This retained draw contains a Gaussian shadow and its ordinary fill.
    let mixed_style = QuadStyle {
        fill: Color(100, 60, 170, 130),
        radius: 9.,
        border_width: 1.5,
        border_color: Color(255, 190, 80, 210),
        shadow: Some(BoxShadow {
            color: Color(240, 40, 20, 200),
            offset: Transform { x: 4., y: 3. },
            blur_radius: 3.,
            spread: 1.,
        }),
        ..Default::default()
    };
    let mixed = scene.append(inner, NodeKind::Quad(mixed_style.clone()), fixed(38., 27.));
    scene.set_transform(mixed, Transform { x: 25., y: 30. });
    let label = scene.append(
        inner,
        NodeKind::Text {
            text: "Full shader".into(),
            font_size: 11.,
            color: Color(240, 245, 250, 220),
        },
        fixed(80., 18.),
    );
    scene.set_transform(label, Transform { x: 7., y: 8. });
    // The same shadow/fill pair without retained geometry exercises ordered runs.
    let ordinary = scene.append(
        scene.root(),
        NodeKind::Quad(mixed_style.clone()),
        fixed(25., 22.),
    );
    scene.set_transform(ordinary, Transform { x: 129., y: 89. });
    let sharp = scene.append(
        scene.root(),
        NodeKind::Quad(QuadStyle {
            shadow: Some(BoxShadow {
                blur_radius: 0.,
                ..mixed_style.shadow.unwrap()
            }),
            ..mixed_style.clone()
        }),
        fixed(18., 15.),
    );
    scene.set_transform(sharp, Transform { x: 134., y: 13. });
    scene.flush();
    let items = scene.layer_items(None);
    let item = items.iter().find(|item| item.id == nearest).unwrap();
    assert!(
        item.clip_regions
            .iter()
            .filter(|clip| clip.corners.iter().any(|r| *r > 0.))
            .count()
            >= 2,
        "fixture must exercise two actual rounded shader clips"
    );
    assert!(
        item.mask.is_some(),
        "fixture must exercise an ancestor fade mask"
    );
    drop(items);

    let mut original = GpuRenderer::new(320, 240).unwrap();
    let mut regular = GpuRenderer::new_with_context(320, 240, &original.context()).unwrap();
    let mut specialized = GpuRenderer::new_with_context(320, 240, &original.context()).unwrap();
    for gpu in [&mut original, &mut regular, &mut specialized] {
        gpu.set_scale_factor(2.);
        gpu.set_background(Color(12, 24, 40, 255));
        // Keep this experiment independent of opaque-interior decomposition.
        gpu.debug_opaque_interiors(false);
    }
    original.debug_split_shading(true);
    original.debug_regular_full_shader(false);
    regular.debug_split_shading(true);
    regular.debug_nearest_image_shader(false);
    regular.debug_clip_interiors(true);
    specialized.debug_split_shading(true);
    specialized.debug_clip_interiors(true);
    let damage = [Rect::new(0., 0., 160., 120.)];
    for single_pass in [false, true] {
        if single_pass {
            original.debug_enable_single_pass();
            regular.debug_enable_single_pass();
            specialized.debug_enable_single_pass();
        }
        let mut with_shadow = None;
        for shadows in [true, false] {
            let style = if shadows {
                mixed_style.clone()
            } else {
                QuadStyle {
                    shadow: None,
                    ..mixed_style.clone()
                }
            };
            scene.set_kind(mixed, NodeKind::Quad(style.clone()));
            scene.set_kind(ordinary, NodeKind::Quad(style));
            scene.flush();
            let original_stats = original.render(&scene, &damage).unwrap();
            let regular_stats = regular.render(&scene, &damage).unwrap();
            let specialized_stats = specialized.render(&scene, &damage).unwrap();
            assert_eq!(original_stats.draw_calls, specialized_stats.draw_calls);
            assert_eq!(regular_stats.draw_calls, specialized_stats.draw_calls);
            // Present before readback, which would otherwise flush deferred draws.
            let expected_presented = original.debug_present_offscreen().unwrap();
            let regular_presented = regular.debug_present_offscreen().unwrap();
            let actual_presented = specialized.debug_present_offscreen().unwrap();
            let expected = original.readback().unwrap();
            let regular_pixels = regular.readback().unwrap();
            let actual = specialized.readback().unwrap();
            assert_eq!(regular_pixels, expected, "RegularFull reference pixels");
            assert_eq!(
                regular_presented, expected_presented,
                "RegularFull presentation"
            );
            let mismatch = actual
                .chunks_exact(4)
                .zip(expected.chunks_exact(4))
                .enumerate()
                .find(|(_, (a, b))| a != b);
            assert!(
                mismatch.is_none(),
                "single_pass={single_pass}, shadows={shadows}, first pixel mismatch={mismatch:?}"
            );
            assert_eq!(
                actual_presented, expected_presented,
                "presented pixels: single_pass={single_pass}, shadows={shadows}"
            );
            if single_pass {
                assert!(
                    specialized.debug_single_pass_frames() > 0,
                    "fixture must exercise the dual-output specialization"
                );
            }
            if shadows {
                with_shadow = Some(actual);
            } else {
                assert_ne!(
                    with_shadow.as_ref().unwrap(),
                    &actual,
                    "Gaussian shadows must contribute visible pixels to the comparison"
                );
            }
        }
    }
}

fn boundary_image() -> ImageData {
    let mut pixels = Vec::new();
    for y in 0..13u8 {
        for x in 0..13u8 {
            let alpha = [0, 1, 127, 255][(x as usize + y as usize) % 4];
            pixels.extend([x * 17, y * 17, 255 - x * 17, alpha]);
        }
    }
    ImageData::new(13, 13, pixels).unwrap()
}

#[test]
fn nearest_image_specialization_preserves_boundary_ties_alpha_and_affine_sampling() {
    // Each source texel lands on a physical pixel boundary at DPR 2. The
    // half-pixel origins expose interpolated UV ties, including the last cell.
    let data = boundary_image();
    let mut reference = GpuRenderer::new(80, 80).unwrap();
    let mut specialized = GpuRenderer::new_with_context(80, 80, &reference.context()).unwrap();
    for gpu in [&mut reference, &mut specialized] {
        gpu.set_scale_factor(2.);
        gpu.set_background(Color(31, 47, 67, 255));
        gpu.debug_split_shading(true);
        gpu.debug_opaque_interiors(false);
    }
    reference.debug_nearest_image_shader(false);
    let mut scene = Scene::new(40., 40.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let node = scene.append(
        scene.root(),
        NodeKind::Image(Arc::new(data.sampled(ImageSampling::Nearest))),
        fixed(6.5, 6.5),
    );
    scene.set_effects(
        node,
        Effects {
            opacity: 0.73,
            ..Default::default()
        },
    );
    for single_pass in [false, true] {
        if single_pass {
            reference.debug_enable_single_pass();
            specialized.debug_enable_single_pass();
        }
        for matrix in [
            Affine::IDENTITY,
            Affine::scale(-1., 1.),
            Affine::rotation(0.31),
            Affine {
                c: 0.4,
                ..Affine::IDENTITY
            },
        ] {
            // Switching to linear on the same texture must leave the nearest
            // pipeline and keep the original filtered sampling semantics.
            for sampling in [ImageSampling::Nearest, ImageSampling::Linear] {
                scene.set_kind(
                    node,
                    NodeKind::Image(Arc::new(data.sampled(sampling).transformed(matrix))),
                );
                for origin in [0.25, 1.75, 7.25] {
                    scene.set_transform(
                        node,
                        Transform {
                            x: origin,
                            y: origin,
                        },
                    );
                    scene.flush();
                    let damage = [Rect::new(0., 0., 40., 40.)];
                    let expected_stats = reference.render(&scene, &damage).unwrap();
                    let actual_stats = specialized.render(&scene, &damage).unwrap();
                    assert_eq!(actual_stats.draw_calls, expected_stats.draw_calls);
                    let expected_presented = reference.debug_present_offscreen().unwrap();
                    let actual_presented = specialized.debug_present_offscreen().unwrap();
                    assert_eq!(
                        specialized.readback().unwrap(),
                        reference.readback().unwrap(),
                        "matrix={matrix:?}, sampling={sampling:?}, origin={origin}, single_pass={single_pass}"
                    );
                    assert_eq!(actual_presented, expected_presented);
                }
            }
        }
    }
}

#[test]
fn clip_interiors_match_reference_at_aa_ties_fractional_dpr_and_affine_clips() {
    let mut chain = EffectChain::new();
    let image = chain
        .render(
            Arc::new(boundary_image().sampled(ImageSampling::Nearest)),
            &[EffectStage::Dither {
                levels: 4,
                cell_size: 1,
            }],
        )
        .unwrap();
    let mut reference = GpuRenderer::new(320, 240).unwrap();
    let mut optimized = GpuRenderer::new_with_context(320, 240, &reference.context()).unwrap();
    for gpu in [&mut reference, &mut optimized] {
        gpu.set_background(Color(0, 0, 0, 0));
        gpu.debug_split_shading(true);
        gpu.debug_opaque_interiors(false);
    }
    optimized.debug_clip_interiors(true);
    let panel = |r: [f32; 4]| NodeKind::Panel {
        layout: Layout::Overlay,
        quad: QuadStyle {
            decoration: Some(Arc::new(Decoration {
                corners: Some(Corners {
                    top_left: r[0],
                    top_right: r[1],
                    bottom_right: r[2],
                    bottom_left: r[3],
                }),
                ..Default::default()
            })),
            ..Default::default()
        },
    };
    let shear = Affine {
        c: 0.4,
        ..Affine::IDENTITY
    };
    // The first three put edge=-AA exactly on a pixel center or on either side
    // at DPR 2. The remaining transforms reuse the affine-image fixtures.
    let cases = [
        (Affine::IDENTITY, [4.; 4], [true, true], false, 0.),
        (Affine::IDENTITY, [4.; 4], [true, true], false, 0.25),
        (Affine::IDENTITY, [4.; 4], [true, true], false, 0.5),
        (
            Affine::rotation(0.31),
            [0., 23., 4., 32.],
            [true, true],
            true,
            0.25,
        ),
        (
            Affine::scale(1.2, 0.7).then(Affine::rotation(0.7)),
            [999.; 4],
            [true, true],
            false,
            0.25,
        ),
        (shear, [0., 20., 999., 3.], [true, true], true, 0.25),
        (
            Affine::scale(-1., 1.),
            [2., 18., 7., 0.],
            [true, true],
            true,
            0.25,
        ),
        (shear, [0.; 4], [true, false], false, 0.25),
        (Affine::rotation(0.31), [0.; 4], [false, true], false, 0.25),
        (shear, [0.; 4], [true, true], false, 0.25),
    ];
    let mut saw_source_dispatch = false;
    for single_pass in [false, true] {
        if single_pass {
            reference.debug_enable_single_pass();
            optimized.debug_enable_single_pass();
        }
        for scale in [1.25, 2.] {
            for gpu in [&mut reference, &mut optimized] {
                gpu.set_scale_factor(scale);
                gpu.resize((160. * scale) as u32, (120. * scale) as u32);
            }
            for (case, (matrix, corners, axes, nested, origin)) in cases.into_iter().enumerate() {
                let mut scene = Scene::new(160., 120.);
                scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
                let parent = scene.append(
                    scene.root(),
                    panel(corners),
                    Style {
                        clip: true,
                        layout_options: Some(Arc::new(LayoutOptions {
                            clip_x: Some(axes[0]),
                            clip_y: Some(axes[1]),
                            ..Default::default()
                        })),
                        ..fixed(96., 72.)
                    },
                );
                let x = if matrix.a < 0. { 125. } else { 26. };
                scene.set_paint_transform(
                    parent,
                    matrix.then(Affine::translation(x + origin, 18. + origin)),
                );
                let container = if nested {
                    let inner = scene.append(
                        parent,
                        panel([3., 0., 17., 8.]),
                        Style {
                            clip: true,
                            ..fixed(76., 56.)
                        },
                    );
                    scene.set_transform(inner, Transform { x: 10.25, y: 8.25 });
                    inner
                } else {
                    parent
                };
                let node =
                    scene.append(container, NodeKind::Image(image.clone()), fixed(120., 96.));
                scene.set_transform(node, Transform { x: -12., y: -12. });
                scene.set_effects(
                    node,
                    Effects {
                        opacity: 0.73,
                        edge_fade: 6.,
                        ..Default::default()
                    },
                );
                scene.flush();
                let items = scene.layer_items(None);
                let item = items.iter().find(|item| item.id == node).unwrap();
                assert!(item.clip_regions.iter().any(|clip| clip.axes == axes));
                if nested {
                    assert!(item.clip_regions.len() >= 2);
                }
                if corners.iter().any(|r| *r > 0.) {
                    assert!(
                        item.clip_regions
                            .iter()
                            .any(|clip| clip.corners.iter().any(|r| *r > 0.))
                    );
                } else {
                    assert!(item.clip_regions.iter().all(|clip| clip.corners == [0.; 4]));
                }
                drop(items);
                // Test each pipeline override independently with unchanged batches.
                for (regular, nearest) in [(false, false), (true, false), (true, true)] {
                    for gpu in [&mut reference, &mut optimized] {
                        gpu.debug_regular_full_shader(regular);
                        gpu.debug_nearest_image_shader(nearest);
                    }
                    let damage = [Rect::new(0., 0., 160., 120.)];
                    let expected_stats = reference.render(&scene, &damage).unwrap();
                    let actual_stats = optimized.render(&scene, &damage).unwrap();
                    assert_eq!(actual_stats.draw_calls, expected_stats.draw_calls);
                    assert_eq!(actual_stats.instances, expected_stats.instances);
                    assert_eq!(
                        actual_stats.shader_dispatches,
                        expected_stats.shader_dispatches
                    );
                    saw_source_dispatch |= actual_stats.shader_dispatches > 0;
                    let expected_presented = reference.debug_present_offscreen().unwrap();
                    let actual_presented = optimized.debug_present_offscreen().unwrap();
                    let expected = reference.readback().unwrap();
                    let actual = optimized.readback().unwrap();
                    assert!(
                        expected
                            .chunks_exact(4)
                            .any(|pixel| pixel[3] > 0 && pixel[3] < 255),
                        "fixture must paint translucent pixels"
                    );
                    let mismatch = actual
                        .chunks_exact(4)
                        .zip(expected.chunks_exact(4))
                        .enumerate()
                        .find(|(_, (a, b))| a != b);
                    assert!(
                        mismatch.is_none(),
                        "case={case}, scale={scale}, single_pass={single_pass}, regular={regular}, nearest={nearest}, first RGBA mismatch={mismatch:?}"
                    );
                    assert_eq!(
                        actual_presented, expected_presented,
                        "presentation: case={case}, scale={scale}, single_pass={single_pass}, regular={regular}, nearest={nearest}"
                    );
                }
            }
        }
    }
    assert!(
        saw_source_dispatch,
        "fixture must run the Dither source effect"
    );
    assert!(optimized.debug_single_pass_frames() > 0);
}
