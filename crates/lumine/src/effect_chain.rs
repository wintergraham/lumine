//! Small ordered content filters. GPU renderers cache each unchanged prefix;
//! software consumers evaluate the same operations lazily in premultiplied RGBA.
use super::{ImageData, next_identity};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EffectStage {
    /// Gaussian standard deviation in image pixels, in (0, 64].
    Blur { radius: f32 },
    /// Ordered 4×4 Bayer quantization per color channel. Alpha is preserved.
    Dither { levels: u32, cell_size: u32 },
}
#[derive(Debug)]
pub struct FilteredImage {
    pub instance: u64,
    pub input: Arc<ImageData>,
    pub stages: Vec<EffectStage>,
}
/// Retain one instance per surface; order is explicit and snapshots immutable.
pub struct EffectChain {
    id: u64,
    last: Option<Arc<ImageData>>,
}
impl Default for EffectChain {
    fn default() -> Self {
        Self::new()
    }
}
impl EffectChain {
    pub fn new() -> Self {
        Self {
            id: next_identity().expect("effect identity exhausted"),
            last: None,
        }
    }
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn render(
        &mut self,
        input: Arc<ImageData>,
        stages: &[EffectStage],
    ) -> Result<Arc<ImageData>, &'static str> {
        if stages.is_empty() {
            return Ok(input);
        }
        let mut total = stages.len();
        let mut ancestor = input.as_ref();
        loop {
            if total > 8 {
                return Err(
                    "image effects support at most eight ordered stages including filtered inputs",
                );
            }
            let Some(chain) = ancestor.effect_chain() else {
                break;
            };
            total += chain.stages.len();
            ancestor = chain.input.as_ref();
        }
        for stage in stages {
            match *stage {
                EffectStage::Blur { radius }
                    if !radius.is_finite() || radius <= 0.0 || radius > 64.0 =>
                {
                    return Err("image blur radius must be finite and in (0, 64]");
                }
                EffectStage::Dither { levels, cell_size }
                    if !(2..=256).contains(&levels) || !(1..=4096).contains(&cell_size) =>
                {
                    return Err("dither requires 2..=256 levels and 1..=4096 pixel cells");
                }
                _ => {}
            }
        }
        if let Some(last) = &self.last {
            let chain = last.chain.as_ref().unwrap();
            if chain.input == input && chain.stages == stages {
                return Ok(last.clone());
            }
        }
        let image = Arc::new(ImageData {
            id: next_identity()?,
            width: input.width,
            height: input.height,
            pixels: Arc::new(OnceLock::new()),
            procedural: None,
            transform: input.transform,
            sampling: input.sampling,
            chain: Some(Arc::new(FilteredImage {
                instance: self.id,
                input,
                stages: stages.to_vec(),
            })),
        });
        self.last = Some(image.clone());
        Ok(image)
    }
}

pub(super) fn fallback(chain: &FilteredImage) -> Arc<[u8]> {
    let width = chain.input.width() as usize;
    let height = chain.input.height() as usize;
    let mut pixels = chain.input.pixels().to_vec();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    const BAYER: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
    for stage in &chain.stages {
        match *stage {
            EffectStage::Blur { radius: sigma } => {
                let sigma = sigma.max(0.1);
                let radius = (sigma * 3.).ceil() as isize;
                let mut weights: Vec<f32> = (-radius..=radius)
                    .map(|x| (-(x * x) as f32 / (2. * sigma * sigma)).exp())
                    .collect();
                let sum: f32 = weights.iter().sum();
                for weight in &mut weights {
                    *weight /= sum;
                }
                for horizontal in [true, false] {
                    let mut out = vec![0; pixels.len()];
                    for y in 0..height {
                        for x in 0..width {
                            let mut value = [0.; 4];
                            for (offset, weight) in (-radius..=radius).zip(&weights) {
                                let sx = if horizontal {
                                    (x as isize + offset).clamp(0, width as isize - 1) as usize
                                } else {
                                    x
                                };
                                let sy = if horizontal {
                                    y
                                } else {
                                    (y as isize + offset).clamp(0, height as isize - 1) as usize
                                };
                                for c in 0..4 {
                                    value[c] += pixels[(sy * width + sx) * 4 + c] as f32 * weight;
                                }
                            }
                            for c in 0..4 {
                                out[(y * width + x) * 4 + c] =
                                    value[c].round().clamp(0., 255.) as u8;
                            }
                        }
                    }
                    pixels = out;
                }
            }
            EffectStage::Dither { levels, cell_size } => {
                let steps = (levels - 1) as f32;
                for (index, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let x = index % width / cell_size as usize % 4;
                    let y = index / width / cell_size as usize % 4;
                    let threshold = (BAYER[y * 4 + x] as f32 + 0.5) / 16.;
                    let alpha = pixel[3] as f32;
                    for channel in &mut pixel[..3] {
                        let straight = if alpha > 0. {
                            (*channel as f32 / alpha).clamp(0., 1.)
                        } else {
                            0.
                        };
                        *channel = ((straight * steps + threshold).floor() / steps * alpha)
                            .round()
                            .clamp(0., 255.) as u8;
                    }
                }
            }
        }
    }
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = (u32::from(*channel) * 255 + alpha / 2)
                .checked_div(alpha)
                .unwrap_or(0)
                .min(255) as u8;
        }
    }
    pixels.into()
}
