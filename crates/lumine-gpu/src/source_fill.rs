//! Explicit Chromium 154 / Skia CPU N32 constant-color contract, not a
//! replacement for arbitrary GPU blending, layers, HDR, or antialiased edges.
//! Kept dependency-free so the independently captured source corpus can be
//! checked without initializing a GPU or rebuilding the renderer.
pub(crate) fn over_opaque(source: [u8; 4], destination: [u8; 3]) -> [u8; 3] {
    let alpha = u32::from(source[3]);
    std::array::from_fn(|c| {
        let premul = (u32::from(source[c]) * alpha + 127) / 255;
        (premul + ((u32::from(destination[c]) * (256 - alpha)) >> 8)) as u8
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
