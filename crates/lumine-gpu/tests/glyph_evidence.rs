//! Native observations only. Expected glyph IDs below are pinned fixture cmap
//! values, never generated from the evidence being tested.
use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, Metrics, Shaping, SwashCache, fontdb::Database,
};
use lumine::{
    affine::Affine,
    scene::*,
    text_layout::{FontFamily, FontStyle, LineHeight},
};
use lumine_gpu::{GlyphEvidenceLimits, GpuRenderer, NativeGlyphStatus};
use std::sync::Arc;

const FONT: &[u8] = include_bytes!("../../../assets/DejaVuSans.ttf");

fn renderer(bytes: &[u8]) -> GpuRenderer {
    let mut gpu = GpuRenderer::new(320, 160).unwrap();
    let mut db = Database::new();
    db.load_font_data(bytes.to_vec());
    *gpu.text_system().borrow_mut() = FontSystem::new_with_locale_and_db("en-US".into(), db);
    gpu.set_background(Color(0, 0, 0, 255));
    gpu
}
fn scene(gpu: &GpuRenderer, value: &str) -> (Scene, NodeId) {
    let mut scene = Scene::new(200., 100.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let node = scene.append(
        scene.root(),
        NodeKind::Text {
            text: Arc::from(value),
            color: Color(255, 255, 255, 255),
            font_size: 16.,
        },
        Style {
            width: Some(160.),
            height: Some(80.),
            text_wrap: false,
            clip: true,
            ..Default::default()
        },
    );
    scene.set_font(
        node,
        FontStyle {
            family: FontFamily::Named("DejaVu Sans".into()),
            line_height: LineHeight::px(22.),
            ..Default::default()
        },
    );
    gpu.install_text(&mut scene);
    (scene, node)
}
fn render(gpu: &mut GpuRenderer, scene: &mut Scene) -> lumine_gpu::GpuStats {
    let report = scene.flush();
    gpu.render(scene, &report.damage).unwrap()
}

#[test]
fn new_and_cached_shapes_keep_all_clusters_without_changing_pixels() {
    let mut gpu = renderer(FONT);
    let (mut scene, node) = scene(&gpu, "A B\né");
    render(&mut gpu, &mut scene);
    let ordinary = gpu.readback().unwrap();
    assert!(gpu.glyph_evidence().unwrap().is_none());
    gpu.enable_glyph_evidence(GlyphEvidenceLimits::default());
    assert!(
        gpu.glyph_evidence().is_err(),
        "no successful evidence before a render"
    );
    assert_eq!(render(&mut gpu, &mut scene).shaped_nodes, 1);
    assert_eq!(ordinary, gpu.readback().unwrap());
    let frame = gpu.glyph_evidence().unwrap().unwrap();
    assert!(frame.full_repaint && frame.shapes_cleared_on_enable);
    assert_eq!(frame.nodes.len(), 1);
    let actual = &frame.nodes[0];
    assert_eq!(actual.scene_node_id, node);
    assert!(!actual.shape_cached);
    let expected = [(0, 1, 0, 36), (1, 2, 0, 3), (2, 3, 0, 37), (4, 6, 1, 171)];
    assert_eq!(
        actual
            .glyphs
            .iter()
            .map(|g| (
                g.shape.cluster_start,
                g.shape.cluster_end,
                g.shape.source_line,
                g.shape.cache_key.glyph_id
            ))
            .collect::<Vec<_>>(),
        expected
    );
    let shapes = actual
        .glyphs
        .iter()
        .map(|g| g.shape.clone())
        .collect::<Vec<_>>();
    let rasters = actual
        .glyphs
        .iter()
        .map(|g| (g.raster_cache_key, g.raster, g.status))
        .collect::<Vec<_>>();
    assert_eq!(frame.fonts.len(), 1, "one blob per actual font/weight");
    assert_eq!(frame.fonts[0].bytes, FONT);
    assert_eq!(frame.fonts[0].collection_index, 0);
    assert!(
        frame.fonts[0].variation_coordinates.is_none(),
        "unavailable coordinates remain unknown"
    );
    assert!(frame.fonts[0].shaping_normalized_coords.is_empty());
    assert!(frame.fonts[0].raster_normalized_coords.is_none());
    assert!(
        frame.fonts[0].variation_axes.is_empty(),
        "pinned static font has no variation axes"
    );
    assert_eq!(render(&mut gpu, &mut scene).shaped_nodes, 0);
    let cached = &gpu.glyph_evidence().unwrap().unwrap().nodes[0];
    assert!(cached.shape_cached);
    assert_eq!(
        cached
            .glyphs
            .iter()
            .map(|g| g.shape.clone())
            .collect::<Vec<_>>(),
        shapes
    );
    assert_eq!(
        cached
            .glyphs
            .iter()
            .map(|g| (g.raster_cache_key, g.raster, g.status))
            .collect::<Vec<_>>(),
        rasters
    );
    assert_eq!(ordinary, gpu.readback().unwrap());
    let owned = gpu.take_glyph_evidence().unwrap().unwrap();
    assert_eq!(owned.fonts[0].bytes, FONT);
    assert!(gpu.glyph_evidence().unwrap().is_none());
    gpu.render(&scene, &[Rect::new(0., 0., 200., 100.)])
        .unwrap();
    assert_eq!(ordinary, gpu.readback().unwrap());
    scene.set_transform(node, Transform { x: 8., y: 4. });
    assert_eq!(render(&mut gpu, &mut scene).shaped_nodes, 0);
}

#[test]
fn missing_glyph_and_whitespace_never_claim_visible_mask_coverage() {
    let mut gpu = renderer(FONT);
    let (mut scene, _) = scene(&gpu, "A \u{10ffff}");
    gpu.enable_glyph_evidence(GlyphEvidenceLimits::default());
    render(&mut gpu, &mut scene);
    let glyphs = &gpu.glyph_evidence().unwrap().unwrap().nodes[0].glyphs;
    assert_eq!(glyphs.len(), 3);
    assert_eq!(glyphs[0].status, NativeGlyphStatus::Mask);
    assert!(glyphs[0].raster.has_ink && glyphs[0].quad.as_ref().unwrap().retained_after_cpu_clip);
    assert_eq!(glyphs[1].shape.cache_key.glyph_id, 3);
    assert_eq!(glyphs[1].status, NativeGlyphStatus::WhitespaceNoInk);
    assert!(!glyphs[1].raster.has_ink);
    assert!(glyphs[1].quad.is_none());
    assert_eq!(glyphs[2].shape.cache_key.glyph_id, 0);
    assert_eq!(
        (glyphs[2].shape.cluster_start, glyphs[2].shape.cluster_end),
        (2, 6)
    );
    assert_eq!(
        glyphs[2].status,
        NativeGlyphStatus::MissingGlyph,
        "a .notdef bitmap must not prove glyph presence"
    );
}

#[test]
fn bidi_clusters_keep_absolute_utf8_offsets_and_actual_direction() {
    let mut gpu = renderer(FONT);
    let (mut scene, _) = scene(&gpu, "A\nאב");
    gpu.enable_glyph_evidence(GlyphEvidenceLimits::default());
    render(&mut gpu, &mut scene);
    let glyphs = &gpu.glyph_evidence().unwrap().unwrap().nodes[0].glyphs;
    assert_eq!(glyphs.len(), 3);
    for (start, end, id) in [(2, 4, 1319), (4, 6, 1320)] {
        let glyph = glyphs
            .iter()
            .find(|g| g.shape.cluster_start == start)
            .unwrap();
        assert_eq!(glyph.shape.cluster_end, end);
        assert_eq!(glyph.shape.cache_key.glyph_id, id);
        assert_eq!(glyph.shape.source_line, 1);
        assert_eq!(glyph.shape.layout_line, 1);
        assert!(glyph.shape.paragraph_rtl);
        assert_eq!(glyph.shape.bidi_level % 2, 1);
    }
}

#[test]
fn generated_glyph_quads_record_actual_cpu_clip_culling() {
    let mut gpu = renderer(FONT);
    let (mut scene, node) = scene(&gpu, "A   A");
    let mut style = scene.style(node);
    style.width = Some(20.);
    scene.set_style(node, style);
    gpu.enable_glyph_evidence(GlyphEvidenceLimits::default());
    render(&mut gpu, &mut scene);
    let glyphs = &gpu.glyph_evidence().unwrap().unwrap().nodes[0].glyphs;
    assert_eq!(glyphs.len(), 5);
    assert!(glyphs[0].quad.as_ref().unwrap().retained_after_cpu_clip);
    assert!(!glyphs[4].quad.as_ref().unwrap().retained_after_cpu_clip);
    assert_eq!(glyphs[0].quad.as_ref().unwrap().submitted, Some(true));
    assert_eq!(glyphs[4].quad.as_ref().unwrap().submitted, Some(false));
    assert_eq!(glyphs[4].quad.as_ref().unwrap().instance_index, None);
    assert_eq!(glyphs[4].shape.cache_key.glyph_id, 36);
}

#[test]
fn actual_batch_membership_excludes_glyphs_under_an_opaque_cover() {
    let mut gpu = renderer(FONT);
    let (mut scene, _) = scene(&gpu, "A");
    scene.resize(320., 160.);
    scene.append(
        scene.root(),
        NodeKind::Quad(QuadStyle {
            fill: Color(0, 0, 0, 255),
            ..Default::default()
        }),
        Style {
            width: Some(320.),
            height: Some(160.),
            ..Default::default()
        },
    );
    gpu.enable_glyph_evidence(GlyphEvidenceLimits::default());
    render(&mut gpu, &mut scene);
    assert!(
        gpu.readback()
            .unwrap()
            .chunks_exact(4)
            .all(|pixel| pixel == [0, 0, 0, 255])
    );
    let quad = gpu.glyph_evidence().unwrap().unwrap().nodes[0].glyphs[0]
        .quad
        .as_ref()
        .unwrap();
    assert!(quad.retained_after_cpu_clip);
    assert_eq!(
        quad.submitted,
        Some(false),
        "opaque full-region cover suppresses the actual batch"
    );
    assert!(quad.instance_index.is_some());
}

#[test]
fn actual_raster_quad_points_match_scale_and_affine_without_metadata_as_reference() {
    let mut gpu = renderer(FONT);
    let (mut scene, node) = scene(&gpu, "A");
    let matrix = Affine {
        b: 0.25,
        ..Affine::IDENTITY
    };
    scene.set_paint_transform(node, matrix);
    scene.set_transform(node, Transform { x: 0.25, y: 0.5 });
    // The renderer bakes affine translation into primitive placement. Derive
    // its equivalent origin from the requested matrix, never from evidence.
    let origin = matrix.inverse().unwrap().point(0.25, 0.5);
    for scale in [1., 1.25, 1.5, 2.] {
        gpu.set_scale_factor(scale);
        gpu.enable_glyph_evidence(GlyphEvidenceLimits::default());
        render(&mut gpu, &mut scene);
        // Shape/raster the known fixture independently of the collector output.
        let system = gpu.text_system();
        let mut fonts = system.borrow_mut();
        let mut buffer = Buffer::new(&mut fonts, Metrics::new(16., 22.));
        buffer.set_text(
            "A",
            &Attrs::new().family(Family::Name("DejaVu Sans")),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut fonts, false);
        let run = buffer.layout_runs().next().unwrap();
        let glyph = &run.glyphs[0];
        assert_eq!(glyph.glyph_id, 36);
        let phase = (
            (origin.0 * scale).rem_euclid(1.),
            (origin.1 * scale).rem_euclid(1.),
        );
        let (key, x, y) = CacheKey::new(
            glyph.font_id,
            36,
            16. * scale,
            (
                (glyph.x + 16. * glyph.x_offset) * scale + phase.0,
                (glyph.y - 16. * glyph.y_offset + run.line_y.round()) * scale + phase.1,
            ),
            glyph.font_weight,
            glyph.physical((0., 0.), scale).cache_key.flags,
        );
        let image = SwashCache::new()
            .get_image_uncached(&mut fonts, key)
            .unwrap();
        let p = image.placement;
        let expected = Rect::new(
            ((x + p.left) as f32 - phase.0) / scale + origin.0,
            ((y - p.top) as f32 - phase.1) / scale + origin.1,
            p.width as f32 / scale,
            p.height as f32 / scale,
        );
        let frame = gpu.glyph_evidence().unwrap().unwrap();
        let node = &frame.nodes[0];
        assert_eq!(node.transform, matrix);
        let allocation_clip = node.allocation_clip.unwrap();
        let allocation = scene.layout_bounds(node.scene_node_id);
        assert_eq!(
            allocation_clip.bounds,
            Rect::new(
                allocation.x + origin.0,
                allocation.y + origin.1,
                allocation.width,
                allocation.height
            )
        );
        assert_eq!(allocation_clip.inverse, matrix.inverse().unwrap());
        assert!(
            !node.clip_regions.is_empty(),
            "affine clip bindings must be retained"
        );
        let observed = &node.glyphs[0];
        assert_eq!(observed.raster_cache_key, key);
        let quad = observed.quad.as_ref().unwrap();
        assert_eq!(quad.bounds, expected);
        for (index, (x, y)) in [
            (expected.x, expected.y),
            (expected.x + expected.width, expected.y),
            (expected.x + expected.width, expected.y + expected.height),
            (expected.x, expected.y + expected.height),
        ]
        .into_iter()
        .enumerate()
        {
            let point = matrix.point(x, y);
            assert_eq!(quad.scene_points[index], [point.0, point.1]);
            assert_eq!(
                quad.physical_points[index],
                [point.0 * scale, point.1 * scale]
            );
        }
    }
}

// Package existing fonts into a real TTC, relocating each sfnt table offset.
// Font bytes and face index must both survive; assuming index zero is incorrect.
fn collection() -> Vec<u8> {
    let mut bytes = b"ttcf\0\x01\0\0\0\0\0\x02\0\0\0\0\0\0\0\0".to_vec();
    for (index, font) in [include_bytes!("fixtures/Geist.ttf").as_slice(), FONT]
        .into_iter()
        .enumerate()
    {
        bytes.resize(bytes.len().next_multiple_of(4), 0);
        let base = bytes.len();
        bytes[12 + index * 4..16 + index * 4].copy_from_slice(&(base as u32).to_be_bytes());
        bytes.extend_from_slice(font);
        let count = u16::from_be_bytes(font[4..6].try_into().unwrap()) as usize;
        for table in 0..count {
            let offset = 12 + table * 16 + 8;
            let original = u32::from_be_bytes(font[offset..offset + 4].try_into().unwrap());
            bytes[base + offset..base + offset + 4]
                .copy_from_slice(&(original + base as u32).to_be_bytes());
        }
    }
    bytes
}

#[test]
fn actual_font_bytes_are_bound_to_the_selected_collection_face() {
    let bytes = collection();
    let mut gpu = renderer(&bytes);
    let (mut scene, _) = scene(&gpu, "AA");
    gpu.enable_glyph_evidence(GlyphEvidenceLimits::default());
    render(&mut gpu, &mut scene);
    let frame = gpu.glyph_evidence().unwrap().unwrap();
    assert_eq!(frame.fonts.len(), 1);
    let font = &frame.fonts[0];
    assert_eq!(font.bytes, bytes);
    assert_eq!(font.collection_index, 1);
    for glyph in &frame.nodes[0].glyphs {
        assert_eq!(glyph.font_index, 0);
        assert_eq!(glyph.shape.cache_key.glyph_id, 36);
        assert_eq!(glyph.raster_cache_key.font_id, font.font_id);
        assert_eq!(glyph.raster_cache_key.font_weight.0, font.weight);
    }
}

#[test]
fn evidence_limits_fail_closed_while_rendering_stays_unchanged() {
    let mut gpu = renderer(FONT);
    let (mut scene, _) = scene(&gpu, "A B");
    render(&mut gpu, &mut scene);
    let ordinary = gpu.readback().unwrap();
    for limits in [
        GlyphEvidenceLimits {
            glyphs: 1,
            ..Default::default()
        },
        GlyphEvidenceLimits {
            nodes: 0,
            ..Default::default()
        },
        GlyphEvidenceLimits {
            font_bytes: FONT.len() - 1,
            ..Default::default()
        },
        GlyphEvidenceLimits {
            fonts: 0,
            ..Default::default()
        },
    ] {
        gpu.enable_glyph_evidence(limits);
        render(&mut gpu, &mut scene);
        assert!(
            gpu.glyph_evidence().is_err(),
            "partial evidence cannot be successful"
        );
        assert_eq!(ordinary, gpu.readback().unwrap());
    }
}

#[test]
fn cached_isolated_text_is_observed_on_every_collected_frame() {
    let mut gpu = renderer(FONT);
    let (mut scene, node) = scene(&gpu, "A");
    scene.set_isolated(scene.root(), true);
    scene.set_effects(
        scene.root(),
        Effects {
            opacity: 0.5,
            ..Default::default()
        },
    );
    render(&mut gpu, &mut scene);
    let ordinary = gpu.readback().unwrap();
    gpu.enable_glyph_evidence(GlyphEvidenceLimits::default());
    render(&mut gpu, &mut scene);
    let first = gpu.readback().unwrap();
    assert_eq!(first, ordinary);
    assert_eq!(
        gpu.glyph_evidence().unwrap().unwrap().nodes[0].scene_node_id,
        node
    );
    let stats = render(&mut gpu, &mut scene);
    assert_eq!(stats.shaped_nodes, 0);
    let frame = gpu.glyph_evidence().unwrap().unwrap();
    assert_eq!(frame.nodes.len(), 1);
    assert!(frame.nodes[0].shape_cached);
    assert_eq!(frame.nodes[0].layer, Some(scene.root()));
    assert_eq!(
        frame.nodes[0].glyphs[0].quad.as_ref().unwrap().submitted,
        Some(true)
    );
    assert_eq!(frame.layers[0].submitted, Some(true));
    assert_eq!(frame.nodes[0].opacity, 1.);
    assert_eq!(frame.layers.len(), 1);
    assert_eq!(frame.layers[0].scene_node_id, scene.root());
    assert_eq!(frame.layers[0].parent_layer, None);
    assert_eq!(frame.layers[0].effects.opacity, 0.5);
    assert_eq!(frame.nodes[0].glyphs[0].shape.cache_key.glyph_id, 36);
    assert_eq!(first, gpu.readback().unwrap());
}

#[test]
fn compacted_glyph_instance_indices_follow_actual_cpu_culling() {
    let mut gpu = renderer(FONT);
    let (mut scene, node) = scene(&gpu, "A B C");
    let mut style = scene.style(node);
    style.width = Some(30.);
    scene.set_style(node, style);
    gpu.enable_glyph_evidence(GlyphEvidenceLimits::default());
    render(&mut gpu, &mut scene);
    let frame = gpu.glyph_evidence().unwrap().unwrap();
    let glyphs = &frame.nodes[0].glyphs;
    let kept: Vec<_> = glyphs
        .iter()
        .filter_map(|g| g.quad.as_ref())
        .filter(|q| q.retained_after_cpu_clip)
        .collect();
    assert!(kept.len() >= 2);
    assert!(
        kept.windows(2)
            .all(|pair| pair[1].instance_index.unwrap() == pair[0].instance_index.unwrap() + 1)
    );
    assert!(kept.iter().all(|q| q.submitted == Some(true)));
    assert!(
        glyphs
            .iter()
            .filter_map(|g| g.quad.as_ref())
            .any(|q| q.submitted == Some(false))
    );
}

#[test]
fn glyph_submitted_to_layer_is_excluded_when_its_composite_is_occluded() {
    let mut gpu = renderer(FONT);
    let (mut scene, node) = scene(&gpu, "A");
    scene.set_isolated(node, true);
    scene.resize(320., 160.);
    scene.append(
        scene.root(),
        NodeKind::Quad(QuadStyle {
            fill: Color(0, 0, 0, 255),
            ..Default::default()
        }),
        Style {
            width: Some(320.),
            height: Some(160.),
            ..Default::default()
        },
    );
    gpu.enable_glyph_evidence(GlyphEvidenceLimits::default());
    render(&mut gpu, &mut scene);
    let frame = gpu.glyph_evidence().unwrap().unwrap();
    assert_eq!(frame.nodes[0].layer, Some(node));
    assert_eq!(
        frame
            .layers
            .iter()
            .find(|layer| layer.scene_node_id == node)
            .unwrap()
            .submitted,
        Some(false)
    );
    assert_eq!(
        frame.nodes[0].glyphs[0].quad.as_ref().unwrap().submitted,
        Some(false)
    );
}
