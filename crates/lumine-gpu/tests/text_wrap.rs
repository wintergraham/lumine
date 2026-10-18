use cosmic_text::{FontSystem, fontdb::Database};
use lumine::{
    layout::{Display, LayoutOptions},
    scene::{Align, Color, Layout, NodeKind, Scene, Style},
    text_layout::{FontFamily, FontStyle, LineHeight},
};
use lumine_gpu::{GpuRenderer, text::ShapedText};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
};

fn pixel_hash(pixels: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    pixels.hash(&mut hasher);
    hasher.finish()
}

fn assert_pixels_equal(actual: &[u8], expected: &[u8], width: usize, context: &str) {
    assert_eq!(actual.len(), expected.len(), "{context}: frame lengths");
    if actual == expected {
        return;
    }
    let (mut left, mut top, mut right, mut bottom, mut count) = (usize::MAX, usize::MAX, 0, 0, 0);
    for (index, (actual, expected)) in actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .enumerate()
    {
        if actual != expected {
            let (x, y) = (index % width, index / width);
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
            count += 1;
        }
    }
    panic!(
        "{context}: {count} pixel differences in [{left}, {top}, {}, {}]; actual hash {:016x}, expected hash {:016x}",
        right - left,
        bottom - top,
        pixel_hash(actual),
        pixel_hash(expected),
    );
}

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
        assert_pixels_equal(
            &gpu.readback().unwrap(),
            &intrinsic,
            800,
            &format!("{value}: natural-width paint loses glyphs"),
        );
        let narrow = expected.0 * 0.55;
        let mut style = scene.style(node);
        style.width = Some(narrow);
        scene.set_style(node, style);
        let frame = scene.flush();
        assert!(scene.layout_bounds(node).height >= pitch * 2. - 0.001);
        gpu.render(&scene, &frame.damage).unwrap();
        assert!(
            gpu.readback().unwrap() != intrinsic,
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
            let clipped_nowrap = gpu.readback().unwrap();
            let clip = scene.bounds(node);
            let mut style = scene.style(node);
            style.width = Some(700.);
            scene.set_style(node, style);
            let frame = scene.flush();
            gpu.render(&scene, &frame.damage).unwrap();
            let nowrap = gpu.readback().unwrap();
            assert!(
                nowrap != intrinsic,
                "full-width explicit nowrap retains cross-slash kerning; hash {:016x}",
                pixel_hash(&nowrap),
            );
            let mut expected_clipped = nowrap;
            // The renderer intersects ordinary text with its box, then rounds its scissor out.
            for (index, pixel) in expected_clipped.chunks_exact_mut(4).enumerate() {
                let (x, y) = (index % 800, index / 800);
                if (x as f32) < clip.x.floor()
                    || (x as f32) >= (clip.x + clip.width).ceil()
                    || (y as f32) < clip.y.floor()
                    || (y as f32) >= (clip.y + clip.height).ceil()
                {
                    pixel.copy_from_slice(&[0, 0, 0, 255]);
                }
            }
            assert_pixels_equal(
                &clipped_nowrap,
                &expected_clipped,
                800,
                "nowrap preserves glyph positions inside its text-box clip",
            );
        }
        let mut style = scene.style(node);
        style.width = None;
        style.text_wrap = true;
        scene.set_style(node, style);
        let frame = scene.flush();
        gpu.render(&scene, &frame.damage).unwrap();
        assert_pixels_equal(
            &gpu.readback().unwrap(),
            &intrinsic,
            800,
            &format!("{value}: restoration must reuse the correct mode"),
        );
        let frame = scene.flush();
        assert_eq!(gpu.render(&scene, &frame.damage).unwrap().shaped_nodes, 0);
    }
}
