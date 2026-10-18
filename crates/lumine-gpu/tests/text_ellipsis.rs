use cosmic_text::{FontSystem, fontdb::Database};
use lumine::{
    scene::{Align, Color, Layout, NodeKind, Scene, Style},
    text_layout::{FontFamily, FontStyle, LineHeight, TextOptions, TextOverflow},
};
use lumine_gpu::GpuRenderer;

#[test]
fn single_line_ellipsis_layout_and_gpu_paint_match_untruncated_automation_text() {
    let mut gpu = GpuRenderer::new(200, 48).unwrap();
    // The imported page's actual font, isolated from host fonts for both layout and paint.
    let mut db = Database::new();
    db.load_font_data(include_bytes!("fixtures/Geist.ttf").to_vec());
    *gpu.text_system().borrow_mut() = FontSystem::new_with_locale_and_db("en-US".into(), db);
    gpu.set_background(Color(0, 0, 0, 255));
    let mut scene = Scene::new(200., 48.);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Column));
    scene.set_style(
        root,
        Style {
            // Reviewed source/native .au-name and .au-summary allocation.
            width: Some(154.3875),
            height: Some(48.),
            align: Align::Start,
            gap: 2.,
            clip: true,
            ..Default::default()
        },
    );
    gpu.install_text(&mut scene);
    let mut nodes = Vec::new();
    for (text, size) in [
        ("Release notes to Slack", 13.),
        ("Post GitHub releases to #ship", 11.5),
    ] {
        let node = scene.append(
            root,
            NodeKind::Text {
                text: text.into(),
                font_size: size,
                color: Color(255, 255, 255, 255),
            },
            Style::default(),
        );
        scene.set_font(
            node,
            FontStyle {
                family: FontFamily::Named("Geist".into()),
                line_height: LineHeight::px(size * 1.3),
                ..Default::default()
            },
        );
        nodes.push(node);
    }
    let frame = scene.flush();
    gpu.render(&scene, &frame.damage).unwrap();
    let reference = gpu.readback().unwrap();
    assert!(reference.chunks_exact(4).any(|pixel| pixel[0] != 0));
    let natural: Vec<_> = nodes
        .iter()
        .map(|node| scene.layout_bounds(*node))
        .collect();
    let options = TextOptions {
        overflow: TextOverflow::Ellipsis,
        line_clamp: std::num::NonZeroU32::new(1),
    };
    for node in &nodes {
        let mut style = scene.style(*node);
        style.text_options = options;
        scene.set_style(*node, style);
    }
    let frame = scene.flush();
    for (node, expected) in nodes.iter().zip(&natural) {
        let actual = scene.layout_bounds(*node);
        assert!(
            (actual.width - expected.width).abs() < 0.001,
            "{actual:?} vs {expected:?}"
        );
        assert_eq!(actual.height, expected.height);
    }
    gpu.render(&scene, &frame.damage).unwrap();
    assert_eq!(
        gpu.readback().unwrap(),
        reference,
        "fitting text must keep every glyph"
    );
    for node in &nodes {
        let mut style = scene.style(*node);
        style.width = Some(80.);
        scene.set_style(*node, style);
    }
    let frame = scene.flush();
    gpu.render(&scene, &frame.damage).unwrap();
    assert_ne!(
        gpu.readback().unwrap(),
        reference,
        "real overflow still truncates"
    );
    for node in &nodes {
        let mut style = scene.style(*node);
        style.width = None;
        scene.set_style(*node, style);
    }
    let frame = scene.flush();
    gpu.render(&scene, &frame.damage).unwrap();
    assert_eq!(
        gpu.readback().unwrap(),
        reference,
        "restoring width must restore full text"
    );
    let frame = scene.flush();
    assert_eq!(gpu.render(&scene, &frame.damage).unwrap().shaped_nodes, 0);
}
