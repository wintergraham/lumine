use std::sync::Arc;
use lumine::{canvas::Canvas, scene::Scene, svg::SvgData};

#[test]
fn rgba_output_budget_is_checked_before_reading_oversized_rgb_pixels() {
    use image::ImageEncoder;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[1, 2, 3], 1, 1, image::ExtendedColorType::Rgb8)
        .unwrap();
    // Valid large RGB metadata fits the decoder's 64 MiB allocation cap; RGBA
    // expansion does not. The tiny original pixel stream must never be read.
    png[16..20].copy_from_slice(&6000u32.to_be_bytes());
    png[20..24].copy_from_slice(&3000u32.to_be_bytes());
    let mut crc = !0u32;
    for byte in &png[12..29] {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    png[29..33].copy_from_slice(&(!crc).to_be_bytes());
    assert_eq!(
        lumine_gpu::assets::decode_image(&png).unwrap_err().0,
        "decoded image exceeds 64 MiB"
    );
}

#[test]
fn detailed_decoration_rejects_nonpositive_dimensions_before_path_or_border_math() {
    let style = lumine::scene::QuadStyle {
        decoration: Some(Arc::new(lumine::decoration::Decoration::default())),
        ..Default::default()
    };
    for (width, height) in [(-1., 20.), (20., -1.), (0., 20.), (20., 0.)] {
        assert!(lumine_gpu::decoration::canvas(&style, width, height).is_err());
    }
}
#[test]
fn finite_extreme_raster_dimensions_return_errors_without_overflow_or_allocation() {
    let source = b"<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/>";
    assert!(lumine_gpu::assets::decode_svg(source, u32::MAX, u32::MAX).is_err());
    let canvas = Canvas::default();
    for (w, h, scale) in [
        (f32::MAX, f32::MAX, 1.),
        (f32::MAX, 100., f32::MAX),
        (1., f32::MAX, 1.),
    ] {
        assert!(lumine_gpu::canvas::rasterize(&canvas, w, h, scale).is_err());
        let mut cache = lumine_gpu::svg::SvgCache::default();
        let source = Arc::new(SvgData::new(&source[..]).unwrap());
        assert!(
            cache
                .get(Scene::new(1., 1.).root(), &source, w, h, scale)
                .is_err()
        );
        assert_eq!(cache.bytes(), 0);
        assert_eq!(cache.rasterizations(), 0);
    }
}
