//! Normalized Gaussian taps packed for bilinear sampling, recomputed only when
//! sigma changes. Pairing adjacent taps halves texture samples without changing
//! the discrete kernel, including its edge-clamped crop behavior.

/// Center first, then positive offsets and combined weights. The shader mirrors
/// every positive sample. Sigma matches the renderer's [0.1, 64] pixel range.
pub(super) fn samples(sigma: f32) -> Vec<[f32; 2]> {
    assert!(sigma.is_finite(), "Gaussian sigma must be finite");
    let sigma = sigma.clamp(0.1, 64.0);
    let radius = (sigma * 3.0).ceil() as usize;
    let sigma = f64::from(sigma);
    let weights: Vec<f64> = (0..=radius)
        .map(|i| (-((i * i) as f64) / (2.0 * sigma * sigma)).exp())
        .collect();
    let total = weights[0] + 2.0 * weights[1..].iter().sum::<f64>();
    let mut samples = Vec::with_capacity(1 + radius.div_ceil(2));
    samples.push([0.0, (weights[0] / total) as f32]);
    for i in (1..=radius).step_by(2) {
        let a = weights[i];
        let b = weights.get(i + 1).copied().unwrap_or(0.0);
        let weight = a + b;
        let offset = i as f64 + b / weight;
        samples.push([offset as f32, (weight / total) as f32]);
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(sigma: f32) -> Vec<f64> {
        let sigma = sigma.clamp(0.1, 64.0);
        let radius = (sigma * 3.0).ceil() as usize;
        let weights: Vec<_> = (0..=radius)
            .map(|i| (-((i * i) as f64) / (2.0 * f64::from(sigma).powi(2))).exp())
            .collect();
        let total = weights[0] + 2.0 * weights[1..].iter().sum::<f64>();
        weights.into_iter().map(|w| w / total).collect()
    }

    #[test]
    fn packed_taps_preserve_normalization_and_discrete_moments() {
        for sigma in [0.1, 0.333_333_34, 0.7, 1.0, 1.01, 2.5, 8.0, 32.0, 64.0] {
            let packed = samples(sigma);
            let reference = reference(sigma);
            assert_eq!(packed[0][0], 0.0);
            assert!(packed.len() <= 97);
            let total = f64::from(packed[0][1])
                + 2.0 * packed[1..].iter().map(|tap| f64::from(tap[1])).sum::<f64>();
            assert!((total - 1.0).abs() < 1e-7, "sigma={sigma}, total={total}");

            let mut first = 0.0;
            let mut second = 0.0;
            for &[offset, weight] in &packed[1..] {
                let offset = f64::from(offset);
                let weight = f64::from(weight);
                let lo = offset.floor();
                let fraction = offset - lo;
                first += weight * offset;
                // Bilinear interpolation mixes discrete texels, so its second
                // moment includes the two integer offsets, not offset² alone.
                second += weight * ((1.0 - fraction) * lo * lo + fraction * (lo + 1.0).powi(2));
            }
            let first_ref = reference
                .iter()
                .enumerate()
                .map(|(i, w)| i as f64 * w)
                .sum::<f64>();
            let second_ref = reference
                .iter()
                .enumerate()
                .map(|(i, w)| (i * i) as f64 * w)
                .sum::<f64>();
            assert!((first - first_ref).abs() <= first_ref.abs().max(1.0) * 1e-6);
            assert!((second - second_ref).abs() <= second_ref.abs().max(1.0) * 1e-6);
        }
    }

    #[test]
    fn paired_linear_samples_match_discrete_edge_clamped_convolution() {
        for len in [1, 2, 3, 7, 17, 65] {
            // Nonuniform data catches weighted-offset mistakes that a constant
            // image cannot. Include transparent/zero-like values at the ends.
            let pixels: Vec<_> = (0..len)
                .map(|i| ((i * 17 + i * i * 7) % 31) as f64 / 30.0)
                .collect();
            let at = |i: isize| pixels[i.clamp(0, len as isize - 1) as usize];
            let linear = |x: f64| {
                let x = x.clamp(0.0, (len - 1) as f64);
                let lo = x.floor() as isize;
                let fraction = x - lo as f64;
                at(lo) * (1.0 - fraction) + at(lo + 1) * fraction
            };
            for sigma in [0.1, 0.5, 1.0, 2.5, 8.0, 64.0] {
                let packed = samples(sigma);
                let weights = reference(sigma);
                for pixel in 0..len {
                    let discrete = weights[0] * at(pixel as isize)
                        + weights
                            .iter()
                            .enumerate()
                            .skip(1)
                            .map(|(i, w)| {
                                w * (at(pixel as isize - i as isize)
                                    + at(pixel as isize + i as isize))
                            })
                            .sum::<f64>();
                    let paired = f64::from(packed[0][1]) * at(pixel as isize)
                        + packed[1..]
                            .iter()
                            .map(|&[offset, weight]| {
                                f64::from(weight)
                                    * (linear(pixel as f64 - f64::from(offset))
                                        + linear(pixel as f64 + f64::from(offset)))
                            })
                            .sum::<f64>();
                    assert!(
                        (paired - discrete).abs() < 2e-6,
                        "len={len}, sigma={sigma}, pixel={pixel}: {paired} vs {discrete}"
                    );
                }
            }
        }
    }
}
