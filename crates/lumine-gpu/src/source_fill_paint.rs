//! Conservative adapter from actual prepared draw order to a source CPU fill.
//! Unproven pixels retain the normal shader/blend; no tolerance or mask changes.
use super::{Draw, NO_MASK, Quad, rect_outside, rgba, source_fill};
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
