//! Separately selected source N32 composition with known ordered RGBA targets.
//! No target readback, expected image, unknown native alpha, or policy grant.
use super::{Draw, NO_MASK, Quad, source_fill, source_fill_paint};
use lumine::{
    affine::Affine,
    scene::{Color, Effects, NodeKind, PaintItem, QuadStyle, Rect, Scene},
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

/// Pinned Chromium folds only a complete single supported drawing operation
/// between SaveLayerAlpha and Restore. Enumerate the actual layer closure;
/// a descendant, source boundary, clip, decoration or other operation rejects
/// this optimization. Arbitrary opacity groups remain on the native path.
pub(super) fn resolve_singleton_opacity(
    scene: &Scene,
    item: &PaintItem<'_>,
    quads: &mut Vec<Quad>,
    start: u32,
    draws: &[Draw],
    known: &mut FxHashMap<u32, PremultipliedCore>,
    opaque: &FxHashMap<u32, source_fill_paint::OpaqueCore>,
    clip: Rect,
    scale: f32,
    background: Color,
    viewport: Rect,
) -> Option<()> {
    if !scene.children(item.id).is_empty()
        || item.source_paint_capability.is_none()
        || item.transform != Affine::IDENTITY
        || item.fade_transform != Affine::IDENTITY
        || item.mask.is_some()
        || !item.clip_regions.is_empty()
        || !item.effects.opacity.is_finite()
        || item.effects.opacity <= 0.
        || item.effects.opacity == 1.
        || item.effects.opacity > 1.
        || item.effects.blur_radius != 0.
        || item.effects.edge_fade != 0.
        || (quads.len() != start as usize && quads.len() != start as usize + 1)
    {
        return None;
    }
    if scene
        .ancestors(item.id)
        .any(|parent| scene.effects(parent) != Effects::default() || scene.is_isolated(parent))
    {
        return None;
    }
    let style = scene.style(item.id);
    if style.clip
        || style.clip_insets.is_some()
        || style.fade_edges != [0.; 2]
        || style.layout_options.is_some()
    {
        return None;
    }
    let closure = scene.layer_items(Some(item.id));
    if closure.len() != 1 {
        return None;
    }
    let inner = &closure[0];
    if inner.id != item.id
        || inner.isolated
        || inner.bounds != item.bounds
        || inner.source_paint_capability != item.source_paint_capability
        || inner.effects != Effects::default()
        || inner.transform != Affine::IDENTITY
        || !inner.clip_regions.is_empty()
        || inner.mask.is_some()
        || inner.fade_transform != Affine::IDENTITY
    {
        return None;
    }
    let (color, radius) = match inner.kind {
        NodeKind::Rect(color) => (*color, 0.),
        NodeKind::Quad(quad) | NodeKind::Panel { quad, .. } => {
            let plain = QuadStyle {
                fill: quad.fill,
                radius: quad.radius,
                ..Default::default()
            };
            if *quad != plain {
                return None;
            }
            (quad.fill, quad.radius)
        }
        _ => return None,
    };
    // ScopedRasterFlags folds into SkPaint alpha; SkARGB32_Blitter::ctor
    // gets the quantized SkColor before SkPreMultiplyColor. Group pixels are
    // never individually multiplied by this alpha.
    let alpha = ((f32::from(color.3) / 255.) * item.effects.opacity * 255.).round() as u8;
    let folded = Color(color.0, color.1, color.2, alpha);
    let bounds = [
        inner.bounds.x,
        inner.bounds.y,
        inner.bounds.width,
        inner.bounds.height,
    ];
    let original = quads[start as usize..].to_vec();
    quads.truncate(start as usize);
    quads.push(Quad {
        rect: bounds,
        fade: bounds,
        color: super::rgba(folded, 1.),
        uv: [0.; 4],
        options: [0.; 4],
        shape: [radius, 0., 0., 1.],
        border: [0.; 4],
        mask: NO_MASK,
    });
    if resolve(
        quads, start, draws, known, opaque, inner, clip, scale, background, viewport,
    )
    .is_some()
    {
        Some(())
    } else {
        quads.truncate(start as usize);
        quads.extend(original);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolved_transparent_rgba_keeps_copy_semantics_through_batching() {
        use super::super::{Shading, draw_diagnostics, shading};
        let rect = Rect::new(0., 0., 96., 80.);
        let quad = Quad {
            rect: [0., 0., 96., 80.],
            uv: [0.; 4],
            color: [26. / 38., 27. / 38., 35. / 38., 38. / 255.],
            fade: [0.; 4],
            options: [0., 0., 0., RESOLVED],
            shape: [0.; 4],
            border: [0.; 4],
            mask: NO_MASK,
        };
        let selected = shading(&quad, 2.);
        assert_eq!(selected, Shading::PremultipliedCopy);
        let draw = Draw {
            diagnostic_source: None,
            transform: Affine::IDENTITY,
            geometry_offset: 0,
            geometry_hash: 0,
            geometry: None,
            shading: selected,
            blur: None,
            blur_mask: None,
            start: 0,
            end: 1,
            clip: rect,
            bounds: rect,
            image: None,
            layer: None,
        };
        for unblended in [false, true] {
            for merged in [false, true] {
                let planned = draw_diagnostics::batches(
                    std::slice::from_ref(&draw),
                    rect,
                    0,
                    unblended,
                    merged,
                )
                .next()
                .unwrap();
                assert_eq!(planned.shading, Shading::PremultipliedCopy);
            }
        }
    }
}
