//! Shared bounded vector raster cache. Rasterization only follows changed content,
//! allocated size or device scale; GPU upload uses ordinary retained image identity.
use crate::GpuError;
use lumine::{
    canvas::*,
    image::ImageData,
    scene::{NodeId, NodeKind, Scene},
};
use resvg::tiny_skia as sk;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};
const BUDGET: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 1024;
struct Entry {
    canvas: Weak<Canvas>,
    decoration: Option<lumine::scene::QuadStyle>,
    width: f32,
    height: f32,
    scale: f32,
    /// The raster's image id, which its GPU texture is keyed by.
    id: u64,
    /// Its pixels, until uploaded (see `CanvasCache::uploaded`).
    pixels: Option<Arc<ImageData>>,
    /// Its size in bytes, uploaded or not: what the budget counts.
    bytes: usize,
    used: u64,
}
/// A cached raster: its image id, and its pixels if not uploaded yet.
pub struct Raster {
    pub id: u64,
    pub pixels: Option<Arc<ImageData>>,
}
impl Entry {
    fn raster(&self) -> Raster {
        Raster {
            id: self.id,
            pixels: self.pixels.clone(),
        }
    }
}
#[derive(Default)]
pub struct CanvasCache {
    entries: HashMap<NodeId, Entry>,
    bytes: usize,
    clock: u64,
    rasterizations: u64,
}
impl CanvasCache {
    pub fn retain(&mut self, scene: &Scene) {
        self.entries.retain(|id, _| {
            scene.contains(*id)
                && match scene.kind(*id) {
                    NodeKind::Canvas(_) => true,
                    NodeKind::Quad(style) | NodeKind::Panel { quad: style, .. } => {
                        style.decoration.is_some()
                    }
                    _ => false,
                }
        });
        self.bytes = self.entries.values().map(|e| e.bytes).sum();
    }
    pub fn image_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.entries.values().map(|e| e.id)
    }
    /// `node`'s raster is on the GPU: its pixels need not stay in memory.
    pub fn uploaded(&mut self, node: NodeId) {
        if let Some(entry) = self.entries.get_mut(&node) {
            entry.pixels = None;
        }
    }
    /// Rasterize `node` again on its next request (its texture was lost).
    pub fn forget(&mut self, node: NodeId) {
        if let Some(entry) = self.entries.remove(&node) {
            self.bytes -= entry.bytes;
        }
    }
    pub fn rasterizations(&self) -> u64 {
        self.rasterizations
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn get_decoration(
        &mut self,
        node: NodeId,
        style: &lumine::scene::QuadStyle,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Result<Raster, GpuError> {
        if style.shadows().len() > 32 {
            return Err(GpuError("panel exceeds 32 shadows".into()));
        }
        self.clock = self.clock.wrapping_add(1);
        if let Some(entry) = self.entries.get_mut(&node)
            && entry.decoration.as_ref() == Some(style)
            && entry.width == width
            && entry.height == height
            && entry.scale == scale
        {
            entry.used = self.clock;
            return Ok(entry.raster());
        }
        let canvas = Arc::new(crate::decoration::canvas(style, width, height)?);
        let image = self.get(node, &canvas, width, height, scale)?;
        if let Some(entry) = self.entries.get_mut(&node) {
            entry.decoration = Some(style.clone());
        }
        Ok(image)
    }
    pub fn get(
        &mut self,
        node: NodeId,
        canvas: &Arc<Canvas>,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Result<Raster, GpuError> {
        self.clock = self.clock.wrapping_add(1);
        if let Some(entry) = self.entries.get_mut(&node)
            && entry
                .canvas
                .upgrade()
                .is_some_and(|old| Arc::ptr_eq(&old, canvas) || old == *canvas)
            && entry.width == width
            && entry.height == height
            && entry.scale == scale
        {
            entry.used = self.clock;
            return Ok(entry.raster());
        }
        let image = Arc::new(rasterize(canvas, width, height, scale)?);
        if let Some(old) = self.entries.remove(&node) {
            self.bytes -= old.bytes;
        }
        let bytes = image.pixels().len();
        while (self.bytes + bytes > BUDGET || self.entries.len() >= MAX_ENTRIES)
            && !self.entries.is_empty()
        {
            let victim = *self.entries.iter().min_by_key(|(_, e)| e.used).unwrap().0;
            self.bytes -= self.entries.remove(&victim).unwrap().bytes;
        }
        self.bytes += bytes;
        self.entries.insert(
            node,
            Entry {
                canvas: Arc::downgrade(canvas),
                decoration: None,
                width,
                height,
                scale,
                id: image.id(),
                pixels: Some(image.clone()),
                bytes,
                used: self.clock,
            },
        );
        self.rasterizations += 1;
        Ok(Raster {
            id: image.id(),
            pixels: Some(image),
        })
    }
}
fn color(color: lumine::scene::Color) -> sk::Color {
    sk::Color::from_rgba8(color.0, color.1, color.2, color.3)
}
fn point(point: Point) -> sk::Point {
    sk::Point::from_xy(point.x, point.y)
}
fn path(path: &Path) -> Option<sk::Path> {
    let mut builder = sk::PathBuilder::new();
    for segment in path.segments() {
        match *segment {
            Segment::Move(p) => builder.move_to(p.x, p.y),
            Segment::Line(p) => builder.line_to(p.x, p.y),
            Segment::Quadratic(a, b) => builder.quad_to(a.x, a.y, b.x, b.y),
            Segment::Cubic(a, b, c) => builder.cubic_to(a.x, a.y, b.x, b.y, c.x, c.y),
            Segment::Close => builder.close(),
        }
    }
    builder.finish()
}
pub fn rasterize(
    canvas: &Canvas,
    width: f32,
    height: f32,
    scale: f32,
) -> Result<ImageData, GpuError> {
    if ![width, height, scale]
        .iter()
        .all(|v| v.is_finite() && *v > 0.)
        || canvas.commands().len() > 65536
    {
        return Err(GpuError(
            "invalid canvas dimensions or command count".into(),
        ));
    }
    let w = (width * scale).ceil() as u32;
    let h = (height * scale).ceil() as u32;
    if w == 0
        || h == 0
        || (u64::from(w) * u64::from(h))
            .checked_mul(4)
            .is_none_or(|bytes| bytes > BUDGET as u64)
    {
        return Err(GpuError("canvas raster exceeds 32 MiB".into()));
    }
    let mut pixmap =
        sk::Pixmap::new(w, h).ok_or_else(|| GpuError("canvas allocation failed".into()))?;
    // Exact scaling to rounded texture extents avoids a fractional final-pixel seam.
    let transform = sk::Transform::from_scale(w as f32 / width, h as f32 / height);
    let mut masks: Vec<sk::Mask> = Vec::new();
    for command in canvas.commands() {
        match command {
            Draw::Clip { path: source, rule } => {
                if masks.len() >= 16
                    || (u64::from(w) * u64::from(h))
                        .checked_mul((masks.len() + 1) as u64)
                        .is_none_or(|bytes| bytes > BUDGET as u64)
                {
                    return Err(GpuError("canvas clip stack exceeds budget".into()));
                }
                let mut mask = masks.last().cloned().unwrap_or_else(|| {
                    let mut mask = sk::Mask::new(w, h).unwrap();
                    mask.data_mut().fill(255);
                    mask
                });
                if let Some(path) = path(source) {
                    mask.intersect_path(
                        &path,
                        match rule {
                            FillRule::Winding => sk::FillRule::Winding,
                            FillRule::EvenOdd => sk::FillRule::EvenOdd,
                        },
                        true,
                        transform,
                    );
                } else {
                    mask.clear();
                }
                masks.push(mask);
                continue;
            }
            Draw::Restore => {
                if masks.pop().is_none() {
                    return Err(GpuError("unbalanced canvas restore".into()));
                }
                continue;
            }
            _ => {}
        }
        let (source, brush) = match command {
            Draw::Fill { path, brush, .. } | Draw::Stroke { path, brush, .. } => (path, brush),
            _ => unreachable!(),
        };
        let Some(path) = path(source) else {
            continue;
        };
        let mut paint = sk::Paint::default();
        let tile;
        paint.shader = match brush {
            Brush::Solid(c) => sk::Shader::SolidColor(color(*c)),
            Brush::Linear { start, end, stops } => {
                if stops.len() < 2
                    || stops.len() > 1024
                    || stops
                        .iter()
                        .any(|s| !s.offset.is_finite() || !(0. ..=1.).contains(&s.offset))
                {
                    return Err(GpuError("invalid gradient stops".into()));
                }
                sk::LinearGradient::new(
                    point(*start),
                    point(*end),
                    stops
                        .iter()
                        .map(|s| sk::GradientStop::new(s.offset, color(s.color)))
                        .collect(),
                    sk::SpreadMode::Pad,
                    sk::Transform::identity(),
                )
                .ok_or_else(|| GpuError("invalid gradient geometry".into()))?
            }
            Brush::Radial {
                center,
                radii,
                stops,
            } => {
                // tiny-skia interpolates straight colors. Sample premultiplied stops
                // here so transparent CSS stops cannot introduce dark fringes.
                Brush::radial(*center, *radii, stops.clone())
                    .map_err(|error| GpuError(error.into()))?;
                let mut pixels = sk::Pixmap::new(w, h)
                    .ok_or_else(|| GpuError("radial allocation failed".into()))?;
                for y in 0..h {
                    for x in 0..w {
                        let dx = ((x as f32 + 0.5) * width / w as f32 - center.x) / radii.x;
                        let dy = ((y as f32 + 0.5) * height / h as f32 - center.y) / radii.y;
                        let offset = dx.hypot(dy);
                        let right = stops.partition_point(|stop| stop.offset <= offset);
                        let left = right.saturating_sub(1);
                        let a = &stops[left];
                        let b = &stops[right.min(stops.len() - 1)];
                        let mix = if right == 0 || right == stops.len() {
                            0.
                        } else {
                            (offset - a.offset) / (b.offset - a.offset)
                        };
                        let a = color(a.color).premultiply();
                        let b = color(b.color).premultiply();
                        let channel = |a: f32, b: f32| ((a + (b - a) * mix) * 255.).round() as u8;
                        let alpha = channel(a.alpha(), b.alpha());
                        pixels.pixels_mut()[(y * w + x) as usize] =
                            sk::PremultipliedColorU8::from_rgba(
                                channel(a.red(), b.red()).min(alpha),
                                channel(a.green(), b.green()).min(alpha),
                                channel(a.blue(), b.blue()).min(alpha),
                                alpha,
                            )
                            .expect("interpolation preserves premultiplication");
                    }
                }
                tile = pixels;
                sk::Pattern::new(
                    tile.as_ref(),
                    sk::SpreadMode::Pad,
                    sk::FilterQuality::Nearest,
                    1.,
                    sk::Transform::from_scale(width / w as f32, height / h as f32),
                )
            }
            Brush::Slash {
                background,
                foreground,
                spacing,
                width,
            } => {
                if !spacing.is_finite()
                    || !width.is_finite()
                    || *spacing < 1.
                    || *spacing > 1024.
                    || *width < 0.
                    || *width > *spacing
                {
                    return Err(GpuError("invalid slash pattern".into()));
                }
                let side = spacing.ceil() as u32;
                let mut pixels = sk::Pixmap::new(side, side).unwrap();
                for y in 0..side {
                    for x in 0..side {
                        let stripe = ((x + y) as f32 % side as f32) < width / spacing * side as f32;
                        pixels.pixels_mut()[(y * side + x) as usize] =
                            color(if stripe { *foreground } else { *background })
                                .premultiply()
                                .to_color_u8();
                    }
                }
                tile = pixels;
                sk::Pattern::new(
                    tile.as_ref(),
                    sk::SpreadMode::Repeat,
                    sk::FilterQuality::Bilinear,
                    1.,
                    sk::Transform::from_scale(spacing / side as f32, spacing / side as f32),
                )
            }
        };
        match command {
            Draw::Fill { rule, .. } => pixmap.fill_path(
                &path,
                &paint,
                match rule {
                    FillRule::Winding => sk::FillRule::Winding,
                    FillRule::EvenOdd => sk::FillRule::EvenOdd,
                },
                transform,
                masks.last(),
            ),
            Draw::Stroke { style, .. } => {
                if !style.width.is_finite()
                    || style.width <= 0.
                    || !style.miter_limit.is_finite()
                    || style.miter_limit < 1.
                    || !style.dash_offset.is_finite()
                    || style.dash.len() > 1024
                    || style.dash.iter().any(|d| !d.is_finite() || *d <= 0.)
                {
                    return Err(GpuError("invalid stroke style".into()));
                }
                let dash = if style.dash.is_empty() {
                    None
                } else {
                    Some(
                        sk::StrokeDash::new(style.dash.clone(), style.dash_offset)
                            .ok_or_else(|| GpuError("invalid dash pattern".into()))?,
                    )
                };
                let stroke = sk::Stroke {
                    width: style.width,
                    miter_limit: style.miter_limit,
                    line_cap: match style.cap {
                        LineCap::Butt => sk::LineCap::Butt,
                        LineCap::Round => sk::LineCap::Round,
                        LineCap::Square => sk::LineCap::Square,
                    },
                    line_join: match style.join {
                        LineJoin::Miter => sk::LineJoin::Miter,
                        LineJoin::Round => sk::LineJoin::Round,
                        LineJoin::Bevel => sk::LineJoin::Bevel,
                    },
                    dash,
                };
                pixmap.stroke_path(&path, &paint, &stroke, transform, masks.last());
            }
            _ => unreachable!(),
        }
    }
    if !masks.is_empty() {
        return Err(GpuError("unbalanced canvas clip".into()));
    }
    let mut pixels = pixmap.take();
    for p in pixels.as_chunks_mut::<4>().0 {
        let alpha = u32::from(p[3]);
        if alpha > 0 {
            for channel in &mut p[..3] {
                *channel = (u32::from(*channel) * 255 + alpha / 2)
                    .checked_div(alpha)
                    .unwrap_or(0)
                    .min(255) as u8;
            }
        }
    }
    ImageData::new(w, h, pixels).map_err(|e| GpuError(e.into()))
}

#[cfg(test)]
mod lifetime_tests {
    use super::*;
    use lumine::scene::{Color, QuadStyle, Rect};

    #[test]
    fn radial_raster_interpolates_premultiplied_colors_on_rectangular_ellipse() {
        let mut canvas = Canvas::new();
        canvas.fill(
            Path::rectangle(Rect::new(0., 0., 41., 21.)),
            Brush::radial(
                Point::new(20.5, 10.5),
                Point::new(20., 10.),
                vec![
                    GradientStop {
                        offset: 0.,
                        color: Color(255, 0, 0, 255),
                    },
                    GradientStop {
                        offset: 1.,
                        color: Color(0, 0, 255, 0),
                    },
                ],
            )
            .unwrap(),
        );
        let image = rasterize(&canvas, 41., 21., 1.).unwrap();
        let pixel = |x: usize, y: usize| &image.pixels()[4 * (y * 41 + x)..][..4];
        assert_eq!(pixel(20, 10), [255, 0, 0, 255]);
        assert_eq!(pixel(30, 10), [255, 0, 0, 128]);
        assert_eq!(pixel(20, 15), pixel(30, 10));
        assert_eq!(pixel(40, 10), [0, 0, 0, 0]);
    }

    #[test]
    fn radial_raster_preserves_transparent_hard_stops_and_source_over() {
        let mut canvas = Canvas::new();
        let rect = Path::rectangle(Rect::new(0., 0., 12., 2.));
        canvas.fill(rect.clone(), Color(255, 255, 255, 255));
        canvas.fill(
            rect,
            Brush::radial(
                Point::new(0.5, 0.5),
                Point::new(20., 20.),
                vec![
                    GradientStop {
                        offset: 0.4,
                        color: Color(0, 0, 0, 0),
                    },
                    GradientStop {
                        offset: 0.4,
                        color: Color(255, 0, 0, 255),
                    },
                    GradientStop {
                        offset: 0.45,
                        color: Color(255, 0, 0, 255),
                    },
                    GradientStop {
                        offset: 0.45,
                        color: Color(0, 0, 255, 255),
                    },
                ],
            )
            .unwrap(),
        );
        let image = rasterize(&canvas, 12., 2., 1.).unwrap();
        let pixel = |x: usize| &image.pixels()[4 * x..][..4];
        assert_eq!(pixel(7), [255, 255, 255, 255]);
        assert_eq!(pixel(8), [255, 0, 0, 255]);
        assert_eq!(pixel(9), [0, 0, 255, 255]);
        assert!(Brush::radial(Point::default(), Point::new(0., 1.), vec![]).is_err());
        assert!(Brush::radial(Point::default(), Point::new(f32::NAN, 1.), vec![]).is_err());
    }

    #[test]
    fn radial_decoration_retains_pixels_until_style_radius_or_bounds_change() {
        use lumine::decoration::{Background, Decoration};
        let mut scene = Scene::new(100., 100.);
        let mut style = QuadStyle::default();
        style.decoration = Some(Arc::new(Decoration {
            background: Some(Background::Brush(
                Brush::radial(
                    Point::new(20., 10.),
                    Point::new(20., 10.),
                    vec![
                        GradientStop {
                            offset: 0.,
                            color: Color(255, 0, 0, 255),
                        },
                        GradientStop {
                            offset: 1.,
                            color: Color(0, 0, 0, 0),
                        },
                    ],
                )
                .unwrap(),
            )),
            ..Default::default()
        }));
        let node = scene.append(
            scene.root(),
            NodeKind::Quad(style.clone()),
            Default::default(),
        );
        let mut cache = CanvasCache::default();
        let first = cache.get_decoration(node, &style, 40., 20., 1.).unwrap();
        cache.uploaded(node);
        let same = cache
            .get_decoration(node, &style.clone(), 40., 20., 1.)
            .unwrap();
        assert_eq!(first.id, same.id);
        assert!(same.pixels.is_none());
        assert_eq!(cache.rasterizations(), 1);
        style.radius = 4.;
        let rounded = cache.get_decoration(node, &style, 40., 20., 1.).unwrap();
        assert_ne!(first.id, rounded.id);
        let resized = cache.get_decoration(node, &style, 80., 20., 1.).unwrap();
        assert_ne!(rounded.id, resized.id);
        assert_eq!(cache.rasterizations(), 3);
    }

    #[test]
    fn removed_canvas_drops_commands_before_next_paint() {
        let mut scene = Scene::new(100., 100.);
        let mut canvas = Canvas::new();
        canvas.fill(
            Path::rectangle(lumine::scene::Rect::new(0., 0., 10., 10.)),
            lumine::scene::Color(255, 0, 0, 255),
        );
        let canvas = Arc::new(canvas);
        let weak = Arc::downgrade(&canvas);
        let node = scene.append(
            scene.root(),
            NodeKind::Canvas(canvas.clone()),
            Default::default(),
        );
        let mut cache = CanvasCache::default();
        let first = cache.get(node, &canvas, 20., 20., 1.).unwrap();
        cache.uploaded(node);
        let second = cache.get(node, &canvas, 20., 20., 1.).unwrap();
        assert_eq!(first.id, second.id);
        assert!(second.pixels.is_none(), "uploaded rasters keep no pixels");
        assert_eq!(cache.rasterizations(), 1);
        scene.remove(node);
        drop(canvas);
        assert!(weak.upgrade().is_none());
        assert!(
            cache.bytes() > 0,
            "test runs before next render retention pass"
        );
    }
    #[test]
    fn tiny_canvas_rasters_obey_entry_count_and_lru_limit() {
        let mut scene = Scene::new(100., 100.);
        let canvas = Arc::new(Canvas::new());
        let mut cache = CanvasCache::default();
        let mut first = None;
        for _ in 0..=MAX_ENTRIES {
            let node = scene.append(
                scene.root(),
                NodeKind::Canvas(canvas.clone()),
                Default::default(),
            );
            first.get_or_insert(node);
            cache.get(node, &canvas, 1., 1., 1.).unwrap();
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert_eq!(cache.bytes(), MAX_ENTRIES * 4);
        assert!(!cache.entries.contains_key(&first.unwrap()));
    }
}
