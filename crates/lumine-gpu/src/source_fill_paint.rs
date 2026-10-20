//! Source-pinned native analytic fills over an actual proven opaque backdrop.
//! Unsupported paint retains the existing path; no tolerance or mask changes.
use super::{Draw, NO_MASK, Quad, rect_outside, rgba, source_fill, source_skia154};
use lumine::{
    affine::Affine,
    scene::{Color, NodeKind, PaintItem, Rect},
};
use rustc_hash::FxHashMap;

#[derive(Clone, Copy)]
pub(super) struct OpaqueCore {
    rect: Rect,
    rgb: [u8; 3],
}
fn array(r: Rect) -> [f32; 4] {
    [r.x, r.y, r.width, r.height]
}
fn rect(r: [f32; 4]) -> Rect {
    Rect::new(r[0], r[1], r[2], r[3])
}
fn contains(a: Rect, b: Rect) -> bool {
    source_fill::contains(array(a), array(b))
}
fn bytes(color: [f32; 4]) -> Option<[u8; 4]> {
    let mut bytes = [0; 4];
    for (byte, value) in bytes.iter_mut().zip(color) {
        if !value.is_finite() || !(0. ..=1.).contains(&value) {
            return None;
        }
        *byte = (value * 255.).round() as u8;
        if f32::from(*byte) / 255. != value {
            return None;
        }
    }
    Some(bytes)
}
fn eligible(item: &PaintItem<'_>) -> bool {
    !item.isolated
        && item.transform == Affine::IDENTITY
        && item.effects.opacity == 1.
        && item.effects.blur_radius == 0.
        && item.effects.edge_fade == 0.
        && item.mask.is_none()
        && matches!(
            item.kind,
            NodeKind::Rect(_) | NodeKind::Quad(_) | NodeKind::Panel { .. }
        )
}
fn core(q: Quad, item: &PaintItem<'_>, clip: Rect, scale: f32) -> Option<Rect> {
    if !eligible(item)
        || q.mask != NO_MASK
        || q.options[0] != 0.
        || q.options[1] != 0.
        || q.options[2] != 0.
        || q.shape[2] != 0.
        || !q
            .rect
            .into_iter()
            .chain(q.fade)
            .chain(q.shape)
            .chain(q.border)
            .all(f32::is_finite)
    {
        return None;
    }
    let region = if q.shape[3] > 0.5 {
        let radius = if q.options[3] > 0.5 {
            q.border.into_iter().fold(0_f32, f32::max)
        } else {
            q.shape[0].max(0.)
        };
        let inset = radius + q.shape[1].max(0.) + (1. / scale).max(0.5);
        rect(source_fill::pixel_core(q.fade, inset, scale)?)
    } else {
        rect(source_fill::pixel_core(q.rect, 0., scale)?)
    };
    let region = region.intersection(rect(q.rect))?.intersection(clip)?;
    let region = rect(source_fill::pixel_core(array(region), 0., scale)?);
    // Full coverage of every ancestor clip, including rounded overflow clips.
    // Nontranslation maps and one-axis clips are deliberately unresolved.
    for parent in item.clip_regions.iter() {
        let inverse = parent.inverse;
        if inverse.a != 1.
            || inverse.d != 1.
            || inverse.b != 0.
            || inverse.c != 0.
            || parent.axes != [true, true]
            || !parent.corners.iter().all(|r| r.is_finite() && *r >= 0.)
        {
            return None;
        }
        let radius = parent.corners.into_iter().fold(0_f32, f32::max);
        let inset = if radius > 0. {
            radius + (1. / scale).max(0.5)
        } else {
            0.
        };
        let full = rect(source_fill::pixel_core(array(parent.bounds), inset, scale)?);
        let local = Rect::new(
            region.x + inverse.tx,
            region.y + inverse.ty,
            region.width,
            region.height,
        );
        if !contains(full, local) {
            return None;
        }
    }
    Some(region)
}

fn backdrop(
    region: Rect,
    draws: &[Draw],
    quads: &[Quad],
    known: &FxHashMap<u32, OpaqueCore>,
    background: Color,
    viewport: Rect,
) -> Option<[u8; 3]> {
    for draw in draws.iter().rev() {
        // A filter can sample beyond its primitive, so use its entire scissor.
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
            if rect(q.rect).intersection(region).is_none() {
                continue;
            }
            // Even a partial overlap blocks a uniform backdrop; do not infer
            // invisibility, fill holes, or combine colors selected by native fit.
            return known
                .get(&index)
                .filter(|k| contains(k.rect, region))
                .map(|k| k.rgb);
        }
    }
    (background.3 == 255 && contains(viewport, region)).then_some([
        background.0,
        background.1,
        background.2,
    ])
}

pub(super) fn resolve_interior(
    quads: &mut Vec<Quad>,
    start: u32,
    draws: &[Draw],
    known: &FxHashMap<u32, OpaqueCore>,
    item: &PaintItem<'_>,
    clip: Rect,
    scale: f32,
    background: Color,
    viewport: Rect,
) {
    if let Some(resolved) = resolve_analytic_opaque(
        quads, start, draws, known, item, clip, scale, background, viewport,
    ) {
        quads.truncate(start as usize);
        quads.extend(resolved);
        return;
    }
    if quads.len() != start as usize + 1 {
        return;
    }
    let q = quads[start as usize];
    let Some(source) = bytes(q.color).filter(|c| c[3] > 0 && c[3] < 255) else {
        return;
    };
    let Some(region) = core(q, item, clip, scale) else {
        return;
    };
    let Some(destination) = backdrop(region, draws, quads, known, background, viewport) else {
        return;
    };
    let rgb = source_fill::over_opaque(source, destination);
    quads[start as usize] = Quad {
        rect: array(region),
        color: rgba(Color(rgb[0], rgb[1], rgb[2], 255), 1.),
        options: [0.; 4],
        shape: [0.; 4],
        border: [0.; 4],
        ..q
    };
    for rim in rect_outside(rect(q.rect), region) {
        quads.push(Quad {
            rect: array(rim),
            ..q
        });
    }
}

/// Source-pinned full coverage AND analytic rims over an independently known
/// opaque destination. Every emitted quad replaces an entire device-pixel run;
/// source coverage and integer blit dispatch have already been evaluated.
/// Unknown/transparent destinations, layers and partial rounded clips remain
/// explicit unsupported cases on the unchanged fallback path.
fn resolve_analytic_opaque(
    quads: &[Quad],
    start: u32,
    draws: &[Draw],
    known: &FxHashMap<u32, OpaqueCore>,
    item: &PaintItem<'_>,
    clip: Rect,
    scale: f32,
    background: Color,
    viewport: Rect,
) -> Option<Vec<Quad>> {
    if quads.len() != start as usize + 1 || !eligible(item) || !scale.is_finite() || scale <= 0. {
        return None;
    }
    let q = quads[start as usize];
    if q.mask != NO_MASK
        || q.options[0] != 0.
        || q.options[1] != 0.
        || q.options[2] != 0.
        || q.shape[1] != 0.
        || q.shape[2] != 0.
    {
        return None;
    }
    let source = bytes(q.color)?;
    if source[3] == 0 {
        return None;
    }
    let radius = if q.shape[3] > 0.5 {
        if q.options[3] > 0.5 {
            if !q.border.into_iter().all(|radius| radius == q.border[0]) {
                return None;
            }
            q.border[0]
        } else {
            q.shape[0]
        }
    } else {
        0.
    };
    let source_bounds = if q.shape[3] > 0.5 { q.fade } else { q.rect };
    if !source_bounds
        .into_iter()
        .chain([radius])
        .all(f32::is_finite)
    {
        return None;
    }
    // Pinned Blink PixelSnappedContouredBorder -> ToPixelSnappedRect rounds
    // physical endpoints. It changes paint coverage, never layout allocation.
    let left = (source_bounds[0] * scale + 0.5).floor();
    let top = (source_bounds[1] * scale + 0.5).floor();
    let right = ((source_bounds[0] + source_bounds[2]) * scale + 0.5).floor();
    let bottom = ((source_bounds[1] + source_bounds[3]) * scale + 0.5).floor();
    let physical = [left, top, right - left, bottom - top];
    let painted = Rect::new(
        left / scale,
        top / scale,
        (right - left) / scale,
        (bottom - top) / scale,
    );
    if !contains(clip, painted) || !contains(viewport, painted) {
        return None;
    }
    for parent in item.clip_regions.iter() {
        let inverse = parent.inverse;
        if inverse.a != 1.
            || inverse.d != 1.
            || inverse.b != 0.
            || inverse.c != 0.
            || parent.axes != [true, true]
        {
            return None;
        }
        if !parent
            .corners
            .into_iter()
            .all(|radius| radius.is_finite() && radius >= 0.)
        {
            return None;
        }
        let radius = parent.corners.into_iter().fold(0_f32, f32::max);
        let inset = if radius > 0. {
            radius + (1. / scale).max(0.5)
        } else {
            0.
        };
        let full = rect(source_fill::pixel_core(array(parent.bounds), inset, scale)?);
        if !contains(
            full,
            Rect::new(
                painted.x + inverse.tx,
                painted.y + inverse.ty,
                painted.width,
                painted.height,
            ),
        ) {
            return None;
        }
    }
    let spans = source_skia154::spans(
        physical,
        radius.max(0.) * scale,
        [
            (viewport.width * scale).round() as u32,
            (viewport.height * scale).round() as u32,
        ],
    )?;
    let mut resolved: Vec<Quad> = Vec::new();
    let mut append = |x: i32, y: i32, width: i32, rgb: [u8; 3]| {
        let region = [
            x as f32 / scale,
            y as f32 / scale,
            width as f32 / scale,
            1. / scale,
        ];
        let color = rgba(Color(rgb[0], rgb[1], rgb[2], 255), 1.);
        if let Some(last) = resolved.last_mut() {
            if last.color == color
                && last.rect[1] == region[1]
                && last.rect[0] + last.rect[2] == region[0]
            {
                last.rect[2] += region[2];
                return;
            }
        }
        resolved.push(Quad {
            rect: region,
            uv: [0.; 4],
            color,
            fade: [0.; 4],
            options: [0.; 4],
            shape: [0.; 4],
            border: [0.; 4],
            mask: NO_MASK,
        });
    };
    for span in spans.iter() {
        let region = Rect::new(
            span.x as f32 / scale,
            span.y as f32 / scale,
            span.width as f32 / scale,
            1. / scale,
        );
        if let Some(destination) = backdrop(region, draws, quads, known, background, viewport) {
            append(
                span.x,
                span.y,
                span.width,
                source_fill::over_opaque_coverage(
                    source,
                    destination,
                    span.coverage as u8,
                    span.blend == 1,
                ),
            );
        } else {
            // Earlier paint can change within a row. Each pixel independently
            // needs an actual ordered destination proof; no filled-in holes.
            for x in span.x..span.x + span.width {
                let region = Rect::new(
                    x as f32 / scale,
                    span.y as f32 / scale,
                    1. / scale,
                    1. / scale,
                );
                let destination = backdrop(region, draws, quads, known, background, viewport)?;
                append(
                    x,
                    span.y,
                    1,
                    source_fill::over_opaque_coverage(
                        source,
                        destination,
                        span.coverage as u8,
                        span.blend == 1,
                    ),
                );
            }
        }
    }
    drop(append);
    (resolved.len() <= 65536).then_some(resolved)
}

pub(super) fn remember_opaque(
    quads: &[Quad],
    start: u32,
    item: &PaintItem<'_>,
    clip: Rect,
    scale: f32,
    known: &mut FxHashMap<u32, OpaqueCore>,
) {
    for index in start..quads.len() as u32 {
        let q = quads[index as usize];
        let Some(color) = bytes(q.color).filter(|c| c[3] == 255) else {
            continue;
        };
        let Some(rect) = core(q, item, clip, scale) else {
            continue;
        };
        known.insert(
            index,
            OpaqueCore {
                rect,
                rgb: [color[0], color[1], color[2]],
            },
        );
    }
}
