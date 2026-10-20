//! Bounded A8 text-shadow masks. Offsets remain paint coordinates, never shaping input.
//! Sigma is physical pixels. Zero outside the glyph is intentional (never edge-clamp).
pub(super) const MAX_PIXELS: usize = 1_048_576;
pub(super) struct Mask {
    pub width: u32,
    pub height: u32,
    pub border: i32,
    pub alpha: Vec<u8>,
}
fn kernel(sigma: f32) -> Result<Vec<f64>, &'static str> {
    if !sigma.is_finite() || !(0. ..=64.).contains(&sigma) {
        return Err("text shadow physical sigma exceeds 0..=64px");
    }
    if sigma < 1. / 3. {
        return Ok(vec![1.]);
    }
    let sigma = sigma as f64;
    if sigma < 2. {
        // Discrete Gaussian (Lindeberg), matching Skia's small-mask support policy.
        let variance = sigma * sigma;
        let mut basis = Vec::new();
        for n in 0..=5usize {
            let mut term =
                (variance / 2.).powi(n as i32) / (1..=n).product::<usize>().max(1) as f64;
            let mut sum = term;
            for k in 1..64 {
                term *= variance * variance / (4. * k as f64 * (k + n) as f64);
                sum += term;
                if term < 1e-6 {
                    break;
                }
            }
            basis.push(sum / variance.exp());
        }
        let n = (1..basis.len()).find(|&n| basis[n] <= 0.01).unwrap_or(5);
        basis.truncate(n);
        let sum = basis[0] + 2. * basis[1..].iter().sum::<f64>();
        for v in &mut basis {
            *v /= sum
        }
        basis[0] = 1. - 2. * basis[1..].iter().sum::<f64>();
        let radius = basis.len() - 1;
        return Ok((0..=radius * 2)
            .map(|i| basis[i.abs_diff(radius)])
            .collect());
    }
    // CSS's three-box Gaussian approximation. Keep intermediate sums unrounded;
    // quantize only after each complete axis, as the mask filter does.
    let window = (sigma * 3. * (2. * std::f64::consts::PI).sqrt() / 4. + 0.5)
        .floor()
        .max(1.) as usize;
    let mut weights = vec![1.];
    for width in [
        window,
        window,
        if window % 2 == 0 { window + 1 } else { window },
    ] {
        let mut next = vec![0.; weights.len() + width - 1];
        for (i, v) in weights.iter().enumerate() {
            for out in &mut next[i..i + width] {
                *out += v / width as f64
            }
        }
        weights = next
    }
    Ok(weights)
}
pub(super) fn blur(
    alpha: &[u8],
    width: u32,
    height: u32,
    sigma: f32,
) -> Result<Mask, &'static str> {
    if width == 0 || height == 0 || width as usize * height as usize != alpha.len() {
        return Err("invalid text shadow mask dimensions");
    }
    let weights = kernel(sigma)?;
    let border = weights.len() / 2;
    let w = width as usize + border * 2;
    let h = height as usize + border * 2;
    if w > 2046 || h > 2046 || w.checked_mul(h).is_none_or(|n| n > MAX_PIXELS) {
        return Err("text shadow mask exceeds 2046px or 1Mi pixels");
    }
    if w.checked_mul(h)
        .and_then(|n| n.checked_mul(weights.len()))
        .is_none_or(|n| n > 64_000_000)
    {
        return Err("text shadow convolution exceeds 64M sample budget");
    }
    let small = sigma >= 1. / 3. && sigma < 2.;
    let window = (sigma as f64 * 3. * (2. * std::f64::consts::PI).sqrt() / 4. + 0.5)
        .floor()
        .max(1.) as u64;
    let divisor = window * window * if window % 2 == 0 { window + 1 } else { window };
    let coefficients: Vec<u64> = weights
        .iter()
        .map(|v| {
            if small {
                (v * 65536.).round() as u64
            } else {
                (v * divisor as f64).round() as u64
            }
        })
        .collect();
    let scale = (4294967296. / divisor as f64).round() as u64;
    let axis = |source: &[u8], sw: usize, sh: usize, horizontal: bool| {
        let (dw, dh) = if horizontal {
            (sw + border * 2, sh)
        } else {
            (sw, sh + border * 2)
        };
        let mut output = vec![0; dw * dh];
        for y in 0..dh {
            for x in 0..dw {
                let mut sum = 0u64;
                for (k, coefficient) in coefficients.iter().enumerate() {
                    let source_index = if horizontal {
                        x.checked_sub(k).filter(|&sx| sx < sw).map(|sx| y * sw + sx)
                    } else {
                        y.checked_sub(k).filter(|&sy| sy < sh).map(|sy| sy * sw + x)
                    };
                    if let Some(i) = source_index {
                        let product = source[i] as u64 * coefficient;
                        sum += if small { product >> 8 } else { product };
                    }
                }
                output[y * dw + x] = if sigma < 1. / 3. {
                    sum as u8
                } else if small {
                    ((sum + 128) >> 8).min(255) as u8
                } else {
                    ((scale * sum + (1 << 31)) >> 32).min(255) as u8
                };
            }
        }
        (output, dw, dh)
    };
    // Skia's small A8 filter uses 8.8 contributions, vertical then horizontal;
    // its three-box filter uses 32-bit reciprocal rounding, horizontal then vertical.
    let (first, fw, fh) = axis(alpha, width as usize, height as usize, !small);
    let (out, _, _) = axis(&first, fw, fh, small);
    Ok(Mask {
        width: w as u32,
        height: h as u32,
        border: border as i32,
        alpha: out,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn impulse_symmetry_and_zero_border() {
        for sigma in [0., 0.2, 0.5, 1., 1.9, 2., 4., 32., 64.] {
            let a = blur(&[255], 1, 1, sigma).unwrap();
            assert_eq!(a.width, a.height);
            assert_eq!(a.alpha, a.alpha.iter().copied().rev().collect::<Vec<_>>());
            assert!((kernel(sigma).unwrap().iter().sum::<f64>() - 1.).abs() < 1e-12);
            assert_eq!(
                blur(&[0], 1, 1, sigma).unwrap().alpha,
                vec![0; a.alpha.len()]
            );
        }
    }
    #[test]
    fn zero_and_bounds() {
        assert_eq!(blur(&[0, 255, 0], 3, 1, 0.).unwrap().alpha, [0, 255, 0]);
        assert!(blur(&[255], 1, 1, f32::NAN).is_err());
        assert!(blur(&[255], 1, 1, 65.).is_err());
        assert!(blur(&[0; 2047], 2047, 1, 0.).is_err());
    }
}
