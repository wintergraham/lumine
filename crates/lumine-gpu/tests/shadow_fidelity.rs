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
    for (x, expected) in [
        (31, 5),
        (36, 35),
        (41, 117),
        (42, 138),
        (47, 220),
        (52, 250),
    ] {
        assert!(
            (alpha(x, 70) - expected).abs() <= 1,
            "x={x}: {} != {expected}",
            alpha(x, 70)
        );
    }
    // Own clipping must not chop the blurred outer shadow at x=40.
    assert!(alpha(36, 70) > 0);
    assert_eq!(alpha(151, 70), 0);
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
