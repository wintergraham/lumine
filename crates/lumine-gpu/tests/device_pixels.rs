use cosmic_text::{Attrs, Buffer, CacheKey, Family, Metrics, Shaping, SwashCache};
use std::sync::Arc;
use lumine::{
    scene::*,
    svg::SvgData,
    text_layout::{FontFamily, FontStyle, LineHeight},
};
use lumine_gpu::{GpuRenderer, text::FontData};

const WIDTH: usize = 360;
const HEIGHT: usize = 72;

fn output(name: &str, pixels: &[u8]) {
    // Optional inspectable artifacts; tests do not depend on existing files.
    if let Some(path) = std::env::var_os("LUMINE_PIXEL_ARTIFACTS") {
        let path = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&path).unwrap();
        image::save_buffer(
            path.join(format!("{name}.png")),
            pixels,
            WIDTH as u32,
            HEIGHT as u32,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
}

#[test]
fn text_matches_direct_device_grid_raster_at_fractional_origins() {
    let mut gpu = GpuRenderer::new(WIDTH as u32, HEIGHT as u32).unwrap();
    // Use the bundled font, independent of installed host fonts or the parent app.
    let data = FontData::new(&include_bytes!("../../../assets/DejaVuSans.ttf")[..]).unwrap();
    data.install(&mut gpu.text_system().borrow_mut());
    gpu.set_background(Color(0, 0, 0, 255));
    let font = FontStyle {
        family: FontFamily::Named("DejaVu Sans".into()),
        line_height: LineHeight::px(18.9),
        ..Default::default()
    };
    for scale in [1., 1.25, 1.5, 2.] {
        gpu.set_scale_factor(scale);
        let mut scene = Scene::new(WIDTH as f32 / scale, HEIGHT as f32 / scale);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let node = scene.append(
            scene.root(),
            NodeKind::Text {
                text: Arc::from("Agyp 0123 · Native text"),
                color: Color(255, 255, 255, 255),
                font_size: 13.5,
            },
            Style {
                width: Some(220.),
                height: Some(40.),
                ..Default::default()
            },
        );
        scene.set_font(node, font.clone());
        gpu.install_text(&mut scene);
        for (step, (x, y)) in [(10.3, 6.7), (10.6, 7.1), (18.6, 11.1)]
            .into_iter()
            .enumerate()
        {
            scene.set_transform(node, Transform { x, y });
            let report = scene.flush();
            let stats = gpu.render(&scene, &report.damage).unwrap();
            if step > 0 {
                assert_eq!(
                    stats.shaped_nodes, 0,
                    "position changes must not reshape text"
                );
            }
            let actual = gpu.readback().unwrap();
            let bounds = scene.bounds(node);
            let mut expected = vec![0u8; WIDTH * HEIGHT * 4];
            for pixel in expected.chunks_mut(4) {
                pixel[3] = 255;
            }
            let system = gpu.text_system();
            let mut fonts = system.borrow_mut();
            let mut buffer = Buffer::new(&mut fonts, Metrics::new(13.5, 18.9));
            buffer.set_size(Some(220.), Some(40.));
            buffer.set_text(
                "Agyp 0123 · Native text",
                &Attrs::new().family(Family::Name("DejaVu Sans")),
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(&mut fonts, false);
            let mut swash = SwashCache::new();
            for run in buffer.layout_runs() {
                for glyph in run.glyphs {
                    // Snap the logical line baseline before DPI scaling. Keep
                    // mark offsets and the final node origin's subpixel phase.
                    let flags = glyph.physical((0., 0.), scale).cache_key.flags;
                    let (key, baseline_x, baseline_y) = CacheKey::new(
                        glyph.font_id,
                        glyph.glyph_id,
                        glyph.font_size * scale,
                        (
                            (glyph.x + glyph.font_size * glyph.x_offset) * scale + bounds.x * scale,
                            (glyph.y - glyph.font_size * glyph.y_offset + run.line_y.round())
                                * scale
                                + bounds.y * scale,
                        ),
                        glyph.font_weight,
                        flags,
                    );
                    let Some(image) = swash.get_image_uncached(&mut fonts, key) else {
                        continue;
                    };
                    assert_eq!(
                        image.data.len(),
                        image.placement.width as usize * image.placement.height as usize
                    );
                    for gy in 0..image.placement.height as usize {
                        for gx in 0..image.placement.width as usize {
                            let x = baseline_x + image.placement.left + gx as i32;
                            let y = baseline_y - image.placement.top + gy as i32;
                            if x < 0 || y < 0 || x >= WIDTH as i32 || y >= HEIGHT as i32 {
                                continue;
                            }
                            let a = image.data[gy * image.placement.width as usize + gx] as u32;
                            let dst = &mut expected[(y as usize * WIDTH + x as usize) * 4..][..3];
                            for c in dst {
                                *c = (a + (u32::from(*c) * (255 - a) + 127) / 255) as u8;
                            }
                        }
                    }
                }
            }
            let worst = actual
                .iter()
                .zip(&expected)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            let mae = actual
                .iter()
                .zip(&expected)
                .map(|(a, b)| a.abs_diff(*b) as f64)
                .sum::<f64>()
                / actual.len() as f64;
            println!("text scale={scale} origin=({x},{y}) max={worst} MAE={mae:.5}");
            output(&format!("text-{scale}-{step}"), &actual);
            assert!(
                worst <= 2,
                "glyphs must reach the physical grid without interpolation: {worst}"
            );
        }
    }
}

#[test]
fn svg_matches_direct_device_grid_raster_without_ceil_size_resampling() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14M5 12h14"/><circle cx="12" cy="12" r="9"/></svg>"##;
    let svg = Arc::new(SvgData::new(&source[..]).unwrap());
    let tree = resvg::usvg::Tree::from_data(source, &resvg::usvg::Options::default()).unwrap();
    let mut gpu = GpuRenderer::new(WIDTH as u32, HEIGHT as u32).unwrap();
    gpu.set_background(Color(0, 0, 0, 255));
    for scale in [1., 1.25, 1.5, 2.] {
        gpu.set_scale_factor(scale);
        let mut scene = Scene::new(WIDTH as f32 / scale, HEIGHT as f32 / scale);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let node = scene.append(
            scene.root(),
            NodeKind::Svg(svg.clone()),
            Style {
                width: Some(13.),
                height: Some(13.),
                ..Default::default()
            },
        );
        scene.set_transform(node, Transform { x: 10.3, y: 6.7 });
        let report = scene.flush();
        gpu.render(&scene, &report.damage).unwrap();
        let actual = gpu.readback().unwrap();
        let mut pixmap = resvg::tiny_skia::Pixmap::new(WIDTH as u32, HEIGHT as u32).unwrap();
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_row(
                13. * scale / 24.,
                0.,
                0.,
                13. * scale / 24.,
                10.3 * scale,
                6.7 * scale,
            ),
            &mut pixmap.as_mut(),
        );
        let mut expected = pixmap.take();
        for p in expected.chunks_mut(4) {
            p[3] = 255;
        }
        let worst = actual
            .iter()
            .zip(&expected)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        let mae = actual
            .iter()
            .zip(&expected)
            .map(|(a, b)| a.abs_diff(*b) as f64)
            .sum::<f64>()
            / actual.len() as f64;
        println!("icon scale={scale} max={worst} MAE={mae:.5}");
        output(&format!("icon-{scale}"), &actual);
        assert!(
            worst <= 2,
            "SVG raster must preserve exact device size and origin: {worst}"
        );
    }
}

#[test]
fn isolated_popup_matches_direct_text_icons_and_shadow_at_device_grid() {
    fn scene(scale: f32, isolated: bool) -> (Scene, NodeId) {
        let mut scene = Scene::new(WIDTH as f32 / scale, HEIGHT as f32 / scale);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let group = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(220.),
                height: Some(30.),
                ..Default::default()
            },
        );
        scene.set_isolated(group, isolated);
        scene.append(
            group,
            NodeKind::Quad(QuadStyle {
                fill: Color(30, 30, 28, 255),
                radius: 8.,
                shadow: Some(BoxShadow {
                    color: Color(180, 180, 180, 90),
                    offset: Transform { x: 0., y: 2. },
                    blur_radius: 3.,
                    spread: 1.,
                }),
                ..Default::default()
            }),
            Style {
                width: Some(220.),
                height: Some(30.),
                ..Default::default()
            },
        );
        let text = scene.append(
            group,
            NodeKind::Text {
                text: Arc::from("Agyp · Popup text"),
                font_size: 13.5,
                color: Color(240, 240, 235, 255),
            },
            Style {
                width: Some(175.),
                height: Some(25.),
                ..Default::default()
            },
        );
        scene.set_font(
            text,
            FontStyle {
                family: FontFamily::Named("DejaVu Sans".into()),
                line_height: LineHeight::px(18.9),
                ..Default::default()
            },
        );
        scene.set_transform(text, Transform { x: 6.3, y: 5.1 });
        let svg=Arc::new(SvgData::new(&br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" fill="none" stroke="white" stroke-width="2"><path d="M4 12h16M12 4v16"/></svg>"##[..]).unwrap());
        let icon = scene.append(
            group,
            NodeKind::Svg(svg),
            Style {
                width: Some(13.),
                height: Some(13.),
                ..Default::default()
            },
        );
        scene.set_transform(icon, Transform { x: 196.1, y: 8.7 });
        (scene, group)
    }
    let mut actual = GpuRenderer::new(WIDTH as u32, HEIGHT as u32).unwrap();
    FontData::new(&include_bytes!("../../../assets/DejaVuSans.ttf")[..])
        .unwrap()
        .install(&mut actual.text_system().borrow_mut());
    let mut reference =
        GpuRenderer::new_with_context(WIDTH as u32, HEIGHT as u32, &actual.context()).unwrap();
    for scale in [1., 1.25, 1.5, 2.] {
        actual.set_scale_factor(scale);
        reference.set_scale_factor(scale);
        let (mut layered, group) = scene(scale, true);
        let (mut direct, other) = scene(scale, false);
        actual.install_text(&mut layered);
        reference.install_text(&mut direct);
        for (step, (x, y)) in [
            (10.3, 10.7),
            (10.6, 11.1),
            (10.6 + 8. / scale, 11.1 + 4. / scale),
        ]
        .into_iter()
        .enumerate()
        {
            layered.set_transform(group, Transform { x, y });
            direct.set_transform(other, Transform { x, y });
            let report = layered.flush();
            let stats = actual.render(&layered, &report.damage).unwrap();
            if step == 2 {
                assert_eq!(stats.layer_repaints, 0);
                assert_eq!(stats.layer_cache_hits, 1);
            }
            direct.flush();
            reference
                .render(&direct, &[direct.bounds(direct.root())])
                .unwrap();
            let (pixels, expected) = (actual.readback().unwrap(), reference.readback().unwrap());
            let worst = pixels
                .iter()
                .zip(&expected)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            let mae = pixels
                .iter()
                .zip(&expected)
                .map(|(a, b)| a.abs_diff(*b) as f64)
                .sum::<f64>()
                / pixels.len() as f64;
            println!("popup scale={scale} origin=({x},{y}) max={worst} MAE={mae:.5}");
            output(&format!("popup-{scale}-{step}"), &pixels);
            assert!(
                worst <= 2,
                "isolated popup must preserve text, icons and shadow bounds: {worst}"
            );
        }
    }
}
