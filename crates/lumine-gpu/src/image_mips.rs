//! Lazy mipmaps for minified decoded images, in the renderer's premultiplied UNORM space.
use lumine::{
    affine::Affine,
    image::{ImageData, ImageSampling},
    scene::Rect,
};

pub(super) fn needed(image: &ImageData, bounds: Rect, transform: Affine, scale: f32) -> bool {
    if image.sampling() != ImageSampling::Linear
        || image.procedural().is_some()
        || image.effect_chain().is_some()
        || image.width().max(image.height()) == 1
        || bounds.width <= 0.
        || bounds.height <= 0.
    {
        return false;
    }
    let Some(inverse) = image.transform().then(transform).inverse() else {
        return false;
    };
    let u = image.width() as f32 / (bounds.width * scale);
    let v = image.height() as f32 / (bounds.height * scale);
    // Match the texture-coordinate derivatives used for implicit GPU LOD.
    (u * inverse.a)
        .hypot(v * inverse.b)
        .max((u * inverse.c).hypot(v * inverse.d))
        > 1.
}

pub(super) fn bytes(mut width: u32, mut height: u32, levels: u32) -> usize {
    let mut bytes = 0;
    for _ in 0..levels {
        bytes += width as usize * height as usize * 4;
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    bytes
}

/// Integrate source texel areas, including fractional edge texels for odd sizes.
/// Filter RGB after premultiplication so hidden transparent colors cannot bleed.
pub(super) fn halve(pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
    let (next_width, next_height) = ((width / 2).max(1), (height / 2).max(1));
    let mut next = vec![0; next_width as usize * next_height as usize * 4];
    // Integer overlap weights avoid losing an odd source row/column.
    for y in 0..next_height {
        let (top, bottom) = (y * height, (y + 1) * height);
        for x in 0..next_width {
            let (left, right) = (x * width, (x + 1) * width);
            let mut sum = [0_u64; 4];
            for sy in top / next_height..bottom.div_ceil(next_height) {
                let wy = bottom.min((sy + 1) * next_height) - top.max(sy * next_height);
                for sx in left / next_width..right.div_ceil(next_width) {
                    let wx = right.min((sx + 1) * next_width) - left.max(sx * next_width);
                    let offset = (sy as usize * width as usize + sx as usize) * 4;
                    for channel in 0..4 {
                        sum[channel] +=
                            u64::from(pixels[offset + channel]) * u64::from(wx) * u64::from(wy);
                    }
                }
            }
            let area = u64::from(width) * u64::from(height);
            let offset = (y as usize * next_width as usize + x as usize) * 4;
            for channel in 0..4 {
                next[offset + channel] = ((sum[channel] + area / 2) / area) as u8;
            }
        }
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GpuContext, GpuRenderer};
    use lumine::scene::{Color, Layout, NodeId, NodeKind, Scene, Style};
    use std::sync::Arc;

    fn fixed(size: f32) -> Style {
        Style {
            width: Some(size),
            height: Some(size),
            ..Default::default()
        }
    }

    fn pattern(size: u32) -> Arc<ImageData> {
        let mut pixels = Vec::new();
        for y in 0..size {
            for x in 0..size {
                pixels.extend([
                    ((x * 73 + y * 151 + x * y * 19) % 256) as u8,
                    ((x + y) % 2 * 255) as u8,
                    ((x * 41 + y * 17) % 256) as u8,
                    255,
                ]);
            }
        }
        Arc::new(ImageData::new(size, size, pixels).unwrap())
    }

    // Exercise direct writes on every adapter, and mapped render uploads when
    // the shared device actually enabled the required mapping feature.
    fn renderers(size: u32) -> Vec<GpuRenderer> {
        let context = GpuContext::new().unwrap();
        let mut direct = GpuRenderer::new_with_context(size, size, &context).unwrap();
        direct.mapped = None;
        let mut renderers = vec![direct];
        if context.inner.mapped_uploads {
            renderers.push(GpuRenderer::new_with_context(size, size, &context).unwrap());
        }
        renderers
    }

    fn scene(image: Arc<ImageData>, size: f32) -> (Scene, NodeId) {
        let mut scene = Scene::new(size, size);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let node = scene.append(scene.root(), NodeKind::Image(image), fixed(size));
        (scene, node)
    }

    fn paint(gpu: &mut GpuRenderer, scene: &mut Scene) -> (usize, Vec<u8>) {
        let damage = scene.flush().damage;
        let uploads = gpu.render(scene, &damage).unwrap().image_uploads;
        (uploads, gpu.readback().unwrap())
    }

    #[test]
    fn linear_minification_matches_area_reference() {
        let image = pattern(128);
        let original = image.pixels().to_vec();
        // Direct 16x16 source-box integration, independent of mip generation.
        let mut expected = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                for c in 0..4 {
                    let mut sum = 0_u32;
                    for sy in y * 16..(y + 1) * 16 {
                        for sx in x * 16..(x + 1) * 16 {
                            sum += u32::from(original[(sy * 128 + sx) * 4 + c]);
                        }
                    }
                    expected.push(((sum + 128) / 256) as u8);
                }
            }
        }
        for mut gpu in renderers(8) {
            // Physical source footprints remain identical at different pixel ratios.
            for scale in [1., 2.] {
                gpu.set_scale_factor(scale);
                let (mut scene, _) = scene(image.clone(), 8. / scale);
                let (_, actual) = paint(&mut gpu, &mut scene);
                // Four rounded mip reductions contribute at most 2 UNORM steps;
                // allow one more for GPU filtering/output quantization.
                for (actual, expected) in actual.iter().zip(&expected) {
                    assert!(
                        actual.abs_diff(*expected) <= 3,
                        "{actual} versus {expected}"
                    );
                }
            }
        }
        assert_eq!(image.pixels(), original);
    }

    #[test]
    fn nearest_then_linear_upgrades_once_and_updates_refresh_levels() {
        let image = pattern(128);
        for mut gpu in renderers(8) {
            let (mut scene, node) = scene(Arc::new(image.sampled(ImageSampling::Nearest)), 8.);
            let (uploads, actual) = paint(&mut gpu, &mut scene);
            assert_eq!(uploads, 1);
            assert_eq!(gpu.image_textures[&image.id()].mip_level_count(), 1);
            for y in 0..8 {
                for x in 0..8 {
                    let offset = (((y * 16 + 8) * 128 + x * 16 + 8) * 4) as usize;
                    assert_eq!(
                        &actual[(y * 8 + x) * 4..][..4],
                        &image.pixels()[offset..][..4]
                    );
                }
            }
            scene.set_kind(node, NodeKind::Image(image.clone()));
            assert_eq!(paint(&mut gpu, &mut scene).0, 1);
            assert!(gpu.image_textures[&image.id()].mip_level_count() > 1);
            // A different sampling variant shares the upgraded texture but
            // nearest must continue loading source level zero.
            scene.set_kind(
                node,
                NodeKind::Image(Arc::new(image.sampled(ImageSampling::Nearest))),
            );
            let (uploads, nearest_again) = paint(&mut gpu, &mut scene);
            assert_eq!(uploads, 0);
            assert_eq!(nearest_again, actual);
            let replacement =
                Arc::new(ImageData::new(128, 128, [17, 83, 149, 255].repeat(128 * 128)).unwrap());
            scene.set_kind(node, NodeKind::Image(replacement));
            let (uploads, updated) = paint(&mut gpu, &mut scene);
            assert_eq!(uploads, 1);
            assert!(updated.chunks_exact(4).all(|p| p == [17, 83, 149, 255]));
        }
    }

    #[test]
    fn nonminified_pixels_survive_lazy_upgrade_and_budget_counts_levels() {
        let image = pattern(16);
        for mut gpu in renderers(16) {
            let (mut scene, node) = scene(image.clone(), 16.);
            let (_, original) = paint(&mut gpu, &mut scene);
            assert_eq!(original, image.pixels());
            let base_bytes = gpu.debug_cache_stats().image_bytes;
            assert_eq!(base_bytes, image.pixels().len());
            gpu.image_budget = base_bytes;
            assert!(gpu.upload_image_levels(&image, true).is_err());
            assert_eq!(gpu.image_textures[&image.id()].mip_level_count(), 1);
            scene.set_style(node, fixed(2.));
            // Admission uses the additional allocation when upgrading an existing ID.
            gpu.image_budget = bytes(16, 16, 5);
            assert_eq!(paint(&mut gpu, &mut scene).0, 1);
            assert_eq!(gpu.debug_cache_stats().image_bytes, gpu.image_budget);
            scene.set_style(node, fixed(16.));
            let (uploads, enlarged) = paint(&mut gpu, &mut scene);
            assert_eq!(uploads, 0);
            assert_eq!(enlarged, original);
        }
    }

    #[test]
    fn minified_transparent_edges_do_not_bleed_hidden_rgb() {
        let mut pixels = Vec::new();
        for _y in 0..64 {
            for x in 0..64 {
                pixels.extend(if x % 2 == 0 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 0]
                });
            }
        }
        let image = Arc::new(ImageData::new(64, 64, pixels).unwrap());
        for mut gpu in renderers(4) {
            gpu.set_background(Color(0, 0, 0, 0));
            let (mut scene, _) = scene(image.clone(), 4.);
            let (_, actual) = paint(&mut gpu, &mut scene);
            // Premultiplied red at half coverage, including the image's edges.
            for pixel in actual.chunks_exact(4) {
                assert!(
                    pixel[0].abs_diff(128) <= 1 && pixel[3].abs_diff(128) <= 1,
                    "{pixel:?}"
                );
                assert_eq!(&pixel[1..3], &[0, 0]);
            }
        }
    }

    #[test]
    fn odd_box_reduction_includes_edge_texels_and_narrow_images() {
        let mut pixels = vec![0; 5 * 3 * 4];
        for y in 0..3 {
            pixels[(y * 5 + 4) * 4..][..4].copy_from_slice(&[255; 4]);
        }
        assert_eq!(halve(&pixels, 5, 3), [0, 0, 0, 0, 102, 102, 102, 102]);
        assert_eq!(
            halve(&[0, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 0], 1, 3),
            [85; 4]
        );
    }

    #[test]
    fn odd_sized_gpu_minification_keeps_last_source_column() {
        let mut pixels = Vec::new();
        for _y in 0..3 {
            for x in 0..5 {
                pixels.extend([if x == 4 { 255 } else { 0 }, 0, 0, 255]);
            }
        }
        let image = Arc::new(ImageData::new(5, 3, pixels).unwrap());
        for mut gpu in renderers(1) {
            let (mut scene, _) = scene(image.clone(), 1.);
            let (_, actual) = paint(&mut gpu, &mut scene);
            // The last column occupies exactly one fifth of the source area.
            assert!(actual[0].abs_diff(51) <= 1, "{actual:?}");
            assert_eq!(&actual[1..], &[0, 0, 255]);
        }
    }

    #[test]
    fn canvas_uploads_remain_single_level() {
        use lumine::canvas::{Canvas, Path};
        for mut gpu in renderers(8) {
            let mut scene = Scene::new(8., 8.);
            let mut canvas = Canvas::new();
            canvas.fill(
                Path::rectangle(Rect::new(0., 0., 8., 8.)),
                Color(17, 83, 149, 255),
            );
            scene.append(scene.root(), NodeKind::Canvas(Arc::new(canvas)), fixed(8.));
            let (_, actual) = paint(&mut gpu, &mut scene);
            assert!(actual.chunks_exact(4).all(|p| p == [17, 83, 149, 255]));
            assert!(
                gpu.image_textures
                    .values()
                    .all(|texture| texture.mip_level_count() == 1)
            );
        }
    }

    #[test]
    fn eligibility_uses_device_ratio_and_combined_affine_derivatives() {
        let image = pattern(16);
        let bounds = Rect::new(0., 0., 8., 8.);
        assert!(!needed(&image, bounds, Affine::IDENTITY, 2.));
        assert!(needed(&image, bounds, Affine::IDENTITY, 1.));
        assert!(!needed(&image, bounds, Affine::scale(2., 2.), 1.));
        assert!(needed(
            &image.transformed(Affine::scale(0.5, 1.)),
            bounds,
            Affine::IDENTITY,
            2.
        ));
        assert!(!needed(
            &image.sampled(ImageSampling::Nearest),
            bounds,
            Affine::IDENTITY,
            1.
        ));
    }
}
