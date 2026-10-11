//! Detailed panel paint lowered into retained vector content.
use crate::GpuError;
use lumine::{
    canvas::*,
    decoration::{Background, BorderStyle, Corners},
    scene::{Insets, QuadStyle, Rect},
};
fn radii(c: Corners, w: f32, h: f32) -> [(f32, f32); 4] {
    let r = [
        c.top_left.max(0.),
        c.top_right.max(0.),
        c.bottom_right.max(0.),
        c.bottom_left.max(0.),
    ];
    let factor = [
        (w, r[0] + r[1]),
        (w, r[3] + r[2]),
        (h, r[0] + r[3]),
        (h, r[1] + r[2]),
    ]
    .into_iter()
    .fold(1_f32, |scale, (size, sum)| {
        if sum > 0. {
            scale.min(size / sum)
        } else {
            scale
        }
    })
    .clamp(0., 1.);
    r.map(|r| (r * factor, r * factor))
}
fn rounded(rect: Rect, r: [(f32, f32); 4]) -> Path {
    let (x, y, right, bottom) = (rect.x, rect.y, rect.x + rect.width, rect.y + rect.height);
    let k = 0.552_284_8;
    let mut p = Path::builder();
    p.move_to(x + r[0].0, y)
        .line_to(right - r[1].0, y)
        .cubic_to(
            Point::new(right - r[1].0 * (1. - k), y),
            Point::new(right, y + r[1].1 * (1. - k)),
            Point::new(right, y + r[1].1),
        )
        .line_to(right, bottom - r[2].1)
        .cubic_to(
            Point::new(right, bottom - r[2].1 * (1. - k)),
            Point::new(right - r[2].0 * (1. - k), bottom),
            Point::new(right - r[2].0, bottom),
        )
        .line_to(x + r[3].0, bottom)
        .cubic_to(
            Point::new(x + r[3].0 * (1. - k), bottom),
            Point::new(x, bottom - r[3].1 * (1. - k)),
            Point::new(x, bottom - r[3].1),
        )
        .line_to(x, y + r[0].1)
        .cubic_to(
            Point::new(x, y + r[0].1 * (1. - k)),
            Point::new(x + r[0].0 * (1. - k), y),
            Point::new(x + r[0].0, y),
        )
        .close();
    p.build().expect("finite normalized corners")
}
pub fn canvas(style: &QuadStyle, width: f32, height: f32) -> Result<Canvas, GpuError> {
    let Some(detail) = &style.decoration else {
        return Err(GpuError("missing detailed decoration".into()));
    };
    let corners = detail.corners.unwrap_or(Corners::all(style.radius));
    if width <= 0.
        || height <= 0.
        || ![
            width,
            height,
            corners.top_left,
            corners.top_right,
            corners.bottom_right,
            corners.bottom_left,
        ]
        .iter()
        .all(|v| v.is_finite())
    {
        return Err(GpuError("invalid decoration dimensions".into()));
    }
    let radius = radii(corners, width, height);
    let outer = rounded(Rect::new(0., 0., width, height), radius);
    let mut canvas = Canvas::new();
    let background = match &detail.background {
        None => Brush::Solid(style.fill),
        Some(Background::Brush(brush)) => brush.clone(),
        Some(Background::Linear { angle, stops }) => {
            if !angle.is_finite() {
                return Err(GpuError("nonfinite gradient angle".into()));
            }
            let (sin, cos) = angle.sin_cos();
            let half = (width * cos.abs() + height * sin.abs()) * 0.5;
            Brush::linear(
                Point::new(width * 0.5 - cos * half, height * 0.5 - sin * half),
                Point::new(width * 0.5 + cos * half, height * 0.5 + sin * half),
                stops.clone(),
            )
            .map_err(|e| GpuError(e.into()))?
        }
    };
    canvas.fill(outer.clone(), background);
    let borders = detail
        .border_widths
        .unwrap_or(Insets::all(style.border_width));
    if ![borders.left, borders.top, borders.right, borders.bottom]
        .iter()
        .all(|v| v.is_finite())
    {
        return Err(GpuError("nonfinite border width".into()));
    }
    let left = borders.left.clamp(0., width);
    let right = borders.right.clamp(0., width - left);
    let top = borders.top.clamp(0., height);
    let bottom = borders.bottom.clamp(0., height - top);
    let maximum = left.max(right).max(top).max(bottom);
    if maximum > 0. && style.border_color.3 > 0 {
        let mut ring = Path::builder();
        ring.append(&outer);
        let inner = Rect::new(
            left,
            top,
            (width - left - right).max(0.),
            (height - top - bottom).max(0.),
        );
        if inner.width > 0. && inner.height > 0. {
            ring.append(&rounded(
                inner,
                [
                    ((radius[0].0 - left).max(0.), (radius[0].1 - top).max(0.)),
                    ((radius[1].0 - right).max(0.), (radius[1].1 - top).max(0.)),
                    (
                        (radius[2].0 - right).max(0.),
                        (radius[2].1 - bottom).max(0.),
                    ),
                    ((radius[3].0 - left).max(0.), (radius[3].1 - bottom).max(0.)),
                ],
            ));
        }
        let ring = ring.build().unwrap();
        match detail.border_style {
            BorderStyle::Solid => {
                canvas.fill_rule(ring, style.border_color, FillRule::EvenOdd);
            }
            BorderStyle::Dashed { length, gap } => {
                if !length.is_finite() || !gap.is_finite() || length <= 0. || gap <= 0. {
                    return Err(GpuError("invalid dashed border".into()));
                }
                canvas
                    .clip(ring, FillRule::EvenOdd)
                    .stroke(
                        outer,
                        style.border_color,
                        Stroke {
                            width: maximum * 2.,
                            dash: vec![length, gap],
                            ..Default::default()
                        },
                    )
                    .restore();
            }
        }
    }
    Ok(canvas)
}

/// Solid panel edges and corners are independent of the stretchable center.
/// Preserve their device-pixel rasters while width/height animate. Gradients,
/// dashed borders and undersized boxes keep the full-size raster path.
pub(crate) fn stretch_grid(
    style: &QuadStyle,
    width: f32,
    height: f32,
    scale: f32,
) -> Option<[f32; 4]> {
    let detail = style.decoration.as_deref()?;
    if detail.border_style != BorderStyle::Solid
        || !matches!(
            &detail.background,
            None | Some(Background::Brush(Brush::Solid(_)))
        )
    {
        return None;
    }
    let c = detail.corners.unwrap_or(Corners::all(style.radius));
    let b = detail
        .border_widths
        .unwrap_or(Insets::all(style.border_width));
    let pad = |radius: f32, border: f32| ((radius.max(border).max(0.) * scale).ceil() + 1.) / scale;
    let cuts = [
        pad(c.top_left.max(c.bottom_left), b.left),
        pad(c.top_left.max(c.top_right), b.top),
        pad(c.top_right.max(c.bottom_right), b.right),
        pad(c.bottom_left.max(c.bottom_right), b.bottom),
    ];
    (cuts.iter().all(|v| v.is_finite())
        && width >= cuts[0] + cuts[2] + 1. / scale
        && height >= cuts[1] + cuts[3] + 1. / scale)
        .then_some(cuts)
}
/// Corner radius used by analytic outer shadows, with the same per-corner policy.
pub fn shadow_corners(style: &QuadStyle, width: f32, height: f32, spread: f32) -> [f32; 4] {
    let c = style
        .decoration
        .as_ref()
        .and_then(|d| d.corners)
        .unwrap_or(Corners::all(style.radius));
    radii(
        Corners {
            top_left: c.top_left + spread,
            top_right: c.top_right + spread,
            bottom_right: c.bottom_right + spread,
            bottom_left: c.bottom_left + spread,
        },
        width,
        height,
    )
    .map(|r| r.0)
}
