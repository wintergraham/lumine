use cosmic_text::{FontSystem, fontdb::Database};
use lumine::{
    layout::{Display, LayoutOptions},
    scene::{Align, Color, Layout, NodeKind, Scene, Style},
    text_layout::{FontFamily, FontStyle, LineHeight},
};
use lumine_gpu::{GpuRenderer, text::ShapedText};
use std::sync::Arc;

#[test]
fn normal_text_max_content_matches_gpu_paint_and_keeps_real_wrapping() {
    let mut gpu = GpuRenderer::new(800, 96).unwrap();
    let mut db = Database::new();
    db.load_font_data(include_bytes!("fixtures/Geist.ttf").to_vec());
    *gpu.text_system().borrow_mut() = FontSystem::new_with_locale_and_db("en-US".into(), db);
    gpu.set_background(Color(0, 0, 0, 255));
    for (value, size, pitch, weight) in [
        ("1 setting matches “Other tabs”", 13., 16.9, 400),
        (
            "Only when you @‑mention them or the task needs them",
            12.,
            17.4,
            400,
        ),
        (
            "Found 12 items · 1 qualifies · would message you",
            12.,
            15.6,
            400,
        ),
        ("Opus 5.5 · Deep", 15., 19.5, 500),
        ("add/send.", 13., 17., 400),
    ] {
        let font = FontStyle {
            family: FontFamily::Named("Geist".into()),
            line_height: LineHeight::px(pitch),
            weight,
            ..Default::default()
        };
        // Normal MaxContent retains soft boundaries even without a finite limit.
        let expected = ShapedText::with_font(
            &mut gpu.text_system().borrow_mut(),
            value,
            size,
            Some(f32::INFINITY),
            &font,
        )
        .size();
        let mut scene = Scene::new(800., 96.);
        let root = scene.root();
        scene.set_kind(root, NodeKind::Container(Layout::Row));
        scene.set_style(
            root,
            Style {
                width: Some(800.),
                height: Some(96.),
                align: Align::Start,
                // Flex basis calculation must issue a real MaxContent probe.
                layout_options: Some(Arc::new(LayoutOptions {
                    display: Some(Display::Flex),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        gpu.install_text(&mut scene);
        let node = scene.append(
            root,
            NodeKind::Text {
                text: value.into(),
                font_size: size,
                color: Color(255, 255, 255, 255),
            },
            Style {
                text_wrap: true,
                ..Default::default()
            },
        );
        scene.set_font(node, font);
        let frame = scene.flush();
        let natural = scene.layout_bounds(node);
        assert!(
            (natural.width - expected.0).abs() < 0.001,
            "{value}: {natural:?} vs {expected:?}"
        );
        assert!(
            (natural.height - pitch).abs() < 0.001,
            "{value}: {natural:?}"
        );
        gpu.render(&scene, &frame.damage).unwrap();
        let intrinsic = gpu.readback().unwrap();
        assert!(intrinsic.chunks_exact(4).any(|pixel| pixel[0] != 0));
        let mut style = scene.style(node);
        style.width = Some(700.);
        scene.set_style(node, style);
        let frame = scene.flush();
        gpu.render(&scene, &frame.damage).unwrap();
        assert_eq!(
            gpu.readback().unwrap(),
            intrinsic,
            "{value}: natural-width paint loses glyphs"
        );
        let narrow = expected.0 * 0.55;
        let mut style = scene.style(node);
        style.width = Some(narrow);
        scene.set_style(node, style);
        let frame = scene.flush();
        assert!(scene.layout_bounds(node).height >= pitch * 2. - 0.001);
        gpu.render(&scene, &frame.damage).unwrap();
        assert_ne!(
            gpu.readback().unwrap(),
            intrinsic,
            "{value}: finite width must wrap"
        );
        let mut style = scene.style(node);
        style.text_wrap = false;
        scene.set_style(node, style);
        let frame = scene.flush();
        assert!(
            (scene.layout_bounds(node).height - pitch).abs() < 0.001,
            "nowrap remains one line"
        );
        gpu.render(&scene, &frame.damage).unwrap();
        if value == "add/send." {
            let nowrap = gpu.readback().unwrap();
            assert_ne!(
                nowrap, intrinsic,
                "explicit nowrap retains cross-slash kerning"
            );
            let mut style = scene.style(node);
            style.width = Some(700.);
            scene.set_style(node, style);
            let frame = scene.flush();
            gpu.render(&scene, &frame.damage).unwrap();
            assert_eq!(
                gpu.readback().unwrap(),
                nowrap,
                "nowrap ignores a soft width limit"
            );
        }
        let mut style = scene.style(node);
        style.width = None;
        style.text_wrap = true;
        scene.set_style(node, style);
        let frame = scene.flush();
        gpu.render(&scene, &frame.damage).unwrap();
        assert_eq!(
            gpu.readback().unwrap(),
            intrinsic,
            "{value}: restoration must reuse the correct mode"
        );
        let frame = scene.flush();
        assert_eq!(gpu.render(&scene, &frame.damage).unwrap().shaped_nodes, 0);
    }
}
