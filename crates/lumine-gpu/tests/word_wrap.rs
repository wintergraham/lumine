use cosmic_text::{FontSystem, fontdb::Database};
use lumine::{
    rich_text::{RichText, TextRun},
    scene::{Color, NodeKind, Scene, Style},
    text_layout::{FontFamily, FontStyle, LineHeight, WordWrap},
};
use lumine_gpu::{GpuRenderer, text::ShapedText};
use std::sync::Arc;

fn label(value: &str, font: &FontStyle, rich: bool) -> NodeKind {
    if rich {
        NodeKind::RichText {
            text: Arc::new(
                RichText::new(
                    value,
                    vec![TextRun {
                        range: 0..value.len(),
                        font: font.clone(),
                        font_size: 13.,
                        color: Color(255, 255, 255, 255),
                        ..Default::default()
                    }],
                )
                .unwrap(),
            ),
        }
    } else {
        NodeKind::Text {
            text: value.into(),
            font_size: 13.,
            color: Color(255, 255, 255, 255),
        }
    }
}

fn same_pixels(actual: &[u8], expected: &[u8], context: &str) {
    assert_eq!(actual.len(), expected.len());
    assert!(
        actual == expected,
        "{context}: {} differing RGBA pixels; first difference {:?}",
        actual
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count(),
        actual
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .position(|(a, b)| a != b)
    );
}

#[test]
fn browser_word_policy_keeps_underwidth_labels_and_wraps_real_paragraphs() {
    let mut gpu = GpuRenderer::new(160, 80).unwrap();
    let mut db = Database::new();
    db.load_font_data(include_bytes!("fixtures/Geist.ttf").to_vec());
    *gpu.text_system().borrow_mut() = FontSystem::new_with_locale_and_db("en-US".into(), db);
    gpu.set_background(Color(0, 0, 0, 255));
    for rich in [false, true] {
        for value in ["Active", "Paused"] {
            let mut font = FontStyle {
                family: FontFamily::Named("Geist".into()),
                line_height: LineHeight::px(16.9),
                word_wrap: WordWrap::Word,
                ..Default::default()
            };
            let natural = ShapedText::with_font(
                &mut gpu.text_system().borrow_mut(),
                value,
                13.,
                Some(f32::INFINITY),
                &font,
            )
            .size()
            .0;
            let mut scene = Scene::new(160., 80.);
            gpu.install_text(&mut scene);
            let node = scene.append(
                scene.root(),
                label(value, &font, rich),
                Style {
                    width: Some(natural - 0.01),
                    text_wrap: true,
                    ..Default::default()
                },
            );
            scene.set_font(node, font.clone());
            let frame = scene.flush();
            assert!((scene.bounds(node).height - 16.9).abs() < 0.001);
            gpu.render(&scene, &frame.damage).unwrap();
            let words = gpu.readback().unwrap();
            assert!(words.chunks_exact(4).any(|pixel| pixel[0] > 0));
            // A single word has identical glyph positions with explicit nowrap at this same clip.
            font.word_wrap = WordWrap::WordOrGlyph;
            scene.set_kind(node, label(value, &font, rich));
            scene.set_font(node, font.clone());
            let mut style = scene.style(node);
            style.text_wrap = false;
            scene.set_style(node, style);
            let frame = scene.flush();
            assert!((scene.bounds(node).height - 16.9).abs() < 0.001);
            gpu.render(&scene, &frame.damage).unwrap();
            same_pixels(&gpu.readback().unwrap(), &words, value);
            let mut style = scene.style(node);
            style.text_wrap = true;
            scene.set_style(node, style);
            let frame = scene.flush();
            assert!(
                (scene.bounds(node).height - 33.8).abs() < 0.001,
                "native default still splits an oversized word"
            );
            gpu.render(&scene, &frame.damage).unwrap();
            assert!(gpu.readback().unwrap() != words);
            font.word_wrap = WordWrap::Word;
            scene.set_kind(node, label(value, &font, rich));
            scene.set_font(node, font);
            let frame = scene.flush();
            gpu.render(&scene, &frame.damage).unwrap();
            same_pixels(&gpu.readback().unwrap(), &words, "restored word policy");
            let frame = scene.flush();
            assert_eq!(gpu.render(&scene, &frame.damage).unwrap().shaped_nodes, 0);
        }
        let font = FontStyle {
            family: FontFamily::Named("Geist".into()),
            line_height: LineHeight::px(16.9),
            word_wrap: WordWrap::Word,
            ..Default::default()
        };
        let value = "Active Paused";
        let narrow = ShapedText::with_font(
            &mut gpu.text_system().borrow_mut(),
            "Paused",
            13.,
            Some(f32::INFINITY),
            &font,
        )
        .size()
        .0 + 1.;
        let mut scene = Scene::new(160., 80.);
        gpu.install_text(&mut scene);
        let node = scene.append(
            scene.root(),
            label(value, &font, rich),
            Style {
                width: Some(150.),
                text_wrap: true,
                ..Default::default()
            },
        );
        scene.set_font(node, font);
        let frame = scene.flush();
        gpu.render(&scene, &frame.damage).unwrap();
        let wide = gpu.readback().unwrap();
        let mut style = scene.style(node);
        style.width = Some(narrow);
        scene.set_style(node, style);
        let frame = scene.flush();
        assert!(
            (scene.bounds(node).height - 33.8).abs() < 0.001,
            "real word boundaries still wrap"
        );
        gpu.render(&scene, &frame.damage).unwrap();
        assert!(gpu.readback().unwrap() != wide);
        let mut style = scene.style(node);
        style.width = Some(150.);
        scene.set_style(node, style);
        let frame = scene.flush();
        gpu.render(&scene, &frame.damage).unwrap();
        same_pixels(
            &gpu.readback().unwrap(),
            &wide,
            "paragraph width restoration",
        );
    }
}
