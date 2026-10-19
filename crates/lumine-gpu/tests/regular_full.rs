//! Exact original/specialized shader comparison, including retained geometry.
use lumine::{
    affine::Affine,
    image::{ImageData, ImageSampling},
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
                255, 192, 255, 20, 255, 255,
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
    let mut specialized = GpuRenderer::new_with_context(320, 240, &original.context()).unwrap();
    for gpu in [&mut original, &mut specialized] {
        gpu.set_scale_factor(2.);
        gpu.set_background(Color(12, 24, 40, 255));
        // Keep this experiment independent of opaque-interior decomposition.
        gpu.debug_opaque_interiors(false);
    }
    original.debug_split_shading(true);
    original.debug_regular_full_shader(false);
    specialized.debug_split_shading(true);
    let damage = [Rect::new(0., 0., 160., 120.)];
    for single_pass in [false, true] {
        if single_pass {
            original.debug_enable_single_pass();
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
            original.render(&scene, &damage).unwrap();
            specialized.render(&scene, &damage).unwrap();
            // Present before readback, which would otherwise flush deferred draws.
            let expected_presented = original.debug_present_offscreen().unwrap();
            let actual_presented = specialized.debug_present_offscreen().unwrap();
            let expected = original.readback().unwrap();
            let actual = specialized.readback().unwrap();
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
