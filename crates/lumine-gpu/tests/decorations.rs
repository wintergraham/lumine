use std::sync::Arc;
use lumine::{canvas::GradientStop, decoration::*, scene::*};
use lumine_gpu::GpuRenderer;
fn style(angle: f32) -> QuadStyle {
    QuadStyle {
        fill: Color(20, 40, 80, 255),
        border_color: Color(255, 255, 255, 255),
        decoration: Some(Arc::new(Decoration {
            background: Some(Background::Linear {
                angle,
                stops: vec![
                    GradientStop {
                        offset: 0.,
                        color: Color(255, 0, 0, 255),
                    },
                    GradientStop {
                        offset: 1.,
                        color: Color(0, 0, 255, 255),
                    },
                ]
                .into(),
            }),
            corners: Some(Corners {
                top_left: 20.,
                top_right: 3.,
                bottom_right: 14.,
                bottom_left: 0.,
            }),
            border_widths: Some(Insets {
                left: 8.,
                right: 2.,
                top: 4.,
                bottom: 6.,
            }),
            shadows: Some(
                vec![
                    BoxShadow {
                        color: Color(0, 255, 0, 180),
                        offset: Transform { x: -12., y: 0. },
                        blur_radius: 2.,
                        spread: 1.,
                    },
                    BoxShadow {
                        color: Color(255, 0, 255, 180),
                        offset: Transform { x: 12., y: 0. },
                        blur_radius: 2.,
                        spread: 1.,
                    },
                ]
                .into(),
            ),
            ..Default::default()
        })),
        ..Default::default()
    }
}
#[test]
fn detailed_panels_cache_resize_and_restore_all_shadow_extents() {
    for isolated in [false, true] {
        let mut scene = Scene::new(160., 130.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let node = scene.append(
            scene.root(),
            NodeKind::Quad(style(0.)),
            Style {
                width: Some(80.),
                height: Some(70.),
                ..Default::default()
            },
        );
        scene.set_transform(node, Transform { x: 40., y: 30. });
        scene.set_isolated(node, isolated);
        let mut gpu = GpuRenderer::new(160, 130).unwrap();
        let report = scene.flush();
        let stats = gpu.render(&scene, &report.damage).unwrap();
        assert_eq!(stats.canvas_rasterizations, 1);
        let data = gpu.readback().unwrap();
        let pixel = |x: usize, y: usize| &data[(y * 160 + x) * 4..][..4];
        assert!(pixel(50, 65)[0] > 200);
        assert!(pixel(110, 65)[2] > 200);
        assert!(pixel(42, 70)[0] > 245 && pixel(42, 70)[1] > 245);
        assert!(pixel(30, 60)[1] > 50);
        assert!(pixel(128, 60)[0] > 50 && pixel(128, 60)[2] > 50);
        let stats = gpu
            .render(&scene, &[Rect::new(0., 0., 160., 130.)])
            .unwrap();
        assert_eq!(stats.canvas_rasterizations, 0);
        assert_eq!(stats.image_uploads, 0);
        let mut changed = style(1.);
        Arc::make_mut(changed.decoration.as_mut().unwrap()).border_style = BorderStyle::Dashed {
            length: 8.,
            gap: 5.,
        };
        scene.set_kind(node, NodeKind::Quad(changed));
        let report = scene.flush();
        assert_eq!(report.layout_nodes, 0);
        gpu.render(&scene, &report.damage).unwrap();
        let mut fresh = GpuRenderer::new(160, 130).unwrap();
        fresh
            .render(&scene, &[Rect::new(0., 0., 160., 130.)])
            .unwrap();
        assert_eq!(gpu.readback().unwrap(), fresh.readback().unwrap());
        scene.set_kind(
            node,
            NodeKind::Quad(QuadStyle {
                fill: Color(10, 20, 30, 255),
                ..Default::default()
            }),
        );
        let report = scene.flush();
        gpu.render(&scene, &report.damage).unwrap();
        let mut fresh = GpuRenderer::new(160, 130).unwrap();
        fresh
            .render(&scene, &[Rect::new(0., 0., 160., 130.)])
            .unwrap();
        assert_eq!(gpu.readback().unwrap(), fresh.readback().unwrap());
        assert_eq!(gpu.debug_cache_stats().canvas_raster_bytes, 0);
    }
}

#[test]
fn resizing_a_decorated_node_replaces_its_raster_instead_of_accumulating() {
    let mut gpu = GpuRenderer::new(320, 320).unwrap();
    let mut scene = Scene::new(320., 320.);
    let panel = scene.append(
        scene.root(),
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad: style(45.),
        },
        Style {
            width: Some(200.),
            height: Some(200.),
            ..Default::default()
        },
    );
    let d = scene.flush().damage;
    gpu.render(&scene, &d).unwrap();
    let one = gpu.debug_cache_stats().image_bytes;
    // A live window resize: one new size (and raster) per frame, no structural change.
    for step in 0..400 {
        scene.set_style(
            panel,
            Style {
                width: Some(200. + (step % 100) as f32),
                height: Some(200. + (step % 100) as f32),
                ..Default::default()
            },
        );
        let d = scene.flush().damage;
        gpu.render(&scene, &d).unwrap();
    }
    let stats = gpu.debug_cache_stats();
    assert_eq!(stats.image_textures, 1, "stale rasters stay resident");
    assert!(stats.image_bytes <= one * 3);
}

#[test]
fn flat_fill_with_per_corner_radii_draws_on_the_gpu_without_a_raster() {
    let mut gpu = GpuRenderer::new(100, 100).unwrap();
    let mut scene = Scene::new(100., 100.);
    let quad = QuadStyle {
        fill: Color(255, 0, 0, 255),
        decoration: Some(Arc::new(Decoration {
            corners: Some(Corners {
                top_left: 0.,
                top_right: 30.,
                bottom_right: 30.,
                bottom_left: 0.,
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    scene.append(
        scene.root(),
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad,
        },
        Style {
            width: Some(100.),
            height: Some(100.),
            ..Default::default()
        },
    );
    let d = scene.flush().damage;
    let stats = gpu.render(&scene, &d).unwrap();
    assert_eq!(stats.canvas_rasterizations, 0);
    assert_eq!(gpu.debug_cache_stats().image_textures, 0);
    let pixels = gpu.readback().unwrap();
    let at = |x: usize, y: usize| &pixels[(y * 100 + x) * 4..(y * 100 + x) * 4 + 4];
    assert_eq!(at(1, 1)[0], 255, "square top-left corner is filled");
    assert_eq!(at(1, 98)[0], 255, "square bottom-left corner is filled");
    assert_eq!(at(98, 1)[3], 0, "rounded top-right corner is cut");
    assert_eq!(at(98, 98)[3], 0, "rounded bottom-right corner is cut");
    assert_eq!(at(50, 50), &[255, 0, 0, 255]);
}

#[test]
fn rounded_overflow_clips_descendant_pixels_and_hit_targets() {
    use lumine::{compose::prelude::*, widgets::Ui};
    for scale in [1., 1.5, 2.] {
        let size = (96. * scale) as u32;
        let mut gpu = GpuRenderer::new(size, size).unwrap();
        gpu.set_scale_factor(scale);
        for isolated in [false, true] {
            let mut ui = Ui::new(96., 96.);
            let root = ui.mount(
                div().size(96., 96.).child(
                    div()
                        .absolute()
                        .left(8.)
                        .top(8.)
                        .size(64., 64.)
                        .rounded(12.)
                        .overflow_hidden()
                        .child(
                            div()
                                .id("child")
                                .absolute()
                                .size(80., 80.)
                                .bg(Color(255, 0, 0, 255))
                                .isolated(isolated),
                        ),
                ),
            );
            ui.prepare_frame();
            let child = root.find("child").unwrap();
            let mut scene = ui.scene.borrow_mut();
            let damage = scene.flush().damage;
            gpu.render(&scene, &damage).unwrap();
            let pixels = gpu.readback().unwrap();
            let at = |x: usize, y: usize| {
                &pixels[(y * size as usize + x) * 4..(y * size as usize + x) * 4 + 4]
            };
            let lo = (8. * scale) as usize;
            let hi = (72. * scale) as usize - 1;
            for (x, y) in [(lo, lo), (hi, lo), (lo, hi), (hi, hi)] {
                assert_eq!(
                    at(x, y)[3],
                    0,
                    "descendant leaked through corner at scale {scale}"
                );
            }
            let center = (40. * scale) as usize;
            assert_eq!(at(center, center), &[255, 0, 0, 255]);
            assert_eq!(
                at((80. * scale) as usize, center)[3],
                0,
                "oversized child escaped clip"
            );
            let edge = (12. * scale) as usize;
            assert!(
                (lo..lo + edge)
                    .any(|y| (lo..lo + edge).any(|x| at(x, y)[3] > 0 && at(x, y)[3] < 255)),
                "missing corner antialiasing at {scale}"
            );
            assert!(!scene.hit_test_all(8.5, 8.5).contains(&child));
            assert!(!scene.hit_test_all(80., 40.).contains(&child));
            assert!(scene.hit_test_all(40., 40.).contains(&child));
        }
    }
}

#[test]
fn caching_a_rounded_clip_preserves_corner_coverage() {
    use lumine::{compose::prelude::*, widgets::Ui};
    let mut gpu = GpuRenderer::new(64, 64).unwrap();
    let mut direct = None;
    for cached in [false, true] {
        let mut ui = Ui::new(64., 64.);
        ui.mount(
            div()
                .size(64., 64.)
                .rounded(12.)
                .overflow_hidden()
                .isolated(cached)
                .child(div().size(64., 64.).bg(Color(255, 0, 0, 255))),
        );
        ui.prepare_frame();
        let mut scene = ui.scene.borrow_mut();
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let pixels = gpu.readback().unwrap();
        if let Some(expected) = &direct {
            assert_eq!(
                &pixels, expected,
                "cached rounded root applied corner coverage twice"
            );
        } else {
            direct = Some(pixels);
        }
    }
}
