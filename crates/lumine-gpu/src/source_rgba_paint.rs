//! Separately selected source N32 composition with known ordered RGBA targets.
//! No target readback, expected image, unknown native alpha, or policy grant.
use super::{Draw, NO_MASK, Quad, source_fill, source_fill_paint};
use lumine::{
    affine::Affine,
    scene::{Color, PaintItem, Rect},
};
use rustc_hash::FxHashMap;

// Private Quad representation: UI nodes/styles cannot request an unblended
// write. Only this successful source resolver constructs these pixel runs.
const RESOLVED: f32 = -16.;
pub(super) fn resolved(q: &Quad) -> bool {
    q.options == [0., 0., 0., RESOLVED]
        && q.shape == [0.; 4]
        && q.border == [0.; 4]
        && q.mask == NO_MASK
        && q.uv == [0.; 4]
}
pub(super) struct PremultipliedCore {
    rect: Rect,
    rgba: [u8; 4],
}
fn contains(a: Rect, b: Rect) -> bool {
    source_fill::contains([a.x, a.y, a.width, a.height], [b.x, b.y, b.width, b.height])
}
fn backdrop(
    region: Rect,
    draws: &[Draw],
    quads: &[Quad],
    known: &FxHashMap<u32, PremultipliedCore>,
    opaque: &FxHashMap<u32, source_fill_paint::OpaqueCore>,
    background: Color,
    viewport: Rect,
) -> Option<[u8; 4]> {
    for draw in draws.iter().rev() {
        let support = if draw.blur.is_some() {
            draw.clip
        } else {
            draw.bounds
        };
        if support.intersection(region).is_none() || draw.clip.intersection(region).is_none() {
            continue;
        }
        if draw.layer.is_some()
            || draw.blur.is_some()
            || draw.image.is_some()
            || draw.transform != Affine::IDENTITY
        {
            return None;
        }
        for index in (draw.start..draw.end).rev() {
            let q = quads[index as usize];
            let rect = Rect::new(q.rect[0], q.rect[1], q.rect[2], q.rect[3]);
            if rect.intersection(region).is_none() {
                continue;
            }
            if let Some(core) = known.get(&index).filter(|c| contains(c.rect, region)) {
                return Some(core.rgba);
            }
            return opaque
                .get(&index)
                .filter(|c| contains(c.rect, region))
                .map(|c| [c.rgb[0], c.rgb[1], c.rgb[2], 255]);
        }
    }
    if !contains(viewport, region) {
        return None;
    }
    match background.3 {
        0 => Some([0; 4]),
        255 => Some([background.0, background.1, background.2, 255]),
        _ => None,
    }
}

pub(super) fn resolve(
    quads: &mut Vec<Quad>,
    start: u32,
    draws: &[Draw],
    known: &mut FxHashMap<u32, PremultipliedCore>,
    opaque: &FxHashMap<u32, source_fill_paint::OpaqueCore>,
    item: &PaintItem<'_>,
    clip: Rect,
    scale: f32,
    background: Color,
    viewport: Rect,
) -> Option<()> {
    if quads.len() != start as usize + 1 {
        return None;
    }
    let prepared = source_fill_paint::prepare_premultiplied(
        quads[start as usize],
        item,
        clip,
        scale,
        viewport,
    )?;
    // An unblended output must carry no remaining fade or clip transform.
    // Every geometric ancestor was checked by shared source preparation.
    if item.fade_transform != Affine::IDENTITY {
        return None;
    }
    let mut output: Vec<(Quad, [u8; 4])> = Vec::new();
    for span in prepared.spans.iter() {
        for x in span.x..span.x + span.width {
            let (coverage, two_pixels) = prepared.coverage(span, x);
            if coverage == 0 {
                continue;
            }
            let region = Rect::new(
                x as f32 / scale,
                span.y as f32 / scale,
                1. / scale,
                1. / scale,
            );
            let destination = backdrop(region, draws, quads, known, opaque, background, viewport)?;
            let rgba = source_fill::over_premultiplied_coverage(
                prepared.source,
                destination,
                coverage,
                two_pixels,
            );
            if rgba[..3].iter().any(|channel| *channel > rgba[3]) {
                return None;
            }
            if let Some((last, previous)) = output.last_mut() {
                if *previous == rgba
                    && last.rect[1] == region.y
                    && last.rect[0] + last.rect[2] == region.x
                {
                    last.rect[2] += region.width;
                    continue;
                }
            }
            if output.len() >= 65536 {
                return None;
            }
            let alpha = f32::from(rgba[3]);
            let color = if alpha == 0. {
                [0.; 4]
            } else {
                [
                    f32::from(rgba[0]) / alpha,
                    f32::from(rgba[1]) / alpha,
                    f32::from(rgba[2]) / alpha,
                    alpha / 255.,
                ]
            };
            output.push((
                Quad {
                    rect: [region.x, region.y, region.width, region.height],
                    color,
                    uv: [0.; 4],
                    fade: [0.; 4],
                    options: [0., 0., 0., RESOLVED],
                    shape: [0.; 4],
                    border: [0.; 4],
                    mask: NO_MASK,
                },
                rgba,
            ));
        }
    }
    let all_clips_resolved = prepared.clip_mask.is_some() || item.clip_regions.is_empty();
    // Full-coverage ancestor clips can remain geometrically present in the
    // old opaque path. Here all ancestors must be evaluated before copying.
    if !all_clips_resolved {
        return None;
    }
    quads.truncate(start as usize);
    for (quad, rgba) in output {
        known.insert(
            quads.len() as u32,
            PremultipliedCore {
                rect: Rect::new(quad.rect[0], quad.rect[1], quad.rect[2], quad.rect[3]),
                rgba,
            },
        );
        quads.push(quad);
    }
    Some(())
}
