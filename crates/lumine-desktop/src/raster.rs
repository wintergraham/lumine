use fontdue::{Font, FontSettings, Metrics};
use std::collections::HashMap;
use lumine::scene::{Color, Effects, NodeKind, Rect, Scene};

type StyledGlyphKey = (String, u32, u32, u32, lumine::text_layout::FontStyle);
type RichGlyphImages = (
    Vec<(i32, i32, Color, cosmic_text::SwashImage)>,
    Vec<lumine_gpu::text::RichDecoration>,
);
type RichGlyphCache = Vec<(
    std::sync::Arc<lumine::rich_text::RichText>,
    u32,
    u32,
    RichGlyphImages,
)>;
type StyledGlyphImages = Vec<(i32, i32, cosmic_text::SwashImage)>;
const STYLED_GLYPH_BUDGET: usize = 8 * 1024 * 1024;
fn styled_bytes(key: &StyledGlyphKey, images: &StyledGlyphImages) -> usize {
    key.0.capacity()
        + key.4.storage_bytes()
        + images.capacity() * std::mem::size_of::<(i32, i32, cosmic_text::SwashImage)>()
        + images
            .iter()
            .map(|(_, _, image)| image.data.capacity())
            .sum::<usize>()
}

const AFFINE_MATTE_BUDGET: usize = 64 * 1024 * 1024;
struct AffineMatte {
    revision: u64,
    ink: Rect,
    edge_fade: f32,
    width: usize,
    height: usize,
    pixels: Vec<[u8; 4]>, // Premultiplied RGBA, linear interpolation before unpremultiply.
}
struct FilteredBackdrop {
    x: usize,
    y: usize,
    width: usize,
    pixels: Vec<u32>,
}
impl FilteredBackdrop {
    fn pixel(&self, x: usize, y: usize) -> u32 {
        self.pixels[(y - self.y) * self.width + x - self.x]
    }
}
/// Persistent CPU backing store with cached glyph masks and damage-only rasterization.
/// Glyph cache is bounded; this backend intentionally favors a small portable implementation.
pub struct Raster {
    pub pixels: Vec<u32>,
    width: usize,
    height: usize,
    font: Font,
    styled_fonts: std::rc::Rc<std::cell::RefCell<Option<lumine_gpu::text::CpuTextRaster>>>,
    styled_glyphs: HashMap<StyledGlyphKey, StyledGlyphImages>,
    rich_glyphs: RichGlyphCache,
    canvases: lumine_gpu::canvas::CanvasCache,
    svgs: lumine_gpu::svg::SvgCache,
    glyphs: HashMap<(char, u32), (Metrics, Vec<u8>)>,
    scratch: Vec<u32>,
    background: u32,
    origin: (f32, f32),
    mask: Option<lumine::scene::FadeMask>,
    affine_mattes: HashMap<lumine::scene::NodeId, std::rc::Rc<AffineMatte>>,
    affine_scene: u64,
    affine_rasterizations: u64,
}
impl Raster {
    /// Register an additional validated font and invalidate retained glyph shapes.
    pub fn register_font(&mut self, font: &lumine_gpu::text::FontData) {
        self.styled_fonts
            .borrow_mut()
            .get_or_insert_with(Default::default)
            .register_font(font);
        self.styled_glyphs.clear();
        self.rich_glyphs.clear();
        self.affine_mattes.clear();
    }

    pub fn new(width: usize, height: usize) -> Self {
        Self {
            pixels: vec![0x10141c; width * height],
            width,
            height,
            font: Font::from_bytes(
                include_bytes!("../../../assets/DejaVuSans.ttf") as &[u8],
                FontSettings::default(),
            )
            .expect("bundled font"),
            styled_fonts: Default::default(),
            styled_glyphs: HashMap::new(),
            rich_glyphs: Vec::new(),
            canvases: Default::default(),
            svgs: Default::default(),
            glyphs: HashMap::new(),
            scratch: Vec::new(),
            background: 0x10141c,
            origin: (0., 0.),
            mask: None,
            affine_mattes: HashMap::new(),
            affine_scene: 0,
            affine_rasterizations: 0,
        }
    }
    /// Number of local primitive rasterizations; unchanged affine/opacity frames reuse pixels.
    pub fn affine_rasterizations(&self) -> u64 {
        self.affine_rasterizations
    }
    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.pixels.resize(width * height, 0x10141c);
    }
    pub fn render(&mut self, scene: &Scene, damage: &[Rect]) {
        if self.affine_scene != scene.identity() {
            self.affine_mattes.clear();
            self.affine_scene = scene.identity();
        }
        self.affine_mattes.retain(|node, _| scene.contains(*node));
        self.canvases.retain(scene);
        self.svgs.retain(scene);
        // No pixel copies here: scrolled regions repaint in full.
        let moved: Vec<Rect> = scene
            .scroll_moves()
            .iter()
            .filter(|scroll| scroll.serial == scene.flush_serial())
            .map(|scroll| scroll.clip)
            .collect();
        let damage = [damage, &moved].concat();
        self.render_flat(scene, scene.layer_items(None), &damage);
    }
    fn render_flat(
        &mut self,
        scene: &Scene,
        mut items: Vec<lumine::scene::PaintItem<'_>>,
        damage: &[Rect],
    ) {
        if damage.is_empty() {
            return;
        }
        // Invisible subtrees must not rasterize glyphs or alter the backdrop.
        items.retain(|item| item.effects.opacity > 0.);
        // Backdrop filters require already-composited pixels behind the filter. The CPU
        // reference path conservatively rebuilds the surface when a filter is present.
        let full = [Rect {
            x: 0.,
            y: 0.,
            width: self.width as f32,
            height: self.height as f32,
        }];
        let has_blur = items.iter().any(|item| item.effects.blur_radius > 0.);
        let damage = if has_blur { &full[..] } else { damage };
        for &dirty in damage {
            let (x0, y0, x1, y1) = self.region(dirty);
            for y in y0..y1 {
                self.pixels[y * self.width + x0..y * self.width + x1].fill(self.background);
            }
            for item in &items {
                self.mask = item.mask;
                let Some(clip) = intersect(dirty, item.clip.unwrap_or(full[0])) else {
                    continue;
                };
                if item.isolated {
                    self.isolated(scene, item, clip);
                    continue;
                }
                if item.transform != lumine::affine::Affine::IDENTITY
                    || !item.clip_regions.is_empty()
                    || item.fade_transform != lumine::affine::Affine::IDENTITY
                {
                    self.transformed(scene, item, clip);
                    continue;
                }
                let Some(visible) = intersect(
                    clip,
                    if let NodeKind::Quad(style) | NodeKind::Panel { quad: style, .. } = item.kind {
                        style.paint_bounds(item.bounds)
                    } else if let NodeKind::Svg(svg) = item.kind {
                        svg.paint_bounds(item.bounds)
                    } else if let NodeKind::Image(image) = item.kind {
                        image.paint_bounds(item.bounds)
                    } else {
                        item.bounds
                    },
                ) else {
                    continue;
                };
                if item.effects.blur_radius > 0. {
                    self.blur(
                        visible,
                        item.effects.blur_radius.ceil().min(64.) as usize,
                        item.bounds,
                        item.effects,
                    );
                }
                match item.kind {
                    NodeKind::Rect(color) => self.rect(item.bounds, clip, *color, item.effects),
                    NodeKind::Quad(style) | NodeKind::Panel { quad: style, .. } => {
                        self.quad(item.id, item.bounds, clip, style.clone(), item.effects)
                    }
                    NodeKind::Svg(svg) => {
                        if item.bounds.width > 0.
                            && item.bounds.height > 0.
                            && let Ok(image) = self.svgs.get(
                                item.id,
                                svg,
                                item.bounds.width,
                                item.bounds.height,
                                1.,
                            )
                        {
                            self.image(item.bounds, clip, &image, item.effects);
                        }
                    }
                    NodeKind::Image(image) => self.image(item.bounds, clip, image, item.effects),
                    NodeKind::Canvas(canvas) => {
                        if item.bounds.width > 0.
                            && item.bounds.height > 0.
                            && let Ok(raster) = self.canvases.get(
                                item.id,
                                canvas,
                                item.bounds.width,
                                item.bounds.height,
                                1.,
                            )
                            // Drawn from memory: never marked uploaded.
                            && let Some(image) = raster.pixels
                        {
                            self.image(item.bounds, clip, &image, item.effects);
                        }
                    }
                    #[cfg(target_os = "macos")]
                    NodeKind::NativeSurface(_) => {} // Native GPU-only source; no CPU readback.
                    NodeKind::RichText { text } => {
                        self.rich_text(text, item.bounds, clip, item.effects)
                    }
                    NodeKind::Text {
                        text,
                        color,
                        font_size,
                    } => {
                        if item.text_options != Default::default() {
                            let runs = if text.is_empty() {
                                Vec::new()
                            } else {
                                vec![lumine::rich_text::TextRun {
                                    range: 0..text.len(),
                                    font: item.font.clone(),
                                    font_size: *font_size,
                                    color: *color,
                                    ..Default::default()
                                }]
                            };
                            let rich = std::sync::Arc::new(
                                lumine::rich_text::RichText::new(text.clone(), runs)
                                    .unwrap()
                                    .with_options(item.text_options),
                            );
                            self.rich_text(&rich, item.bounds, clip, item.effects);
                        } else if *item.font == Default::default() {
                            self.text(text, item.bounds, clip, *color, *font_size, item.effects)
                        } else {
                            self.styled_text(
                                text,
                                item.bounds,
                                clip,
                                *color,
                                *font_size,
                                item.effects,
                                item.font,
                            )
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    /// Reference fallback rasterizes only the affected primitive, then samples
    /// it through the inverse matrix. The destination backdrop stays in world
    /// space, so a transformed frosted panel still samples preceding siblings.
    fn transformed(&mut self, scene: &Scene, item: &lumine::scene::PaintItem<'_>, clip: Rect) {
        use lumine::affine::Affine;
        let Some(inverse) = item.transform.inverse() else {
            return;
        };
        let ink = match item.kind {
            NodeKind::Quad(style) | NodeKind::Panel { quad: style, .. } => {
                style.paint_bounds(item.bounds)
            }
            NodeKind::Image(image) => image.paint_bounds(item.bounds),
            NodeKind::Svg(svg) => svg.paint_bounds(item.bounds),
            _ => item.bounds,
        };
        let x = ink.x.floor();
        let y = ink.y.floor();
        let width = ((ink.x + ink.width).ceil() - x).max(1.) as usize;
        let height = ((ink.y + ink.height).ceil() - y).max(1.) as usize;
        if width
            .checked_mul(height)
            .is_none_or(|n| n > 16 * 1024 * 1024)
        {
            return;
        }
        let matte = if matches!(item.kind, NodeKind::Container(_)) {
            None
        } else {
            let matte = self
                .affine_mattes
                .get(&item.id)
                .filter(|matte| {
                    matte.revision == scene.paint_revision(item.id)
                        && matte.ink == ink
                        && matte.edge_fade == item.effects.edge_fade
                })
                .cloned();
            let matte = if let Some(matte) = matte {
                matte
            } else {
                let make = |background| Self {
                    pixels: vec![background; width * height],
                    width,
                    height,
                    font: self.font.clone(),
                    styled_fonts: self.styled_fonts.clone(),
                    styled_glyphs: HashMap::new(),
                    rich_glyphs: Vec::new(),
                    canvases: Default::default(),
                    svgs: Default::default(),
                    glyphs: HashMap::new(),
                    scratch: Vec::new(),
                    background,
                    origin: (x, y),
                    mask: None,
                    affine_mattes: HashMap::new(),
                    affine_scene: 0,
                    affine_rasterizations: 0,
                };
                let mut black = make(0);
                let mut white = make(0xffffff);
                let mut primitive = item.clone();
                primitive.bounds.x -= x;
                primitive.bounds.y -= y;
                primitive.transform = Affine::IDENTITY;
                primitive.clip_regions = std::rc::Rc::from([]);
                primitive.fade_transform = Affine::IDENTITY;
                primitive.mask = None;
                primitive.clip = None;
                primitive.effects.opacity = 1.;
                primitive.effects.blur_radius = 0.;
                let full = [Rect::new(0., 0., width as f32, height as f32)];
                black.render_flat(scene, vec![primitive.clone()], &full);
                white.render_flat(scene, vec![primitive], &full);
                let pixels = black
                    .pixels
                    .iter()
                    .zip(&white.pixels)
                    .map(|(&b, &w)| {
                        let alpha = 255 - ((w & 255) as i32 - (b & 255) as i32).clamp(0, 255);
                        [(b >> 16) as u8, (b >> 8) as u8, b as u8, alpha as u8]
                    })
                    .collect();
                let matte = std::rc::Rc::new(AffineMatte {
                    revision: scene.paint_revision(item.id),
                    ink,
                    edge_fade: item.effects.edge_fade,
                    width,
                    height,
                    pixels,
                });
                self.affine_mattes.remove(&item.id);
                let retained = self
                    .affine_mattes
                    .values()
                    .map(|matte| matte.pixels.len() * 4)
                    .sum::<usize>();
                if self.affine_mattes.len() >= 128
                    || retained + width * height * 4 > AFFINE_MATTE_BUDGET
                {
                    self.affine_mattes.clear();
                }
                self.affine_mattes.insert(item.id, matte.clone());
                self.affine_rasterizations = self.affine_rasterizations.saturating_add(1);
                matte
            };
            Some(matte)
        };
        let Some(visible) = intersect(item.transform.bounds(ink), clip) else {
            return;
        };
        let (x0, y0, x1, y1) = self.region(visible);
        // Keep a copy only for backdrop sampling; the filter is applied to the
        // exact transformed allocation rather than its enclosing AABB.
        let backdrop = (item.effects.blur_radius > 0.).then(|| {
            self.filtered_backdrop(visible, item.effects.blur_radius.ceil().min(64.) as usize)
        });
        for py in y0..y1 {
            for px in x0..x1 {
                let (wx, wy) = (px as f32 + 0.5, py as f32 + 0.5);
                let coverage = item
                    .clip_regions
                    .iter()
                    .fold(1., |alpha, region| alpha * region.coverage(wx, wy));
                if coverage <= 0. {
                    continue;
                }
                let (lx, ly) = inverse.point(wx, wy);
                let (mx, my) = item.fade_transform.point(wx, wy);
                let _ = mx;
                let opacity = item.effects.opacity * coverage * mask_alpha(my, item.mask);
                let inside = lx >= item.bounds.x
                    && ly >= item.bounds.y
                    && lx < item.bounds.x + item.bounds.width
                    && ly < item.bounds.y + item.bounds.height;
                if inside && let Some(backdrop) = &backdrop {
                    let color = backdrop.pixel(px, py);
                    blend(
                        &mut self.pixels[py * self.width + px],
                        Color((color >> 16) as u8, (color >> 8) as u8, color as u8, 255),
                        opacity * edge_alpha(ly, item.bounds, item.effects.edge_fade),
                    );
                }
                let Some(matte) = &matte else {
                    continue;
                };
                let sample = sample_matte(matte, lx - x - 0.5, ly - y - 0.5);
                if sample[3] <= 0. {
                    continue;
                }
                let channel =
                    |index: usize| (sample[index] * 255. / sample[3]).round().clamp(0., 255.) as u8;
                blend(
                    &mut self.pixels[py * self.width + px],
                    Color(
                        channel(0),
                        channel(1),
                        channel(2),
                        sample[3].round().clamp(0., 255.) as u8,
                    ),
                    opacity,
                );
            }
        }
        self.mask = item.mask;
    }
    fn isolated(&mut self, scene: &Scene, item: &lumine::scene::PaintItem<'_>, clip: Rect) {
        let Some(inverse) = item.transform.inverse() else {
            return;
        };
        let bounds = scene.layer_bounds(item.id);
        let x = bounds.x.floor();
        let y = bounds.y.floor();
        let width = ((bounds.x + bounds.width).ceil() - x).max(1.) as usize;
        let height = ((bounds.y + bounds.height).ceil() - y).max(1.) as usize;
        // Reference backend extracts coverage using black/white mattes, preserving
        // its public opaque RGB pixel format. GPU layers store premultiplied RGBA.
        if width
            .checked_mul(height)
            .is_none_or(|n| n > 16 * 1024 * 1024)
        {
            return;
        }
        let make = |background| Self {
            pixels: vec![background; width * height],
            width,
            height,
            font: self.font.clone(),
            styled_fonts: self.styled_fonts.clone(),
            styled_glyphs: HashMap::new(),
            rich_glyphs: Vec::new(),
            canvases: Default::default(),
            svgs: Default::default(),
            glyphs: HashMap::new(),
            scratch: Vec::new(),
            background,
            origin: (x, y),
            mask: None,
            affine_mattes: HashMap::new(),
            affine_scene: 0,
            affine_rasterizations: 0,
        };
        let mut black = make(0);
        let mut white = make(0xffffff);
        let items: Vec<_> = scene
            .layer_items(Some(item.id))
            .into_iter()
            .map(|mut i| {
                i.bounds.x -= x;
                i.bounds.y -= y;
                let to_world = lumine::affine::Affine::translation(x, y);
                let to_target = lumine::affine::Affine::translation(-x, -y);
                i.transform = to_world.then(i.transform).then(to_target);
                i.clip_regions = i
                    .clip_regions
                    .iter()
                    .map(|region| lumine::scene::PaintClip {
                        inverse: to_world.then(region.inverse),
                        ..*region
                    })
                    .collect::<Vec<_>>()
                    .into();
                i.fade_transform = to_world.then(i.fade_transform).then(to_target);
                i.clip = i
                    .clip
                    .map(|r| Rect::new(r.x - x, r.y - y, r.width, r.height));
                i.mask = i
                    .mask
                    .map(|(r, bands)| (Rect::new(r.x - x, r.y - y, r.width, r.height), bands));
                i
            })
            .collect();
        let full = [Rect::new(0., 0., width as f32, height as f32)];
        black.render_flat(scene, items.clone(), &full);
        white.render_flat(scene, items, &full);
        let output = Rect::new(
            x - self.origin.0,
            y - self.origin.1,
            width as f32,
            height as f32,
        );
        let Some(visible) = intersect(output, clip) else {
            return;
        };
        let backdrop = (item.effects.blur_radius > 0.).then(|| {
            self.filtered_backdrop(visible, item.effects.blur_radius.ceil().min(64.) as usize)
        });
        let (x0, y0, x1, y1) = self.region(visible);
        for py in y0..y1 {
            for px in x0..x1 {
                let (wx, wy) = (px as f32 + 0.5, py as f32 + 0.5);
                let coverage = item
                    .clip_regions
                    .iter()
                    .fold(1., |alpha, region| alpha * region.coverage(wx, wy));
                if coverage <= 0. {
                    continue;
                }
                let (_, fy) = item.fade_transform.point(wx, wy);
                let (lx, ly) = inverse.point(wx, wy);
                let opacity = item.effects.opacity
                    * coverage
                    * edge_alpha(ly, item.bounds, item.effects.edge_fade)
                    * mask_alpha(fy, self.mask);
                if let Some(backdrop) = &backdrop
                    && lx >= item.bounds.x
                    && ly >= item.bounds.y
                    && lx < item.bounds.x + item.bounds.width
                    && ly < item.bounds.y + item.bounds.height
                {
                    let color = backdrop.pixel(px, py);
                    blend(
                        &mut self.pixels[py * self.width + px],
                        Color((color >> 16) as u8, (color >> 8) as u8, color as u8, 255),
                        opacity,
                    );
                }
                let sx = (px as f32 - output.x) as usize;
                let sy = (py as f32 - output.y) as usize;
                let b = black.pixels[sy * width + sx];
                let w = white.pixels[sy * width + sx];
                let alpha = 255 - ((w & 255) as i32 - (b & 255) as i32).clamp(0, 255);
                if alpha == 0 {
                    continue;
                }
                let channel = |shift: u32| {
                    ((((b >> shift) & 255) * 255 + alpha as u32 / 2) / alpha as u32).min(255) as u8
                };
                let color = Color(channel(16), channel(8), channel(0), alpha as u8);
                blend(&mut self.pixels[py * self.width + px], color, opacity);
            }
        }
    }
    fn region(&self, rect: Rect) -> (usize, usize, usize, usize) {
        (
            rect.x.floor().clamp(0., self.width as f32) as usize,
            rect.y.floor().clamp(0., self.height as f32) as usize,
            (rect.x + rect.width).ceil().clamp(0., self.width as f32) as usize,
            (rect.y + rect.height).ceil().clamp(0., self.height as f32) as usize,
        )
    }
    fn rect(&mut self, bounds: Rect, clip: Rect, color: Color, effects: Effects) {
        let Some(r) = intersect(bounds, clip) else {
            return;
        };
        let (x0, y0, x1, y1) = self.region(r);
        for y in y0..y1 {
            let edge =
                edge_alpha(y as f32, bounds, effects.edge_fade) * mask_alpha(y as f32, self.mask);
            for x in x0..x1 {
                blend(
                    &mut self.pixels[y * self.width + x],
                    color,
                    effects.opacity * edge,
                );
            }
        }
    }
    fn image(
        &mut self,
        bounds: Rect,
        clip: Rect,
        image: &lumine::image::ImageData,
        effects: Effects,
    ) {
        let transform = image.paint_transform(bounds);
        let Some(inverse) = transform.inverse() else {
            return;
        };
        let Some(visible) = intersect(image.paint_bounds(bounds), clip) else {
            return;
        };
        let (x0, y0, x1, y1) = self.region(visible);
        let pixels = image.pixels();
        for y in y0..y1 {
            let opacity = effects.opacity * mask_alpha(y as f32, self.mask);
            for x in x0..x1 {
                let (local_x, local_y) = inverse.point(x as f32 + 0.5, y as f32 + 0.5);
                if local_x < bounds.x
                    || local_x >= bounds.x + bounds.width
                    || local_y < bounds.y
                    || local_y >= bounds.y + bounds.height
                {
                    continue;
                }
                let sx = (((local_x - bounds.x) / bounds.width * image.width() as f32) as u32)
                    .min(image.width() - 1);
                let sy = (((local_y - bounds.y) / bounds.height * image.height() as f32) as u32)
                    .min(image.height() - 1);
                let i = ((sy * image.width() + sx) * 4) as usize;
                let p = &pixels[i..i + 4];
                blend(
                    &mut self.pixels[y * self.width + x],
                    Color(p[0], p[1], p[2], p[3]),
                    opacity * edge_alpha(local_y - 0.5, bounds, effects.edge_fade),
                );
            }
        }
    }
    fn quad(
        &mut self,
        node: lumine::scene::NodeId,
        bounds: Rect,
        clip: Rect,
        style: lumine::scene::QuadStyle,
        effects: Effects,
    ) {
        for shadow in style.shadows().iter().take(32) {
            let core = bounds.expand(shadow.spread.max(0.));
            let core = Rect::new(
                core.x + shadow.offset.x,
                core.y + shadow.offset.y,
                core.width,
                core.height,
            );
            if let Some(visible) = intersect(core.expand(shadow.blur_radius.max(0.) * 3.), clip) {
                let (x0, y0, x1, y1) = self.region(visible);
                for y in y0..y1 {
                    let opacity = effects.opacity * mask_alpha(y as f32, self.mask);
                    for x in x0..x1 {
                        let d = rounded_distance(x as f32 + 0.5, y as f32 + 0.5, core, {
                            let corners = lumine_gpu::decoration::shadow_corners(
                                &style,
                                core.width,
                                core.height,
                                shadow.spread.max(0.),
                            );
                            let right = x as f32 + 0.5 > core.x + core.width * 0.5;
                            let bottom = y as f32 + 0.5 > core.y + core.height * 0.5;
                            corners[match (right, bottom) {
                                (false, false) => 0,
                                (true, false) => 1,
                                (true, true) => 2,
                                (false, true) => 3,
                            }]
                        });
                        let alpha =
                            (-0.5 * (d.max(0.) / shadow.blur_radius.max(0.001)).powi(2)).exp();
                        blend(
                            &mut self.pixels[y * self.width + x],
                            shadow.color,
                            alpha * opacity,
                        );
                    }
                }
            }
        }
        if style.decoration.is_some() && bounds.width > 0. && bounds.height > 0. {
            if let Ok(raster) =
                self.canvases
                    .get_decoration(node, &style, bounds.width, bounds.height, 1.)
                && let Some(image) = raster.pixels
            {
                self.image(bounds, clip, &image, effects);
            }
            return;
        }
        let Some(visible) = intersect(bounds, clip) else {
            return;
        };
        let (x0, y0, x1, y1) = self.region(visible);
        for y in y0..y1 {
            let amount = effects.opacity
                * edge_alpha(y as f32, bounds, effects.edge_fade)
                * mask_alpha(y as f32, self.mask);
            for x in x0..x1 {
                let d = rounded_distance(x as f32 + 0.5, y as f32 + 0.5, bounds, style.radius);
                let outer = (0.5 - d).clamp(0., 1.);
                let inner = (0.5 - d - style.border_width.max(0.)).clamp(0., 1.);
                let a = style.fill.3 as f32 / 255. * inner
                    + style.border_color.3 as f32 / 255. * (outer - inner);
                if a > 0. {
                    let channel = |fill: u8, border: u8| {
                        ((fill as f32 * style.fill.3 as f32 / 255. * inner
                            + border as f32 * style.border_color.3 as f32 / 255. * (outer - inner))
                            / a)
                            .round() as u8
                    };
                    let c = Color(
                        channel(style.fill.0, style.border_color.0),
                        channel(style.fill.1, style.border_color.1),
                        channel(style.fill.2, style.border_color.2),
                        (a * 255.).round() as u8,
                    );
                    blend(&mut self.pixels[y * self.width + x], c, amount);
                }
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn rich_text(
        &mut self,
        text: &std::sync::Arc<lumine::rich_text::RichText>,
        bounds: Rect,
        clip: Rect,
        effects: Effects,
    ) {
        let Some(clip) = intersect(bounds, clip) else {
            return;
        };
        let (x0, y0, x1, y1) = self.region(clip);
        let width = bounds.width.to_bits();
        let height = bounds.height.to_bits();
        let mut index = self
            .rich_glyphs
            .iter()
            .position(|(rich, w, h, _)| rich == text && *w == width && *h == height);
        let mut oversized = false;
        if index.is_none() {
            let images = self
                .styled_fonts
                .borrow_mut()
                .get_or_insert_with(Default::default)
                .rasterize_rich(text, bounds.width, bounds.height);
            let bytes = |rich: &lumine::rich_text::RichText, images: &RichGlyphImages| {
                rich.storage_bytes()
                    + images.0.capacity()
                        * std::mem::size_of::<(i32, i32, Color, cosmic_text::SwashImage)>()
                    + images.1.capacity() * std::mem::size_of::<lumine_gpu::text::RichDecoration>()
                    + images
                        .0
                        .iter()
                        .map(|(_, _, _, i)| i.data.capacity())
                        .sum::<usize>()
            };
            let new_bytes = bytes(text, &images);
            if self.rich_glyphs.len() >= 128
                || self
                    .rich_glyphs
                    .iter()
                    .map(|(r, _, _, i)| bytes(r, i))
                    .sum::<usize>()
                    .saturating_add(new_bytes)
                    > STYLED_GLYPH_BUDGET
            {
                self.rich_glyphs.clear();
            }
            oversized = new_bytes > STYLED_GLYPH_BUDGET;
            index = Some(self.rich_glyphs.len());
            self.rich_glyphs.push((text.clone(), width, height, images));
        }
        let images = &self.rich_glyphs[index.unwrap()].3;
        let paint_decorations = |pixels: &mut [u32], background: bool| {
            for d in images.1.iter().filter(|d| d.background == background) {
                let left = (bounds.x + d.bounds.x).floor().max(x0 as f32) as usize;
                let top = (bounds.y + d.bounds.y).floor().max(y0 as f32) as usize;
                let right = (bounds.x + d.bounds.x + d.bounds.width)
                    .ceil()
                    .min(x1 as f32)
                    .max(0.) as usize;
                let bottom = (bounds.y + d.bounds.y + d.bounds.height)
                    .ceil()
                    .min(y1 as f32)
                    .max(0.) as usize;
                for y in top..bottom {
                    let opacity = effects.opacity
                        * edge_alpha(y as f32, bounds, effects.edge_fade)
                        * mask_alpha(y as f32, self.mask);
                    for x in left..right {
                        blend(&mut pixels[y * self.width + x], d.color, opacity);
                    }
                }
            }
        };
        paint_decorations(&mut self.pixels, true);
        for (x, y, color, image) in &images.0 {
            let gx = bounds.x.floor() as i32 + x + image.placement.left;
            let gy = bounds.y.floor() as i32 + y - image.placement.top;
            for by in 0..image.placement.height as usize {
                let py = gy + by as i32;
                if py < y0 as i32 || py >= y1 as i32 {
                    continue;
                }
                let opacity = effects.opacity
                    * edge_alpha(py as f32, bounds, effects.edge_fade)
                    * mask_alpha(py as f32, self.mask);
                for bx in 0..image.placement.width as usize {
                    let px = gx + bx as i32;
                    if px < x0 as i32 || px >= x1 as i32 {
                        continue;
                    }
                    let i = by * image.placement.width as usize + bx;
                    let (ink, alpha) = match image.content {
                        cosmic_text::SwashContent::Mask => (*color, image.data[i] as f32 / 255.),
                        cosmic_text::SwashContent::Color => (
                            Color(
                                image.data[i * 4],
                                image.data[i * 4 + 1],
                                image.data[i * 4 + 2],
                                color.3,
                            ),
                            image.data[i * 4 + 3] as f32 / 255.,
                        ),
                        cosmic_text::SwashContent::SubpixelMask => {
                            (*color, image.data[i * 4] as f32 / 255.)
                        }
                    };
                    blend(
                        &mut self.pixels[py as usize * self.width + px as usize],
                        ink,
                        opacity * alpha,
                    );
                }
            }
        }
        paint_decorations(&mut self.pixels, false);
        if oversized {
            self.rich_glyphs.pop();
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn styled_text(
        &mut self,
        text: &str,
        bounds: Rect,
        clip: Rect,
        color: Color,
        size: f32,
        effects: Effects,
        font: &lumine::text_layout::FontStyle,
    ) {
        let Some(clip) = intersect(bounds, clip) else {
            return;
        };
        let (x0, y0, x1, y1) = self.region(clip);
        // Bound retained shaped data; default-font text keeps the lightweight fontdue path.
        let key = (
            text.to_owned(),
            size.to_bits(),
            bounds.width.to_bits(),
            bounds.height.to_bits(),
            font.clone(),
        );
        let mut oversized = false;
        if !self.styled_glyphs.contains_key(&key) {
            let images = self
                .styled_fonts
                .borrow_mut()
                .get_or_insert_with(Default::default)
                .rasterize(text, size, bounds.width, bounds.height, font);
            let new_bytes = styled_bytes(&key, &images);
            let bytes: usize = self
                .styled_glyphs
                .iter()
                .map(|(key, images)| styled_bytes(key, images))
                .sum();
            if self.styled_glyphs.len() >= 128
                || bytes.saturating_add(new_bytes) > STYLED_GLYPH_BUDGET
            {
                self.styled_glyphs.clear();
            }
            oversized = new_bytes > STYLED_GLYPH_BUDGET;
            self.styled_glyphs.insert(key.clone(), images);
        }
        for (x, y, image) in &self.styled_glyphs[&key] {
            let gx = bounds.x.floor() as i32 + x + image.placement.left;
            let gy = bounds.y.floor() as i32 + y - image.placement.top;
            for by in 0..image.placement.height as usize {
                let py = gy + by as i32;
                if py < y0 as i32 || py >= y1 as i32 {
                    continue;
                }
                let opacity = effects.opacity
                    * edge_alpha(py as f32, bounds, effects.edge_fade)
                    * mask_alpha(py as f32, self.mask);
                for bx in 0..image.placement.width as usize {
                    let px = gx + bx as i32;
                    if px < x0 as i32 || px >= x1 as i32 {
                        continue;
                    }
                    let i = by * image.placement.width as usize + bx;
                    let (ink, alpha) = match image.content {
                        cosmic_text::SwashContent::Mask => (color, image.data[i] as f32 / 255.),
                        cosmic_text::SwashContent::Color => (
                            Color(
                                image.data[i * 4],
                                image.data[i * 4 + 1],
                                image.data[i * 4 + 2],
                                color.3,
                            ),
                            image.data[i * 4 + 3] as f32 / 255.,
                        ),
                        cosmic_text::SwashContent::SubpixelMask => {
                            (color, image.data[i * 4] as f32 / 255.)
                        }
                    };
                    blend(
                        &mut self.pixels[py as usize * self.width + px as usize],
                        ink,
                        opacity * alpha,
                    );
                }
            }
        }
        if oversized {
            self.styled_glyphs.remove(&key);
        }
    }
    fn text(
        &mut self,
        text: &str,
        bounds: Rect,
        clip: Rect,
        color: Color,
        size: f32,
        effects: Effects,
    ) {
        let Some(clip) = intersect(bounds, clip) else {
            return;
        };
        let (x0, y0, x1, y1) = self.region(clip);
        let mut x = bounds.x;
        let mut baseline = bounds.y + size;
        let line_height = lumine::text_layout::LineHeight::default().resolve(size);
        for ch in text.chars() {
            if ch == '\n' {
                x = bounds.x;
                baseline += line_height;
                continue;
            }
            if self.glyphs.len() >= 4096 {
                self.glyphs.clear();
            }
            let (m, bitmap) = self
                .glyphs
                .entry((ch, size.to_bits()))
                .or_insert_with(|| self.font.rasterize(ch, size));
            if x + m.advance_width > bounds.x + bounds.width {
                x = bounds.x;
                baseline += line_height;
            }
            if baseline - size > clip.y + clip.height {
                break;
            }
            let gx = x.floor() as i32 + m.xmin;
            let gy = baseline.floor() as i32 - m.height as i32 - m.ymin;
            for by in 0..m.height {
                let py = gy + by as i32;
                if py < y0 as i32 || py >= y1 as i32 {
                    continue;
                }
                let opacity = effects.opacity
                    * edge_alpha(py as f32, bounds, effects.edge_fade)
                    * mask_alpha(py as f32, self.mask);
                for bx in 0..m.width {
                    let px = gx + bx as i32;
                    if px < x0 as i32 || px >= x1 as i32 {
                        continue;
                    }
                    blend(
                        &mut self.pixels[py as usize * self.width + px as usize],
                        color,
                        opacity * bitmap[by * m.width + bx] as f32 / 255.,
                    );
                }
            }
            x += m.advance_width;
        }
    }
    // Separable sliding-window box filter: O(area), independent of blur radius.
    fn filtered_backdrop(&mut self, bounds: Rect, radius: usize) -> FilteredBackdrop {
        let (x0, y0, x1, y1) = self.region(bounds);
        let width = x1 - x0;
        let mut pixels = Vec::with_capacity(width * (y1 - y0));
        for y in y0..y1 {
            pixels.extend_from_slice(&self.pixels[y * self.width + x0..y * self.width + x1]);
        }
        let mask = self.mask.take();
        self.blur(
            bounds,
            radius,
            bounds,
            Effects {
                blur_radius: radius as f32,
                ..Default::default()
            },
        );
        self.mask = mask;
        for y in y0..y1 {
            for x in x0..x1 {
                std::mem::swap(
                    &mut pixels[(y - y0) * width + x - x0],
                    &mut self.pixels[y * self.width + x],
                );
            }
        }
        FilteredBackdrop {
            x: x0,
            y: y0,
            width,
            pixels,
        }
    }
    fn blur(&mut self, bounds: Rect, radius: usize, fade_bounds: Rect, effects: Effects) {
        if radius == 0 {
            return;
        }
        let expanded = Rect {
            x: bounds.x - radius as f32,
            y: bounds.y - radius as f32,
            width: bounds.width + 2. * radius as f32,
            height: bounds.height + 2. * radius as f32,
        };
        let (x0, y0, x1, y1) = self.region(expanded);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let w = x1 - x0;
        let h = y1 - y0;
        self.scratch.resize(w * h, 0);
        for y in 0..h {
            let mut sum = [0u64; 3];
            let mut left = 0;
            let mut right = 0;
            for x in 0..w {
                let end = (x + radius + 1).min(w);
                while right < end {
                    add(&mut sum, self.pixels[(y + y0) * self.width + x0 + right]);
                    right += 1;
                }
                let start = x.saturating_sub(radius);
                while left < start {
                    sub(&mut sum, self.pixels[(y + y0) * self.width + x0 + left]);
                    left += 1;
                }
                self.scratch[y * w + x] = pack(sum, (right - left) as u64);
            }
        }
        let (bx0, by0, bx1, by1) = self.region(bounds);
        for x in bx0..bx1 {
            let mut sum = [0u64; 3];
            let mut top = 0;
            let mut bottom = 0;
            for y in 0..h {
                let end = (y + radius + 1).min(h);
                while bottom < end {
                    add(&mut sum, self.scratch[bottom * w + x - x0]);
                    bottom += 1;
                }
                let start = y.saturating_sub(radius);
                while top < start {
                    sub(&mut sum, self.scratch[top * w + x - x0]);
                    top += 1;
                }
                if y + y0 >= by0 && y + y0 < by1 {
                    let filtered = pack(sum, (bottom - top) as u64);
                    blend(
                        &mut self.pixels[(y + y0) * self.width + x],
                        Color(
                            (filtered >> 16) as u8,
                            (filtered >> 8) as u8,
                            filtered as u8,
                            255,
                        ),
                        effects.opacity
                            * edge_alpha((y + y0) as f32, fade_bounds, effects.edge_fade)
                            * mask_alpha((y + y0) as f32, self.mask),
                    );
                }
            }
        }
    }
    pub fn write_ppm(&self, path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
        write!(out, "P6\n{} {}\n255\n", self.width, self.height)?;
        for p in &self.pixels {
            out.write_all(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8])?;
        }
        Ok(())
    }
}
fn sample_matte(matte: &AffineMatte, x: f32, y: f32) -> [f32; 4] {
    if !x.is_finite()
        || !y.is_finite()
        || x < -1.
        || y < -1.
        || x > matte.width as f32
        || y > matte.height as f32
    {
        return [0.; 4];
    }
    let (left, top) = (x.floor() as isize, y.floor() as isize);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let mut result = [0.; 4];
    for (dx, wx) in [(0, 1. - fx), (1, fx)] {
        for (dy, wy) in [(0, 1. - fy), (1, fy)] {
            let (sx, sy) = (left + dx, top + dy);
            if sx < 0 || sy < 0 || sx >= matte.width as isize || sy >= matte.height as isize {
                continue;
            }
            let pixel = matte.pixels[sy as usize * matte.width + sx as usize];
            for channel in 0..4 {
                result[channel] += pixel[channel] as f32 * wx * wy;
            }
        }
    }
    result
}

fn add(s: &mut [u64; 3], p: u32) {
    s[0] += ((p >> 16) & 255) as u64;
    s[1] += ((p >> 8) & 255) as u64;
    s[2] += (p & 255) as u64;
}
fn sub(s: &mut [u64; 3], p: u32) {
    s[0] -= ((p >> 16) & 255) as u64;
    s[1] -= ((p >> 8) & 255) as u64;
    s[2] -= (p & 255) as u64;
}
fn pack(s: [u64; 3], n: u64) -> u32 {
    ((s[0] / n) as u32) << 16 | ((s[1] / n) as u32) << 8 | (s[2] / n) as u32
}
fn blend(pixel: &mut u32, c: Color, opacity: f32) {
    let a = (c.3 as f32 * opacity.clamp(0., 1.)).round() as u32;
    let inv = 255 - a;
    let r = (c.0 as u32 * a + ((*pixel >> 16) & 255) * inv + 127) / 255;
    let g = (c.1 as u32 * a + ((*pixel >> 8) & 255) * inv + 127) / 255;
    let b = (c.2 as u32 * a + (*pixel & 255) * inv + 127) / 255;
    *pixel = (r << 16) | (g << 8) | b;
}
fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let width = (a.x + a.width).min(b.x + b.width) - x;
    let height = (a.y + a.height).min(b.y + b.height) - y;
    (width > 0. && height > 0.).then_some(Rect {
        x,
        y,
        width,
        height,
    })
}

fn edge_alpha(y: f32, bounds: Rect, fade: f32) -> f32 {
    if fade > 0. {
        ((y + 0.5 - bounds.y).min(bounds.y + bounds.height - y - 0.5) / fade).clamp(0., 1.)
    } else {
        1.
    }
}

fn mask_alpha(y: f32, mask: Option<lumine::scene::FadeMask>) -> f32 {
    let Some((bounds, bands)) = mask else {
        return 1.;
    };
    let y = y + 0.5;
    let top = if bands[0] > 0. {
        (y - bounds.y) / bands[0]
    } else {
        1.
    };
    let bottom = if bands[1] > 0. {
        (bounds.y + bounds.height - y) / bands[1]
    } else {
        1.
    };
    top.clamp(0., 1.).powi(2) * bottom.clamp(0., 1.).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumine::scene::{Effects, Layout, NodeKind, Style, Transform};
    #[test]
    fn nearest_image_cells_stay_crisp_at_fractional_origin() {
        let image = std::sync::Arc::new(
            lumine::image::ImageData::new(2, 1, vec![0, 0, 0, 255, 255, 255, 255, 255])
                .unwrap()
                .sampled(lumine::image::ImageSampling::Nearest),
        );
        let mut scene = Scene::new(3., 1.);
        let node = scene.append(
            scene.root(),
            NodeKind::Image(image),
            Style {
                width: Some(2.),
                height: Some(1.),
                ..Default::default()
            },
        );
        scene.set_transform(node, Transform { x: 0.5, y: 0. });
        let mut raster = Raster::new(3, 1);
        let damage = scene.flush().damage;
        raster.render(&scene, &damage);
        assert_eq!(raster.pixels[1] & 0xffffff, 0xffffff);
    }
    #[test]
    fn affine_mattes_reuse_pixels_until_content_changes_and_prune_removed_nodes() {
        use lumine::affine::Affine;
        let mut scene = Scene::new(100., 100.);
        let node = scene.append(
            scene.root(),
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(20.),
                height: Some(20.),
                ..Default::default()
            },
        );
        scene.set_transform(node, Transform { x: 40., y: 40. });
        scene.set_paint_transform(node, Affine::rotation(0.2));
        let mut raster = Raster::new(100, 100);
        let damage = scene.flush().damage;
        raster.render(&scene, &damage);
        assert_eq!(raster.affine_rasterizations(), 1);
        for angle in [0.3, 0.4, 0.5] {
            scene.set_paint_transform(node, Affine::rotation(angle));
            scene.set_effects(
                node,
                Effects {
                    opacity: 0.5,
                    ..Default::default()
                },
            );
            let damage = scene.flush().damage;
            raster.render(&scene, &damage);
        }
        assert_eq!(raster.affine_rasterizations(), 1);
        scene.set_kind(node, NodeKind::Rect(Color(0, 255, 0, 255)));
        let damage = scene.flush().damage;
        raster.render(&scene, &damage);
        assert_eq!(raster.affine_rasterizations(), 2);
        assert_eq!(raster.affine_mattes.len(), 1);
        scene.remove(node);
        let damage = scene.flush().damage;
        raster.render(&scene, &damage);
        assert!(raster.affine_mattes.is_empty());
    }
    #[test]
    fn transformed_blur_samples_world_backdrop_and_does_not_touch_aabb_corners() {
        use lumine::affine::Affine;
        let mut scene = Scene::new(40., 40.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        scene.append(
            scene.root(),
            NodeKind::Rect(Color(255, 255, 255, 255)),
            Style {
                width: Some(20.),
                height: Some(40.),
                ..Default::default()
            },
        );
        let frost = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(20.),
                height: Some(20.),
                ..Default::default()
            },
        );
        scene.set_paint_transform(frost, Affine::rotation(std::f32::consts::FRAC_PI_4));
        scene.set_transform(frost, Transform { x: 20., y: 10. });
        scene.set_effects(
            frost,
            Effects {
                blur_radius: 3.,
                ..Default::default()
            },
        );
        let mut raster = Raster::new(40, 40);
        let damage = scene.flush().damage;
        raster.render(&scene, &damage);
        assert_eq!(raster.pixels[11 * 40 + 7], 0xffffff);
        let filtered = raster.pixels[22 * 40 + 19];
        assert_ne!(filtered, 0xffffff);
        assert_ne!(filtered, raster.background);
        scene.set_isolated(frost, true);
        scene.set_paint_transform(
            frost,
            Affine {
                a: 1.,
                b: 1.,
                c: 1.,
                d: 1.,
                tx: 0.,
                ty: 0.,
            },
        );
        let damage = scene.flush().damage;
        raster.render(&scene, &damage);
        assert_eq!(raster.pixels[22 * 40 + 19], 0xffffff);
    }
    #[test]
    fn affine_subtree_pixels_match_inverse_hit_geometry_and_exact_clips() {
        use lumine::affine::Affine;
        let mut scene = Scene::new(100., 100.);
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(20.),
                height: Some(20.),
                clip: true,
                ..Default::default()
            },
        );
        scene.append(
            parent,
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(20.),
                height: Some(40.),
                ..Default::default()
            },
        );
        scene.set_paint_transform(parent, Affine::rotation(std::f32::consts::FRAC_PI_4));
        scene.set_transform(parent, Transform { x: 50., y: 20. });
        let mut raster = Raster::new(100, 100);
        let damage = scene.flush().damage;
        raster.render(&scene, &damage);
        assert_eq!(raster.pixels[34 * 100 + 50], 0xff0000);
        assert_eq!(raster.pixels[21 * 100 + 37], raster.background);
        assert_eq!(raster.pixels[58 * 100 + 50], raster.background);
        scene.set_isolated(parent, true);
        let damage = scene.flush().damage;
        raster.render(&scene, &damage);
        assert_eq!(raster.pixels[34 * 100 + 50], 0xff0000);
        assert_eq!(raster.pixels[21 * 100 + 37], raster.background);
    }
    #[test]
    fn panel_background_border_and_children_have_correct_paint_order() {
        use lumine::scene::{Insets, QuadStyle};
        let mut scene = Scene::new(40., 40.);
        let panel = scene.append(
            scene.root(),
            NodeKind::Panel {
                layout: Layout::Row,
                quad: QuadStyle {
                    fill: Color(20, 40, 60, 255),
                    radius: 4.,
                    border_color: Color(200, 100, 0, 255),
                    border_width: 2.,
                    ..Default::default()
                },
            },
            Style {
                width: Some(40.),
                height: Some(40.),
                padding_edges: Some(Insets {
                    left: 10.,
                    top: 12.,
                    right: 3.,
                    bottom: 5.,
                }),
                ..Default::default()
            },
        );
        scene.append(
            panel,
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(8.),
                height: Some(8.),
                ..Default::default()
            },
        );
        let mut raster = Raster::new(40, 40);
        let damage = scene.flush().damage;
        raster.render(&scene, &damage);
        assert_eq!(raster.pixels[6 * 40 + 6], 0x14283c);
        assert_eq!(raster.pixels[14 * 40 + 12], 0xff0000);
        assert_eq!(raster.pixels[40 + 20], 0xc86400);
    }
    fn scene() -> Scene {
        let mut s = Scene::new(100., 80.);
        s.set_kind(s.root(), NodeKind::Container(Layout::Overlay));
        s
    }
    fn rectangle(s: &mut Scene, x: f32, color: Color) -> lumine::scene::NodeId {
        let id = s.append(
            s.root(),
            NodeKind::Rect(color),
            Style {
                width: Some(30.),
                height: Some(30.),
                ..Default::default()
            },
        );
        s.set_transform(id, Transform { x, y: 10. });
        id
    }
    #[test]
    fn incremental_matches_full_after_move_opacity_and_removal() {
        let mut s = scene();
        rectangle(&mut s, 5., Color(80, 20, 20, 255));
        let top = rectangle(&mut s, 20., Color(0, 200, 255, 150));
        let mut partial = Raster::new(100, 80);
        let mut full = Raster::new(100, 80);
        let damage = s.flush().damage;
        partial.render(&s, &damage);
        for step in 0..4 {
            match step {
                0 => s.set_transform(top, Transform { x: 40., y: 20. }),
                1 => s.set_effects(
                    top,
                    Effects {
                        opacity: 0.4,
                        ..Default::default()
                    },
                ),
                2 => s.remove(top),
                _ => {}
            }
            let damage = s.flush().damage;
            partial.render(&s, &damage);
            full.render(&s, &[Rect::new(0., 0., 100., 80.)]);
            assert!(
                partial.pixels == full.pixels,
                "incremental image differs from full repaint at step {step}"
            );
        }
    }
    #[test]
    fn blur_and_edge_fade_change_pixels_and_stay_stable() {
        let mut s = scene();
        rectangle(&mut s, 10., Color(255, 255, 255, 255));
        let glass = rectangle(&mut s, 25., Color(50, 80, 160, 100));
        s.set_effects(
            glass,
            Effects {
                opacity: 0.8,
                blur_radius: 4.,
                edge_fade: 8.,
            },
        );
        let mut r = Raster::new(100, 80);
        let damage = s.flush().damage;
        r.render(&s, &damage);
        let expected = r.pixels.clone();
        r.render(&s, &[Rect::new(0., 0., 100., 80.)]);
        assert_eq!(expected, r.pixels);
        s.set_effects(glass, Effects::default());
        let damage = s.flush().damage;
        r.render(&s, &damage);
        assert_ne!(expected, r.pixels);
    }
}

fn rounded_distance(x: f32, y: f32, bounds: Rect, radius: f32) -> f32 {
    let r = radius.max(0.).min(bounds.width.min(bounds.height) * 0.5);
    let qx = (x - bounds.x - bounds.width * 0.5).abs() - bounds.width * 0.5 + r;
    let qy = (y - bounds.y - bounds.height * 0.5).abs() - bounds.height * 0.5 + r;
    qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - r
}

#[cfg(test)]
mod extended_tests {
    use super::*;
    use lumine::scene::{BoxShadow, Layout, QuadStyle, Style, Transform};
    #[test]
    fn shadow_damage_and_clipped_images_match_full_repaint() {
        let mut raster = Raster::new(80, 60);
        let mut scene = Scene::new(80., 60.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let quad = scene.append(
            scene.root(),
            NodeKind::Quad(QuadStyle {
                fill: Color(230, 30, 40, 255),
                radius: 8.,
                border_color: Color(0, 255, 0, 255),
                border_width: 2.,
                shadow: Some(BoxShadow {
                    color: Color(0, 0, 0, 160),
                    offset: Transform { x: -8., y: 3. },
                    blur_radius: 3.,
                    spread: 2.,
                }),
                decoration: None,
            }),
            Style {
                width: Some(24.),
                height: Some(24.),
                ..Default::default()
            },
        );
        scene.set_transform(quad, Transform { x: 20., y: 10. });
        let d = scene.flush().damage;
        raster.render(&scene, &d);
        scene.set_transform(quad, Transform { x: 45., y: 25. });
        let d = scene.flush().damage;
        raster.render(&scene, &d);
        let partial = raster.pixels.clone();
        raster.render(&scene, &[Rect::new(0., 0., 80., 60.)]);
        assert_eq!(partial, raster.pixels);
        let image =
            std::sync::Arc::new(lumine::image::ImageData::new(1, 1, vec![255, 0, 255, 255]).unwrap());
        scene.append(
            scene.root(),
            NodeKind::Image(image),
            Style {
                width: Some(10.),
                height: Some(10.),
                ..Default::default()
            },
        );
        let d = scene.flush().damage;
        raster.render(&scene, &d);
        assert_eq!(raster.pixels[0], 0xff00ff);
    }
}

#[cfg(test)]
mod isolation_tests {
    use super::*;
    use lumine::scene::{Layout, Style, Transform};
    #[test]
    fn overlapping_fade_bands_multiply_independent_quadratic_ramps() {
        let mut scene = Scene::new(8., 8.);
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(8.),
                height: Some(8.),
                clip: true,
                fade_edges: [8., 8.],
                ..Default::default()
            },
        );
        scene.append(
            parent,
            NodeKind::Rect(Color(255, 255, 255, 255)),
            Style {
                width: Some(8.),
                height: Some(8.),
                ..Default::default()
            },
        );
        let mut raster = Raster::new(8, 8);
        let damage = scene.flush().damage;
        raster.render(&scene, &damage);
        let mut expected = 0x10141c;
        blend(
            &mut expected,
            Color(255, 255, 255, 255),
            (3.5_f32 / 8.).powi(2) * (4.5_f32 / 8.).powi(2),
        );
        assert_eq!(raster.pixels[3 * 8 + 4], expected);
    }

    #[test]
    fn ancestor_fade_bands_follow_translated_images_and_isolated_layers() {
        for isolated in [false, true] {
            let mut scene = Scene::new(32., 32.);
            let group = scene.append(
                scene.root(),
                NodeKind::Container(Layout::Overlay),
                Style {
                    width: Some(16.),
                    height: Some(16.),
                    clip: true,
                    fade_edges: [4., 8.],
                    ..Default::default()
                },
            );
            scene.set_transform(group, Transform { x: 4., y: 6. });
            scene.set_isolated(group, isolated);
            let green = Color(0, 255, 0, 255);
            scene.append(
                group,
                NodeKind::Image(std::sync::Arc::new(
                    lumine::image::ImageData::new(1, 1, vec![0, 255, 0, 255]).unwrap(),
                )),
                Style {
                    width: Some(16.),
                    height: Some(16.),
                    ..Default::default()
                },
            );
            let mut raster = Raster::new(32, 32);
            let damage = scene.flush().damage;
            raster.render(&scene, &damage);
            let mut top = 0x10141c;
            blend(&mut top, green, (0.5_f32 / 4.).powi(2));
            let mut bottom = 0x10141c;
            blend(&mut bottom, green, (0.5_f32 / 8.).powi(2));
            assert_eq!(raster.pixels[6 * 32 + 4], top, "isolated={isolated}");
            assert_eq!(raster.pixels[21 * 32 + 4], bottom, "isolated={isolated}");
            assert_eq!(raster.pixels[13 * 32 + 4], 0x00ff00);
            scene.set_transform(group, Transform { x: 4., y: 10. });
            let damage = scene.flush().damage;
            raster.render(&scene, &damage);
            let mut fresh = Raster::new(32, 32);
            fresh.render(&scene, &[Rect::new(0., 0., 32., 32.)]);
            assert_eq!(raster.pixels, fresh.pixels);
        }
    }

    #[test]
    fn backdrop_blur_blends_coverage_instead_of_overwriting_at_partial_opacity() {
        let mut scene = Scene::new(16., 16.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        scene.append(
            scene.root(),
            NodeKind::Rect(Color(255, 255, 255, 255)),
            Style {
                width: Some(8.),
                height: Some(16.),
                ..Default::default()
            },
        );
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(16.),
                height: Some(16.),
                clip: true,
                ..Default::default()
            },
        );
        let panel = scene.append(
            parent,
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(16.),
                height: Some(16.),
                ..Default::default()
            },
        );
        let mut opaque = Raster::new(16, 16);
        scene.set_effects(
            panel,
            Effects {
                blur_radius: 4.,
                ..Default::default()
            },
        );
        let damage = scene.flush().damage;
        opaque.render(&scene, &damage);
        let mut style = scene.style(parent);
        style.fade_edges = [4., 4.];
        scene.set_style(parent, style);
        scene.set_effects(
            panel,
            Effects {
                blur_radius: 4.,
                opacity: 0.5,
                ..Default::default()
            },
        );
        let mut half = Raster::new(16, 16);
        let damage = scene.flush().damage;
        half.render(&scene, &damage);
        let filtered = opaque.pixels[8 * 16 + 7];
        let mut expected = 0xffffff;
        blend(
            &mut expected,
            Color(
                (filtered >> 16) as u8,
                (filtered >> 8) as u8,
                filtered as u8,
                255,
            ),
            0.5,
        );
        assert_ne!(filtered, 0xffffff);
        assert_eq!(half.pixels[8 * 16 + 7], expected);
        let mut faded = 0xffffff;
        blend(
            &mut faded,
            Color(
                (filtered >> 16) as u8,
                (filtered >> 8) as u8,
                filtered as u8,
                255,
            ),
            0.5 * (0.5_f32 / 4.).powi(2),
        );
        assert_eq!(half.pixels[7], faded);
    }

    #[test]
    fn opacity_is_applied_once_to_overlapping_children() {
        let mut scene = Scene::new(40., 30.);
        let group = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(20.),
                height: Some(20.),
                ..Default::default()
            },
        );
        scene.set_isolated(group, true);
        scene.set_effects(
            group,
            Effects {
                opacity: 0.5,
                ..Default::default()
            },
        );
        scene.append(
            group,
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(20.),
                height: Some(20.),
                ..Default::default()
            },
        );
        let blue = scene.append(
            group,
            NodeKind::Rect(Color(0, 0, 255, 255)),
            Style {
                width: Some(12.),
                height: Some(20.),
                ..Default::default()
            },
        );
        scene.set_transform(blue, Transform { x: 8., y: 0. });
        let mut raster = Raster::new(40, 30);
        let d = scene.flush().damage;
        raster.render(&scene, &d);
        let p = raster.pixels[10 * 40 + 12];
        assert!(
            ((p >> 16) & 255) < 20,
            "red backdrop must not leak through the isolated blue child"
        );
        assert!((p & 255) > 130);
        scene.set_transform(group, Transform { x: 10., y: 5. });
        let d = scene.flush().damage;
        raster.render(&scene, &d);
        let partial = raster.pixels.clone();
        raster.render(&scene, &[Rect::new(0., 0., 40., 30.)]);
        assert_eq!(partial, raster.pixels);
    }
}

#[cfg(test)]
mod line_height_tests {
    use super::*;
    use lumine::{
        scene::Style,
        text_layout::{FontStyle, LineHeight},
    };

    #[test]
    fn changing_line_pitch_repairs_old_pixels_and_keys_software_shapes() {
        let mut scene = Scene::new(120., 100.);
        let node = scene.append(
            scene.root(),
            NodeKind::Text {
                text: "M\nM".into(),
                color: Color(255, 255, 255, 255),
                font_size: 18.,
            },
            Style {
                width: Some(80.),
                height: Some(90.),
                ..Default::default()
            },
        );
        let mut incremental = Raster::new(120, 100);
        for pitch in [40., 8., 24., 40.] {
            scene.set_font(
                node,
                FontStyle {
                    line_height: LineHeight::px(pitch),
                    ..Default::default()
                },
            );
            let damage = scene.flush().damage;
            assert!(!damage.is_empty());
            incremental.render(&scene, &damage);
            let mut fresh = Raster::new(120, 100);
            fresh.render(&scene, &[Rect::new(0., 0., 120., 100.)]);
            assert_eq!(incremental.pixels, fresh.pixels, "pitch {pitch}");
            assert!(
                incremental
                    .styled_glyphs
                    .keys()
                    .any(|key| key.4.line_height == LineHeight::px(pitch))
            );
            let cache_size = incremental.styled_glyphs.len();
            scene.set_font(
                node,
                FontStyle {
                    line_height: LineHeight::px(pitch),
                    ..Default::default()
                },
            );
            let idle = scene.flush().damage;
            assert!(idle.is_empty());
            incremental.render(&scene, &idle);
            assert_eq!(incremental.styled_glyphs.len(), cache_size);
        }
    }
}

#[cfg(test)]
mod letter_spacing_tests {
    use super::*;
    use lumine::{
        scene::Style,
        text_layout::{FontStyle, LetterSpacing},
    };

    #[test]
    fn tracking_changes_repair_old_glyphs_and_distinguish_cached_font_keys() {
        let mut scene = Scene::new(240., 70.);
        let node = scene.append(
            scene.root(),
            NodeKind::Text {
                text: "MMMM".into(),
                color: Color(255, 255, 255, 255),
                font_size: 18.,
            },
            Style {
                width: Some(220.),
                height: Some(50.),
                ..Default::default()
            },
        );
        let mut incremental = Raster::new(240, 70);
        let mut previous = None;
        for spacing in [3., -1., -100., 3.] {
            scene.set_font(
                node,
                FontStyle {
                    letter_spacing: LetterSpacing::px(spacing),
                    ..Default::default()
                },
            );
            let damage = scene.flush().damage;
            incremental.render(&scene, &damage);
            let mut fresh = Raster::new(240, 70);
            fresh.render(&scene, &[Rect::new(0., 0., 240., 70.)]);
            assert_eq!(incremental.pixels, fresh.pixels);
            if let Some(old) = previous.replace(incremental.pixels.clone()) {
                assert_ne!(
                    old, incremental.pixels,
                    "tracking must move actual glyph ink"
                );
            }
            assert!(
                incremental
                    .styled_glyphs
                    .keys()
                    .any(|key| key.4.letter_spacing == LetterSpacing::px(spacing))
            );
        }
        assert_eq!(incremental.styled_glyphs.len(), 3);
    }
}

#[cfg(test)]
mod hidden_effect_tests {
    use super::*;
    use lumine::scene::{Layout, Style};

    #[test]
    fn transparent_subtrees_do_not_blur_backdrop_or_rasterize_text() {
        for isolated in [false, true] {
            let mut scene = Scene::new(60., 40.);
            scene.append(
                scene.root(),
                NodeKind::Rect(Color(255, 255, 255, 255)),
                Style {
                    width: Some(30.),
                    height: Some(40.),
                    absolute: true,
                    ..Default::default()
                },
            );
            let mut raster = Raster::new(60, 40);
            let damage = scene.flush().damage;
            raster.render(&scene, &damage);
            let baseline = raster.pixels.clone();
            let group = scene.append(
                scene.root(),
                NodeKind::Container(Layout::Overlay),
                Style {
                    width: Some(60.),
                    height: Some(40.),
                    absolute: true,
                    ..Default::default()
                },
            );
            scene.set_isolated(group, isolated);
            scene.set_effects(
                group,
                Effects {
                    opacity: 0.,
                    blur_radius: 5.,
                    ..Default::default()
                },
            );
            scene.append(
                group,
                NodeKind::Text {
                    text: "Hidden".into(),
                    color: Color(255, 0, 0, 255),
                    font_size: 16.,
                },
                Style::default(),
            );
            let damage = scene.flush().damage;
            raster.render(&scene, &damage);
            assert!(
                raster.pixels == baseline,
                "hidden subtree changed backdrop; isolated={isolated}"
            );
            assert!(raster.glyphs.is_empty());
            assert!(raster.scratch.is_empty());
            scene.set_effects(
                group,
                Effects {
                    opacity: 1.,
                    blur_radius: 5.,
                    ..Default::default()
                },
            );
            let damage = scene.flush().damage;
            raster.render(&scene, &damage);
            assert_ne!(raster.pixels, baseline);
            scene.set_effects(
                group,
                Effects {
                    opacity: 0.,
                    blur_radius: 5.,
                    ..Default::default()
                },
            );
            let damage = scene.flush().damage;
            raster.render(&scene, &damage);
            assert_eq!(raster.pixels, baseline);
        }
    }
}

#[cfg(test)]
#[test]
fn rounded_overflow_masks_reference_raster_corners() {
    use lumine::{compose::prelude::*, widgets::Ui};
    for cached in [false, true] {
        let mut ui = Ui::new(64., 64.);
        ui.mount(
            div()
                .size(64., 64.)
                .rounded(12.)
                .overflow_hidden()
                .isolated(cached)
                .child(div().size(64., 64.).bg(rgb(0xff0000))),
        );
        ui.prepare_frame();
        let mut scene = ui.scene.borrow_mut();
        let damage = scene.flush().damage;
        let mut raster = Raster::new(64, 64);
        raster.render(&scene, &damage);
        for i in [0, 63, 63 * 64, 64 * 64 - 1] {
            assert_eq!(raster.pixels[i], 0x10141c);
        }
        assert_eq!(raster.pixels[32 * 64 + 32], 0xff0000);
        assert!(
            (0..12).any(
                |y| (0..12).any(|x| ![0x10141c, 0xff0000].contains(&raster.pixels[y * 64 + x]))
            )
        );
    }
}
