use std::sync::Arc;
use lumine::{
    affine::Affine,
    image::{ImageData, ImageSampling},
    scene::*,
};
use lumine_gpu::GpuRenderer;

fn fixed(w: f32, h: f32) -> Style {
    Style {
        width: Some(w),
        height: Some(h),
        ..Default::default()
    }
}
fn pixel(bytes: &[u8], x: usize, y: usize) -> &[u8] {
    &bytes[(y * 120 + x) * 4..(y * 120 + x) * 4 + 4]
}

#[test]
fn nearest_image_sampling_preserves_cells_and_reuses_upload_at_fractional_origin() {
    let data = Arc::new(ImageData::new(2, 1, vec![0, 0, 0, 255, 255, 255, 255, 255]).unwrap());
    assert_eq!(data.sampling(), ImageSampling::Linear);
    let nearest = Arc::new(data.sampled(ImageSampling::Nearest));
    assert_eq!(data.id(), nearest.id());
    assert_ne!(data, nearest);
    assert_eq!(
        nearest.transformed(Affine::IDENTITY).sampling(),
        ImageSampling::Nearest
    );
    for isolated in [false, true] {
        let mut gpu = GpuRenderer::new(120, 120).unwrap();
        let mut scene = Scene::new(120., 120.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            fixed(10., 10.),
        );
        scene.set_isolated(parent, isolated);
        let node = scene.append(parent, NodeKind::Image(data.clone()), fixed(2., 1.));
        scene.set_transform(node, Transform { x: 0.5, y: 0. });
        let damage = scene.flush().damage;
        assert_eq!(gpu.render(&scene, &damage).unwrap().image_uploads, 1);
        let linear = gpu.readback().unwrap();
        assert!(
            (120..=136).contains(&pixel(&linear, 1, 0)[0]),
            "linear {:?}",
            pixel(&linear, 1, 0)
        );
        scene.set_kind(node, NodeKind::Image(nearest.clone()));
        let report = scene.flush();
        assert_eq!(report.layout_nodes, 0);
        assert_eq!(gpu.render(&scene, &report.damage).unwrap().image_uploads, 0);
        assert_eq!(pixel(&gpu.readback().unwrap(), 1, 0), &[255, 255, 255, 255]);
    }
}

#[test]
fn nearest_half_pixel_origins_do_not_duplicate_or_skip_grid_cells() {
    let mut pixels = Vec::new();
    for y in 0..13 {
        for x in 0..13 {
            pixels.extend([x * 17, y * 17, if (x + y) % 2 == 0 { 255 } else { 0 }, 255]);
        }
    }
    let data = Arc::new(
        ImageData::new(13, 13, pixels)
            .unwrap()
            .sampled(ImageSampling::Nearest),
    );
    for isolated in [false, true] {
        let mut gpu = GpuRenderer::new(120, 120).unwrap();
        let mut scene = Scene::new(120., 120.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            fixed(30., 30.),
        );
        scene.set_isolated(parent, isolated);
        let node = scene.append(parent, NodeKind::Image(data.clone()), fixed(13., 13.));
        for origin in [0.5, 1.5, 7.5] {
            scene.set_transform(
                node,
                Transform {
                    x: origin,
                    y: origin,
                },
            );
            let damage = scene.flush().damage;
            gpu.render(&scene, &damage).unwrap();
            let actual = gpu.readback().unwrap();
            for y in 1..12 {
                for x in 1..12 {
                    let expected = &data.pixels()[(y * 13 + x) * 4..(y * 13 + x + 1) * 4];
                    assert_eq!(
                        pixel(&actual, origin as usize + x, origin as usize + y),
                        expected,
                        "cell ({x},{y}), origin {origin}, isolated {isolated}"
                    );
                }
            }
        }
    }
}

#[test]
fn affine_pixels_damage_clipping_and_isolated_coordinates() {
    let data = Arc::new(ImageData::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap());
    for isolated in [false, true] {
        for clipped in [false, true] {
            let mut gpu = GpuRenderer::new(120, 120).unwrap();
            let mut scene = Scene::new(120., 120.);
            scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
            scene.append(
                scene.root(),
                NodeKind::Rect(Color(10, 20, 30, 255)),
                fixed(120., 120.),
            );
            let parent = scene.append(
                scene.root(),
                NodeKind::Container(Layout::Overlay),
                Style {
                    clip: clipped,
                    ..fixed(40., 20.)
                },
            );
            scene.set_transform(parent, Transform { x: 40., y: 40. });
            scene.set_isolated(parent, isolated);
            let node = scene.append(parent, NodeKind::Image(data.clone()), fixed(40., 20.));
            let report = scene.flush();
            assert_eq!(gpu.render(&scene, &report.damage).unwrap().image_uploads, 1);
            let matrices = [
                Affine::rotation(std::f32::consts::FRAC_PI_2),
                Affine::scale(-1., 1.),
                Affine::scale(1.2, 0.7).then(Affine::rotation(0.7)),
                Affine {
                    c: 0.4,
                    ..Affine::IDENTITY
                },
                Affine::scale(0., 1.),
                Affine::IDENTITY,
            ];
            for (index, matrix) in matrices.into_iter().enumerate() {
                scene.set_kind(node, NodeKind::Image(Arc::new(data.transformed(matrix))));
                let report = scene.flush();
                assert_eq!(report.layout_nodes, 0);
                assert_eq!(gpu.render(&scene, &report.damage).unwrap().image_uploads, 0);
                let pixels = gpu.readback().unwrap();
                if index == 0 {
                    let top = pixel(&pixels, 60, 35);
                    let bottom = pixel(&pixels, 60, 65);
                    if clipped {
                        assert_eq!(top, &[10, 20, 30, 255]);
                        assert_eq!(bottom, &[10, 20, 30, 255]);
                    } else {
                        assert!(
                            top[0] > 245 && top[1] < 5,
                            "top {top:?}, isolated={isolated}"
                        );
                        assert!(bottom[1] > 245 && bottom[0] < 5, "bottom {bottom:?}");
                    }
                }
                let mut fresh = GpuRenderer::new(120, 120).unwrap();
                fresh
                    .render(&scene, &[Rect::new(0., 0., 120., 120.)])
                    .unwrap();
                assert_eq!(
                    pixels,
                    fresh.readback().unwrap(),
                    "isolated={isolated}, clipped={clipped}, transform={matrix:?}"
                );
            }
        }
    }
}

#[test]
fn affine_images_keep_ancestor_fades_fixed_to_the_viewport() {
    let data = Arc::new(ImageData::new(1, 1, vec![255, 0, 0, 255]).unwrap());
    for isolated in [false, true] {
        let mut gpu = GpuRenderer::new(120, 120).unwrap();
        gpu.set_background(Color(0, 0, 0, 0));
        let mut scene = Scene::new(120., 120.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                fade_edges: [20., 20.],
                clip: true,
                ..fixed(80., 80.)
            },
        );
        scene.set_transform(parent, Transform { x: 20., y: 20. });
        scene.set_isolated(parent, isolated);
        let node = scene.append(parent, NodeKind::Image(data.clone()), fixed(80., 80.));
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let identity = gpu.readback().unwrap();
        scene.set_kind(
            node,
            NodeKind::Image(Arc::new(
                data.transformed(Affine::rotation(std::f32::consts::FRAC_PI_2)),
            )),
        );
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let rotated = gpu.readback().unwrap();
        for y in [22, 25, 40, 60, 79, 95, 97] {
            assert_eq!(
                pixel(&rotated, 60, y),
                pixel(&identity, 60, y),
                "ancestor mask y={y}, isolated={isolated}"
            );
        }
        assert!(pixel(&rotated, 60, 22)[3] < 10);
        assert_eq!(pixel(&rotated, 60, 60), &[255, 0, 0, 255]);
    }
}
