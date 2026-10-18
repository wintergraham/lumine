//! Verify actual shader pixels against Gaussian coverage and pen.dev exports.
use lumine::scene::{BoxShadow, Color, Layout, NodeKind, QuadStyle, Scene, Style, Transform};
use lumine_gpu::GpuRenderer;

#[test]
fn shadows_have_gaussian_edges_negative_spread_and_ancestor_clipping() {
    let mut scene = Scene::new(160., 140.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let parent = scene.append(
        scene.root(),
        NodeKind::Container(Layout::Overlay),
        Style {
            width: Some(150.),
            height: Some(140.),
            clip: true,
            ..Default::default()
        },
    );
    let node = scene.append(
        parent,
        NodeKind::Quad(QuadStyle {
            fill: Color(0, 0, 0, 0),
            radius: 0.,
            shadow: Some(BoxShadow {
                color: Color(0, 0, 0, 255),
                offset: Transform::default(),
                blur_radius: 5.,
                spread: -2.,
            }),
            ..Default::default()
        }),
        Style {
            width: Some(80.),
            height: Some(60.),
            clip: true,
            ..Default::default()
        },
    );
    scene.set_transform(node, Transform { x: 40., y: 40. });
    let mut renderer = GpuRenderer::new(160, 140).unwrap();
    let report = scene.flush();
    renderer.render(&scene, &report.damage).unwrap();
    let pixels = renderer.readback().unwrap();
    let alpha = |x: usize, y: usize| pixels[(y * 160 + x) * 4 + 3] as i32;
    // Shadow core starts at x=42 after negative spread. Pixel centers are x+.5.
    for (x, expected) in [(31, 5), (36, 35), (39, 79)] {
        assert!(
            (alpha(x, 70) - expected).abs() <= 1,
            "x={x}: {} != {expected}",
            alpha(x, 70)
        );
    }
    // Own clipping must not chop the blurred outer shadow at x=40.
    assert!(alpha(36, 70) > 0);
    for x in [40, 41, 42, 47, 52, 80, 119] {
        assert_eq!(alpha(x, 70), 0, "outer shadow entered the border box");
    }
    assert_eq!(alpha(151, 70), 0);
}

#[test]
fn outer_shadows_preserve_panel_interiors_and_rounded_cutouts() {
    use lumine::{
        affine::Affine,
        decoration::{Corners, Decoration},
    };
    use std::sync::Arc;

    let mut gpu = GpuRenderer::new(128, 128).unwrap();
    let pixel = |pixels: &[u8], x: usize, y: usize| -> [u8; 4] {
        pixels[(y * 128 + x) * 4..(y * 128 + x + 1) * 4]
            .try_into()
            .unwrap()
    };
    // Cover the legacy solid, per-corner solid and rasterized decoration paths.
    for mode in 0..3 {
        for split in [false, true] {
            gpu.debug_split_shading(split);
            for alpha in [0, 96, 255] {
                let mut scene = Scene::new(128., 128.);
                scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
                gpu.set_background(Color(20, 40, 70, 255));
                scene.append(
                    scene.root(),
                    NodeKind::Rect(Color(60, 90, 30, 255)),
                    Style {
                        width: Some(64.),
                        height: Some(128.),
                        ..Default::default()
                    },
                );
                let clipped = mode == 2;
                let parent = scene.append(
                    scene.root(),
                    NodeKind::Panel {
                        layout: Layout::Overlay,
                        quad: QuadStyle {
                            radius: 18.,
                            ..Default::default()
                        },
                    },
                    Style {
                        width: Some(96.),
                        height: Some(80.),
                        clip: clipped,
                        fade_edges: if clipped { [14., 10.] } else { [0., 0.] },
                        ..Default::default()
                    },
                );
                scene.set_transform(parent, Transform { x: 16., y: 20. });
                if clipped {
                    scene.set_paint_transform_origin(parent, Affine::rotation(0.15), [0.5, 0.5]);
                }
                let mut style = QuadStyle {
                    fill: Color(130, 60, 90, alpha),
                    border_color: Color(80, 150, 180, 255),
                    border_width: if clipped { 1. } else { 0. },
                    decoration: (mode > 0).then(|| {
                        Arc::new(Decoration {
                            corners: Some(Corners {
                                top_left: 0.,
                                top_right: 18.,
                                bottom_right: 10.,
                                bottom_left: 6.,
                            }),
                            ..Default::default()
                        })
                    }),
                    ..Default::default()
                };
                let panel = scene.append(
                    parent,
                    NodeKind::Quad(style.clone()),
                    Style {
                        width: Some(64.),
                        height: Some(48.),
                        ..Default::default()
                    },
                );
                scene.set_transform(panel, Transform { x: 4., y: 6. });
                let damage = scene.flush().damage;
                gpu.render(&scene, &damage).unwrap();
                let without = gpu.readback().unwrap();
                style.shadow = Some(BoxShadow {
                    color: Color(255, 230, 80, 200),
                    offset: Transform { x: 5., y: -3. },
                    blur_radius: 3.,
                    spread: 8.,
                });
                scene.set_kind(panel, NodeKind::Quad(style));
                let damage = scene.flush().damage;
                gpu.render(&scene, &damage).unwrap();
                let with = gpu.readback().unwrap();
                let sample = |pixels: &[u8], x: f32, y: f32| {
                    let (x, y) = scene.local_to_world(panel, x, y);
                    pixel(pixels, x.floor() as usize, y.floor() as usize)
                };
                for (x, y) in [(32., 24.), (12., 20.), (52., 28.)] {
                    assert_eq!(
                        sample(&with, x, y),
                        sample(&without, x, y),
                        "shadow changed interior: mode={mode} split={split} alpha={alpha} ({x},{y})"
                    );
                }
                if alpha == 0 {
                    assert_eq!(
                        sample(&with, 32., 24.),
                        [60, 90, 30, 255],
                        "transparent panel must retain the underlying rectangle"
                    );
                } else if alpha == 255 {
                    assert_eq!(
                        sample(&with, 32., 24.),
                        [130, 60, 90, 255],
                        "opaque panel fill must stay unchanged"
                    );
                }
                assert_ne!(
                    sample(&with, 70., 24.),
                    sample(&without, 70., 24.),
                    "outer ring disappeared: mode={mode} split={split} alpha={alpha}"
                );
                if mode > 0 && alpha == 0 {
                    assert_eq!(
                        sample(&with, 2., 2.),
                        sample(&without, 2., 2.),
                        "square top-left interior must be knocked out"
                    );
                    assert_ne!(
                        sample(&with, 62., 2.),
                        sample(&without, 62., 2.),
                        "rounded top-right cutout must retain the shadow"
                    );
                }
                if clipped {
                    assert_eq!(
                        pixel(&with, 16, 20),
                        pixel(&without, 16, 20),
                        "shadow escaped the transformed rounded ancestor clip"
                    );
                }
            }
        }
    }
}

#[test]
fn pen_omnibox_two_shadow_alpha_profile_matches_export() {
    // Keep the independent Pen export with the test; missing artifacts must not
    // turn a fidelity check into a silently passing no-op.
    let reference = image::load_from_memory(include_bytes!("fixtures/omnibox-shadow.png"))
        .unwrap()
        .to_rgba8();
    let (w, h) = reference.dimensions();
    let x = w / 2;
    let top = (0..h)
        .find(|y| reference.get_pixel(x, *y)[3] == 255)
        .unwrap();
    let mut scene = Scene::new(w as f32, h as f32);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let mut quad = QuadStyle {
        fill: Color(26, 26, 24, 255),
        radius: 12.,
        ..Default::default()
    };
    quad.decoration = Some(std::sync::Arc::new(lumine::decoration::Decoration {
        shadows: Some(
            vec![
                BoxShadow {
                    color: Color(0, 0, 0, 128),
                    offset: Transform { x: 0., y: 16. },
                    blur_radius: 20.,
                    spread: 0.,
                },
                BoxShadow {
                    color: Color(0, 0, 0, 77),
                    offset: Transform { x: 0., y: 2. },
                    blur_radius: 3.,
                    spread: 0.,
                },
            ]
            .into(),
        ),
        ..Default::default()
    }));
    let node = scene.append(
        scene.root(),
        NodeKind::Quad(quad),
        Style {
            width: Some(680.),
            height: Some(348.),
            ..Default::default()
        },
    );
    scene.set_transform(
        node,
        Transform {
            x: 60.,
            y: top as f32,
        },
    );
    let mut renderer = GpuRenderer::new(w, h).unwrap();
    let report = scene.flush();
    renderer.render(&scene, &report.damage).unwrap();
    let pixels = renderer.readback().unwrap();
    let rmse = ((0..top)
        .map(|y| {
            let got = pixels[((y * w + x) * 4 + 3) as usize] as f32;
            (got - reference.get_pixel(x, y)[3] as f32).powi(2)
        })
        .sum::<f32>()
        / top as f32)
        .sqrt();
    println!("pen.dev top shadow alpha RMSE: {rmse:.3}/255");
    assert!(rmse < 2., "pen.dev alpha profile RMSE {rmse}");
    if let Ok(output) = std::env::var("SHADOW_FIDELITY_PNG") {
        image::save_buffer(output, &pixels, w, h, image::ColorType::Rgba8).unwrap();
    }
}
