//! Explicit Chromium 154 / Skia CPU N32 constant-color and AA blit stages,
//! not a replacement for arbitrary GPU blending, layers, or HDR.
//! Kept dependency-free so the independently captured source corpus can be
//! checked without initializing a GPU or rebuilding the renderer.
pub(crate) fn over_opaque(source: [u8; 4], destination: [u8; 3]) -> [u8; 3] {
    let alpha = u32::from(source[3]);
    std::array::from_fn(|c| {
        let premul = (u32::from(source[c]) * alpha + 127) / 255;
        (premul + ((u32::from(destination[c]) * (256 - alpha)) >> 8)) as u8
    })
}

/// Original SkARGB32 blit dispatch. Coverage is applied after source premul;
/// its two-pixel specialization combines the terms before the final shift.
pub(crate) fn over_opaque_coverage(
    source: [u8; 4],
    destination: [u8; 3],
    coverage: u8,
    two_pixels: bool,
) -> [u8; 3] {
    let alpha = u32::from(source[3]);
    let scale = u32::from(coverage) + 1;
    std::array::from_fn(|channel| {
        let premul = (u32::from(source[channel]) * alpha + 127) / 255;
        let destination = u32::from(destination[channel]);
        let value = if !two_pixels {
            let scaled_alpha = (alpha * scale) >> 8;
            ((premul * scale) >> 8) + ((destination * (256 - scaled_alpha)) >> 8)
        } else if alpha == 255 {
            let scale = u32::from(coverage) + (u32::from(coverage) >> 7);
            (premul * scale + destination * (256 - scale)) >> 8
        } else {
            let product = 65535 - alpha * scale;
            let destination_scale = (product + (product >> 8)) >> 8;
            (premul * scale + destination * destination_scale) >> 8
        };
        value.min(255) as u8
    })
}

/// The same original N32 blit operations with all four premultiplied target
/// channels retained. Destination bytes must be independently known, never
/// reconstructed from an unknown native draw or read from expected pixels.
pub(crate) fn over_premultiplied_coverage(
    source: [u8; 4],
    destination: [u8; 4],
    coverage: u8,
    two_pixels: bool,
) -> [u8; 4] {
    let alpha = u32::from(source[3]);
    let scale = u32::from(coverage) + 1;
    std::array::from_fn(|channel| {
        let premul = if channel == 3 {
            alpha
        } else {
            (u32::from(source[channel]) * alpha + 127) / 255
        };
        let destination = u32::from(destination[channel]);
        let value = if !two_pixels {
            let scaled_alpha = (alpha * scale) >> 8;
            ((premul * scale) >> 8) + ((destination * (256 - scaled_alpha)) >> 8)
        } else if alpha == 255 {
            let scale = u32::from(coverage) + (u32::from(coverage) >> 7);
            (premul * scale + destination * (256 - scale)) >> 8
        } else {
            let product = 65535 - alpha * scale;
            let destination_scale = (product + (product >> 8)) >> 8;
            (premul * scale + destination * destination_scale) >> 8
        };
        value.min(255) as u8
    })
}

pub(crate) fn contains(outer: [f32; 4], inner: [f32; 4]) -> bool {
    outer.into_iter().chain(inner).all(f32::is_finite)
        && outer[2] > 0.
        && outer[3] > 0.
        && inner[2] > 0.
        && inner[3] > 0.
        && inner[0] >= outer[0]
        && inner[1] >= outer[1]
        && inner[0] + inner[2] <= outer[0] + outer[2]
        && inner[1] + inner[3] <= outer[1] + outer[3]
}

/// Entire device pixels, with no fractional boundary shared with the rim.
pub(crate) fn pixel_core(bounds: [f32; 4], inset: f32, scale: f32) -> Option<[f32; 4]> {
    if !bounds.into_iter().chain([inset, scale]).all(f32::is_finite) || scale <= 0. || inset < 0. {
        return None;
    }
    let left = ((bounds[0] + inset) * scale).ceil() / scale;
    let top = ((bounds[1] + inset) * scale).ceil() / scale;
    let right = ((bounds[0] + bounds[2] - inset) * scale).floor() / scale;
    let bottom = ((bounds[1] + bounds[3] - inset) * scale).floor() / scale;
    (right > left && bottom > top).then_some([left, top, right - left, bottom - top])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_source_rim_blit_stages_are_not_float_coverage() {
        // Actual original Chromium fixture, physical (20,16), radius8.
        // Source manifest 1f80c99abbcb9d129d8b03195dc89259c546f7b4327a7e5433c09aeeb5b491f5.
        assert_eq!(
            over_opaque_coverage([169, 190, 230, 20], [26, 26, 24], 56, false),
            [27, 28, 27]
        );
        assert_eq!(
            over_opaque_coverage([169, 190, 230, 255], [26, 26, 24], 56, false),
            [57, 62, 69]
        );
        // Skia's opaque two-pixel interpolation is a distinct dispatch.
        assert_ne!(
            over_opaque_coverage([169, 190, 230, 255], [26, 26, 24], 56, true),
            [57, 62, 69]
        );
    }
    #[test]
    fn captured_chromium_cpu_constant_color_corpus() {
        // Immutable actual browser outputs, recorded before this implementation.
        // Chromium 73c14f6 / Skia 2466dcf; source evidence manifest SHA256:
        // c8cc65ea978457b01abee21533bf690da2277cc57000915b0b688b1f361ff35f.
        let mut count = 0;
        for line in include_str!("testdata/chromium154-software-over-opaque.tsv").lines() {
            if line.starts_with('#') {
                continue;
            }
            let v: Vec<u8> = line
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            assert_eq!(v.len(), 10);
            assert_eq!(
                over_opaque([v[0], v[1], v[2], v[3]], [v[4], v[5], v[6]]),
                [v[7], v[8], v[9]],
                "{line}"
            );
            count += 1;
        }
        assert_eq!(count, 4608);
    }
    #[test]
    fn alpha_endpoints_and_destination_attenuation_are_distinct() {
        assert_eq!(over_opaque([169, 190, 230, 0], [26, 26, 24]), [26, 26, 24]);
        assert_eq!(
            over_opaque([169, 190, 230, 255], [26, 26, 24]),
            [169, 190, 230]
        );
        assert_eq!(over_opaque([169, 190, 230, 20], [26, 26, 24]), [36, 38, 40]);
        // Quantizing source premultiplication then normal hardware blend differs.
        assert_ne!(13. + 26. * (1. - 20. / 255.), 36.);
    }
    #[test]
    fn cores_snap_inward_and_reject_invalid_or_empty_geometry() {
        assert_eq!(
            pixel_core([0.1, 0.2, 20., 18.], 5., 2.),
            Some([5.5, 5.5, 9.5, 7.5])
        );
        assert!(pixel_core([0., 0., 8., 8.], 5., 2.).is_none());
        assert!(pixel_core([f32::NAN, 0., 20., 20.], 0., 2.).is_none());
        assert!(pixel_core([0., 0., 20., 20.], 0., 0.).is_none());
        assert!(!contains([0., 0., 10., 10.], [9., 9., 2., 2.]));
        assert!(!contains([0., 0., f32::INFINITY, 10.], [1., 1., 2., 2.]));
    }
}
