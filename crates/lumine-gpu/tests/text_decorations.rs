use cosmic_text::{FontSystem, fontdb::Database};
use lumine::{
    rich_text::{Decoration, RichText, TextRun},
    scene::{Color, NodeKind, Rect, Scene, Style},
    text_layout::{FontFamily, FontStyle, LineHeight},
};
use lumine_gpu::GpuRenderer;
use std::sync::Arc;

fn same_pixels(actual: &[u8], expected: &[u8], context: &str) {
    assert_eq!(actual.len(), expected.len());
    assert!(
        actual == expected,
        "{context}: {} differing RGBA pixels, first {:?}",
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
fn baseline_offsets_and_zero_thickness_invalidate_retained_decoration_geometry() {
    let mut gpu = GpuRenderer::new(180, 80).unwrap();
    let mut db = Database::new();
    db.load_font_data(include_bytes!("fixtures/Geist.ttf").to_vec());
    *gpu.text_system().borrow_mut() = FontSystem::new_with_locale_and_db("en-US".into(), db);
    gpu.set_background(Color(0, 0, 0, 255));
    let mut fresh = GpuRenderer::new_with_context(180, 80, &gpu.context()).unwrap();
    fresh.set_background(Color(0, 0, 0, 255));
    let font = FontStyle {
        family: FontFamily::Named("Geist".into()),
        line_height: LineHeight::px(40.),
        ..Default::default()
    };
    let value = "Underline";
    let mut scene = Scene::new(180., 80.);
    gpu.install_text(&mut scene);
    let node = scene.append(
        scene.root(),
        NodeKind::Text {
            text: value.into(),
            color: Color(255, 255, 255, 255),
            font_size: 20.,
        },
        Style {
            width: Some(160.),
            height: Some(60.),
            ..Default::default()
        },
    );
    scene.set_font(node, font.clone());
    let frame = scene.flush();
    gpu.render(&scene, &frame.damage).unwrap();
    let plain = gpu.readback().unwrap();
    let make = |decoration| NodeKind::RichText {
        text: Arc::new(
            RichText::new(
                value,
                vec![TextRun {
                    range: 0..value.len(),
                    font: font.clone(),
                    font_size: 20.,
                    underline: Some(decoration),
                    ..Default::default()
                }],
            )
            .unwrap(),
        ),
    };
    let green = Color(0, 255, 0, 255);
    scene.set_kind(node, make(Decoration::default().with_thickness(0.)));
    let frame = scene.flush();
    gpu.render(&scene, &frame.damage).unwrap();
    same_pixels(
        &gpu.readback().unwrap(),
        &plain,
        "plain and undecorated rich glyphs match",
    );
    let mut rows = Vec::new();
    for offset in [4., -4., 0., 4.] {
        scene.set_kind(
            node,
            make(
                Decoration::new(2.)
                    .color(green)
                    .with_baseline_offset(offset),
            ),
        );
        let frame = scene.flush();
        assert_eq!(
            frame.layout_nodes, 0,
            "decoration changes preserve text metrics"
        );
        let stats = gpu.render(&scene, &frame.damage).unwrap();
        assert_eq!(
            stats.shaped_nodes, 1,
            "offset changes cannot use the recolor-only path"
        );
        let pixels = gpu.readback().unwrap();
        fresh
            .render(&scene, &[Rect::new(0., 0., 180., 80.)])
            .unwrap();
        same_pixels(
            &pixels,
            &fresh.readback().unwrap(),
            "retained offset versus fresh geometry",
        );
        let painted_rows: Vec<_> = pixels
            .chunks_exact(180 * 4)
            .enumerate()
            .filter_map(|(y, row)| {
                row.chunks_exact(4)
                    .any(|p| p == [0, 255, 0, 255])
                    .then_some(y)
            })
            .collect();
        assert!(
            !painted_rows.is_empty(),
            "underline must paint at offset {offset}"
        );
        rows.push(painted_rows[0]);
    }
    assert_eq!(rows[0], rows[3]);
    assert_eq!(
        rows[0] - rows[1],
        8,
        "signed offsets move the top edge by their exact difference"
    );
    let red = Color(255, 0, 0, 255);
    scene.set_kind(
        node,
        make(Decoration::new(2.).color(red).with_baseline_offset(4.)),
    );
    let frame = scene.flush();
    assert_eq!(
        gpu.render(&scene, &frame.damage).unwrap().shaped_nodes,
        0,
        "color-only changes reuse geometry"
    );
    assert!(
        gpu.readback()
            .unwrap()
            .chunks_exact(4)
            .any(|p| p == [255, 0, 0, 255])
    );
    scene.set_kind(
        node,
        make(
            Decoration::new(2.)
                .color(green)
                .with_baseline_offset(4.)
                .with_thickness(0.),
        ),
    );
    let frame = scene.flush();
    assert_eq!(frame.layout_nodes, 0);
    gpu.render(&scene, &frame.damage).unwrap();
    same_pixels(
        &gpu.readback().unwrap(),
        &plain,
        "exact zero thickness paints only the original glyphs",
    );
    scene.set_kind(node, make(Decoration::new(0.).color(green)));
    let frame = scene.flush();
    gpu.render(&scene, &frame.damage).unwrap();
    assert!(
        gpu.readback()
            .unwrap()
            .chunks_exact(4)
            .any(|p| p[1] > 0 && p[0] == 0 && p[2] == 0),
        "legacy new(0.) still paints a line"
    );
}
