//! Damage-scissored retained GPU renderer for Vulkan (Linux) and Metal (macOS).
//! The output is premultiplied BGRA8 (read back as RGBA8). Text uses cosmic-text shaping and a bounded
//! GPU glyph atlas; stable text never reshapes on paint or compositor updates.
use cosmic_text::{Buffer, CacheKey, FontSystem, Metrics, SwashCache, SwashContent};
use lumine::scene::{Color, NodeId, NodeKind, Rect, Scene};
use rustc_hash::FxHashMap;
use std::{cell::RefCell, fmt, rc::Rc, sync::Arc};

#[cfg(feature = "benchmark")]
pub mod benchmark;
mod blur_cache;
mod dual_kawase;
mod effect_chain;
mod gaussian;
mod image_mips;
#[cfg(target_os = "macos")]
mod native_surface;
mod paint_geometry;
mod procedural;
pub mod profiling;
mod draw_diagnostics;
mod surface;
mod upload;
pub use surface::PresentationStatus;

const MAX_ATLAS: u32 = 2048;
const INITIAL_ATLAS: u32 = 512;
/// An atlas that fills again within this many frames of its last recycle
/// doubles instead (see `render_flat`).
const ATLAS_THRASH_FRAMES: u64 = 600;
/// How long after its last resize a macOS layer keeps a third drawable.
#[cfg(target_os = "macos")]
const WIDE_DRAWABLES: std::time::Duration = std::time::Duration::from_secs(2);
/// SVG rasters up to this size (physical pixels) share the glyph atlas.
const ATLAS_IMAGE_MAX: u32 = 128;
/// Write straight-alpha `pixels` premultiplied (as `upload_image` does) into
/// the interior of a bordered atlas cell `width` + 2 texels wide.
fn premultiply_into_cell(cell: &mut [u8], pixels: &[u8], width: u32) {
    let row = width as usize * 4;
    for (y, source) in pixels.chunks_exact(row).enumerate() {
        let start = ((y + 1) * (width as usize + 2) + 1) * 4;
        for (texel, pixel) in cell[start..start + row]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(source.as_chunks::<4>().0)
        {
            let alpha = u16::from(pixel[3]);
            for channel in 0..3 {
                texel[channel] = ((u16::from(pixel[channel]) * alpha + 127) / 255) as u8;
            }
            texel[3] = pixel[3];
        }
    }
}
/// The retained target's format: BGRA, as window surfaces are on macOS and
/// X11, so presenting can be a plain copy (see `copy_present`). Readback
/// converts to RGBA.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
/// Backdrop filter implementation. Existing `Effects` and blur radii work with either.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlurAlgorithm {
    #[default]
    Gaussian,
    /// Approximate Gaussian using a resolution pyramid; radius changes blend smoothly.
    DualKawase,
}
#[derive(Debug)]
pub struct GpuError(pub String);
impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for GpuError {}
#[derive(Default, Debug, Clone, Copy)]
pub struct GpuStats {
    pub draw_calls: usize,
    /// Encoded render passes, including repainted isolated layers; excludes presentation.
    pub render_passes: usize,
    /// Filter passes, including the final composite, included in render_passes.
    pub blur_passes: usize,
    pub blur_cache_hits: usize,
    pub blur_texture_allocations: usize,
    /// Physical output pixels whose filter input was recomputed (excludes cache hits).
    pub filtered_pixels: u64,
    pub instances: usize,
    pub shaped_nodes: usize,
    pub glyph_uploads: usize,
    pub image_uploads: usize,
    pub shader_dispatches: usize,
    pub shader_resource_allocations: usize,
    pub effect_stage_cache_hits: usize,
    /// Native GPU imports/conversions, never CPU image uploads.
    pub native_surface_imports: usize,
    pub native_surface_conversions: usize,
    pub canvas_rasterizations: usize,
    pub svg_rasterizations: usize,
    pub geometry_rebuilds: usize,
    pub vertex_buffer_allocations: usize,
    pub paint_geometry_buffer_allocations: usize,
    pub damaged_pixels: u64,
    pub layer_repaints: usize,
    pub layer_cache_hits: usize,
    pub layer_texture_allocations: usize,
    /// Scrolled regions shifted by copying instead of redrawn.
    pub scroll_copies: usize,
    /// Device pixels transferred by retained scroll operations (one read/write each).
    pub copied_pixels: u64,
    pub scroll_phase_hits: usize,
}
/// Retained allocation counters for diagnostics; excludes driver/font database allocations.
#[derive(Debug, Clone, Copy)]
pub struct DebugCacheStats {
    pub shaped_nodes: usize,
    pub shaped_bytes: usize,
    pub cached_quads: usize,
    pub atlas_entries: usize,
    pub atlas_bytes: usize,
    pub swash_images: usize,
    pub swash_outlines: usize,
    pub image_textures: usize,
    pub image_bytes: usize,
    /// Includes persistent shader resources also referenced by image_bytes.
    pub shader_resource_bytes: usize,
    /// Includes chain outputs also referenced by image_bytes.
    pub effect_chain_bytes: usize,
    pub blur_cache_bytes: usize,
    pub blur_scratch_bytes: usize,
    pub canvas_raster_bytes: usize,
    pub svg_raster_bytes: usize,
    pub vertex_buffer_bytes: usize,
    pub paint_geometry_bytes: usize,
    pub layer_textures: usize,
    pub layer_bytes: usize,
    /// One older fractional-scroll origin; capped at 64 MiB.
    pub scroll_cache_bytes: usize,
}
const SHAPE_BUDGET: usize = 8 * 1024 * 1024;
/// Shaping frames a cached word survives without being used.
const SHAPE_RUN_AGES: u64 = 2;
/// Shaping calls after which the word cache starts a fresh table (see
/// `render`).
const SHAPE_RESET: u64 = 512;
const VERTEX_BUDGET: usize = 32 * 1024 * 1024;
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Quad {
    rect: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    fade: [f32; 4],
    options: [f32; 4],
    shape: [f32; 4],
    border: [f32; 4],
    /// A fade mask with axis bounds and signed edge bands from a
    /// `fade_edges` ancestor; zero bands leave the quad as is.
    mask: [f32; 4],
}
const NO_MASK: [f32; 4] = [0.; 4];
fn fade_parameters(rect: Rect, bands: [f32; 2]) -> [f32; 4] {
    let [start, end] = bands;
    if start < 0. || end < 0. {
        [rect.x, rect.x + rect.width, start, end]
    } else {
        [rect.y, rect.y + rect.height, start, end]
    }
}
/// How a quad is drawn, where drawing is split (see `push_draw`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shading {
    /// Original full shader, including Gaussian shadows; also the A/B reference.
    Full,
    /// All full-shader features except Gaussian shadows.
    RegularFull,
    /// Nearest premultiplied images, retaining transforms, fades and all clips.
    NearestImage,
    /// Colour, or colour times a texture sample: `basic`.
    Basic,
    /// An opaque flat colour: `basic` without blending, which reads every
    /// pixel it writes. Same pixels.
    Opaque,
}
fn selected_shading(shading: Shading, regular_full: bool, nearest_image: bool) -> Shading {
    match shading {
        Shading::RegularFull | Shading::NearestImage if !regular_full => Shading::Full,
        Shading::NearestImage if !nearest_image => Shading::RegularFull,
        _ => shading,
    }
}
fn nearest_image(quad: &Quad) -> bool {
    // These lanes also encode shape corners/shadows. Only specialize when
    // the original shader cannot enter either shape or outer-shadow coverage.
    quad.options[1] == 2.
        && quad.options[3] == 1.
        && (quad.options[2] >= 0.5 || (quad.shape[2] <= 0. && quad.shape[3] <= 0.5))
}
fn full_shading(quads: &[Quad]) -> Shading {
    if quads.iter().any(gaussian_shadow) {
        Shading::Full
    } else if quads.iter().all(nearest_image) {
        Shading::NearestImage
    } else {
        Shading::RegularFull
    }
}
fn gaussian_shadow(quad: &Quad) -> bool {
    quad.options[2] < 0.5 && quad.shape[2] >= 0.01
}
fn shading(quad: &Quad, _scale: f32) -> Shading {
    // The basic shader preserves square-panel coverage at fractional pixels,
    // so smooth scrolling does not force backgrounds through the full shader.
    let square = quad.shape[3] > 0.5
        && quad.shape[0] <= 0.
        && quad.shape[1] <= 0.
        && quad.options[3] < 0.5
        && quad.rect == quad.fade;
    let plain = quad.options[0] == 0.
        && !(quad.options[1] > 1.5 && quad.options[3] > 0.5)
        && quad.options[2] < 0.5
        && quad.shape[2] <= 0.
        && (quad.shape[3] <= 0.5 || square)
        && quad.mask[2] == 0.
        && quad.mask[3] == 0.;
    if !plain {
        if gaussian_shadow(quad) {
            Shading::Full
        } else {
            Shading::RegularFull
        }
    } else if quad.options[1] <= 0.5 && quad.color[3] >= 1. && !square {
        Shading::Opaque
    } else {
        Shading::Basic
    }
}
/// The quad pipelines, each compiled on first use: a software rasterizer
/// compiles a pipeline into megabytes of code, and plain interfaces never
/// need the full shader there.
struct QuadPipelines {
    device: wgpu::Device,
    shader: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    full: [std::cell::OnceCell<wgpu::RenderPipeline>; 2],
    regular_full: [std::cell::OnceCell<wgpu::RenderPipeline>; 2],
    nearest_image: [std::cell::OnceCell<wgpu::RenderPipeline>; 2],
    full_clear: std::cell::OnceCell<wgpu::RenderPipeline>,
    basic: std::cell::OnceCell<wgpu::RenderPipeline>,
    basic_clear: std::cell::OnceCell<wgpu::RenderPipeline>,
    drawable: Option<wgpu::TextureFormat>,
}
impl QuadPipelines {
    /// The pipeline drawing quads of `shading`.
    fn get(&self, shading: Shading, clip_interiors: bool) -> &wgpu::RenderPipeline {
        let variant = usize::from(clip_interiors);
        let (cell, blend) = match shading {
            Shading::Full => (&self.full[variant], true),
            Shading::RegularFull => (&self.regular_full[variant], true),
            Shading::NearestImage => (&self.nearest_image[variant], true),
            Shading::Basic => (&self.basic, true),
            Shading::Opaque => (&self.basic_clear, false),
        };
        cell.get_or_init(|| {
            quad_pipeline(
                &self.device,
                &self.shader,
                &self.layout,
                blend,
                self.drawable,
                shading,
                clip_interiors,
            )
        })
    }
    /// The pipeline clearing a region to the background quad.
    fn clear(&self, basic: bool) -> &wgpu::RenderPipeline {
        if basic {
            self.get(Shading::Opaque, false)
        } else {
            self.full_clear.get_or_init(|| {
                quad_pipeline(
                    &self.device,
                    &self.shader,
                    &self.layout,
                    false,
                    self.drawable,
                    Shading::Full,
                    false,
                )
            })
        }
    }
}
/// Damage regions merge when their union adds at most half their area and
/// this many device pixels (see `merge_damage`).
const NEAR_PIXELS: f32 = 256.;
/// Damaged regions of at least this many device pixels draw opaque quads
/// without blending (see `encode_draws`).
const SPLIT_PIXELS: f32 = 32_768.;
/// Keep SDF coverage/borders in the rim; large constant interiors need neither
/// antialiasing math nor a framebuffer read for blending. Partition on device
/// pixel boundaries so the interior and rim never shade the same pixel.
fn opaque_interior(quads: &mut Vec<Quad>, start: u32, scale: f32, clip: Rect, damage: &[Rect]) {
    if quads.len() <= start as usize {
        return;
    }
    let q = *quads.last().unwrap();
    if q.color[3] < 1.
        || q.rect != q.fade
        || q.shape[3] <= 0.5
        || q.shape[2] > 0.
        || q.options[0] != 0.
        || q.options[1] != 0.
        || q.options[2] >= 0.5
        || q.mask != NO_MASK
    {
        return;
    }
    let radius = if q.options[3] > 0.5 {
        q.border.into_iter().fold(0_f32, f32::max)
    } else {
        q.shape[0].max(0.)
    };
    // The shader clamps its AA width to at least 0.5 logical pixels.
    let inset = radius + q.shape[1].max(0.) + (1. / scale).max(0.5);
    let (left, top) = (
        ((q.rect[0] + inset) * scale).ceil() / scale,
        ((q.rect[1] + inset) * scale).ceil() / scale,
    );
    let (right, bottom) = (
        ((q.rect[0] + q.rect[2] - inset) * scale).floor() / scale,
        ((q.rect[1] + q.rect[3] - inset) * scale).floor() / scale,
    );
    if right <= left
        || bottom <= top
        || (right - left) * (bottom - top) * scale * scale < SPLIT_PIXELS
    {
        return;
    }
    let center = Rect::new(left, top, right - left, bottom - top);
    let Some(visible) = center.intersection(clip) else {
        return;
    };
    let pixels = damage
        .iter()
        .filter_map(|d| visible.intersection(*d))
        .map(|r| r.width * r.height * scale * scale)
        .sum::<f32>();
    // Thin scroll strips do not justify more runs/pipeline switches.
    if pixels < SPLIT_PIXELS {
        return;
    }
    *quads.last_mut().unwrap() = Quad {
        rect: [left, top, center.width, center.height],
        options: [0.; 4],
        shape: [0.; 4],
        border: [0.; 4],
        ..q
    };
    for rim in rect_outside(
        Rect::new(q.rect[0], q.rect[1], q.rect[2], q.rect[3]),
        center,
    ) {
        quads.push(Quad {
            rect: [rim.x, rim.y, rim.width, rim.height],
            ..q
        });
    }
}
/// Split a draw's quads into runs of one shading, in paint order, when
/// `split`: software and integrated GPUs benefit from lighter shaders for
/// backgrounds; discrete GPUs favor fewer draws through the full shader.
fn push_draw(draws: &mut Vec<Draw>, quads: &[Quad], draw: Draw, split: bool, scale: f32) {
    if !split || draw.end <= draw.start {
        draws.push(draw);
        return;
    }
    if draw.geometry_offset != 0 || draw.blur.is_some() || draw.layer.is_some() {
        // Preserve retained world bounds and bindings; Basic cannot read geometry.
        let shading = full_shading(&quads[draw.start as usize..draw.end as usize]);
        draws.push(Draw { shading, ..draw });
        return;
    }
    let mut start = draw.start;
    while start < draw.end {
        let kind = shading(&quads[start as usize], scale);
        let mut end = start + 1;
        while end < draw.end && shading(&quads[end as usize], scale) == kind {
            end += 1;
        }
        draws.push(Draw {
            start,
            end,
            bounds: quad_bounds(&quads[start as usize..end as usize]),
            // Classify the existing run without introducing new boundaries.
            shading: if kind == Shading::RegularFull {
                full_shading(&quads[start as usize..end as usize])
            } else {
                kind
            },
            ..draw.clone()
        });
        start = end;
    }
}
fn texel_at(texture: &wgpu::Texture, x: u32, y: u32) -> wgpu::TexelCopyTextureInfo<'_> {
    wgpu::TexelCopyTextureInfo {
        texture,
        mip_level: 0,
        origin: wgpu::Origin3d { x, y, z: 0 },
        aspect: wgpu::TextureAspect::All,
    }
}
/// Repair needed to move `scroll`'s pixels exactly. Content is contiguous,
/// free of stationary masks and backdrop filters, over an opaque rectangle
/// or the window background. Foreground pixels and their shifted ghosts are
/// repainted after the copy.
struct ScrollRepair {
    regions: Vec<Rect>,
    foreground: Vec<Rect>,
}
fn copy_repair(
    scene: &Scene,
    scroll: &lumine::scene::ScrollMove,
    damage: &[Rect],
) -> Option<ScrollRepair> {
    let clip = scroll.clip;
    let items = scene.layer_items_within(None, Some(&[clip]));
    if items.iter().any(|item| {
        item.transform != lumine::affine::Affine::IDENTITY
            || !item.clip_regions.is_empty()
            || item.fade_transform != lumine::affine::Affine::IDENTITY
    }) {
        return None;
    }
    let content = |item: &lumine::scene::PaintItem<'_>| scene.is_within(item.id, scroll.node);
    let first = items.iter().position(content)?;
    let last = items.iter().rposition(content).expect("found above");
    let paints = |item: &lumine::scene::PaintItem<'_>| scroll_item_bounds(scene, item, clip);
    let repainted = |rect: Rect| {
        damage.iter().any(|d| {
            d.x <= rect.x
                && d.y <= rect.y
                && d.x + d.width >= rect.x + rect.width
                && d.y + d.height >= rect.y + rect.height
        })
    };
    for item in &items[first..=last] {
        if !content(item) || item.mask.is_some() || item.effects.blur_radius > 0. {
            return None;
        }
    }
    let mut repair = Vec::new();
    let foreground: Vec<_> = items[last + 1..].iter().filter_map(paints).collect();
    for rect in foreground.iter().copied() {
        for rect in [
            Some(rect),
            Rect::new(
                rect.x + scroll.dx,
                rect.y + scroll.dy,
                rect.width,
                rect.height,
            )
            .intersection(clip),
        ]
        .into_iter()
        .flatten()
        {
            if !repainted(rect) {
                repair.push(rect);
            }
        }
    }
    for item in items[..first].iter().rev() {
        let Some(rect) = paints(item) else {
            continue;
        };
        if let NodeKind::Quad(quad) | NodeKind::Panel { quad, .. } = item.kind
            && quad.fill.3 == 255
            && quad.radius <= 0.
            && quad.border_width <= 0.
            && quad.shadow.is_none()
            && quad.decoration.is_none()
            && item.effects.opacity >= 1.
            && item.effects.edge_fade <= 0.
            && item.mask.is_none()
            && !item.isolated
            && rect == clip
        {
            return Some(ScrollRepair {
                regions: repair,
                foreground,
            });
        }
        if !repainted(rect) {
            return None;
        }
    }
    Some(ScrollRepair {
        regions: repair,
        foreground,
    })
}
fn scroll_item_bounds(
    scene: &Scene,
    item: &lumine::scene::PaintItem<'_>,
    clip: Rect,
) -> Option<Rect> {
    let bounds = if item.isolated {
        scene.layer_bounds(item.id)
    } else {
        match item.kind {
            NodeKind::Container(_) if item.effects.blur_radius <= 0. => return None,
            NodeKind::Quad(quad) | NodeKind::Panel { quad, .. } => quad.paint_bounds(item.bounds),
            NodeKind::Svg(svg) => svg.paint_bounds(item.bounds),
            NodeKind::Image(image) => image.paint_bounds(item.bounds),
            _ => item.bounds,
        }
    };
    if item.effects.opacity <= 0. {
        return None;
    }
    let bounds = if item.isolated {
        bounds
    } else {
        item.transform.bounds(bounds)
    };
    bounds.intersection(item.clip?)?.intersection(clip)
}
/// Positioned glyphs of laid-out runs; rich text takes each glyph's colour
/// from its run (it is shaped without colours, see `rich_buffer_raw`).
fn glyphs_of<'a>(
    runs: impl Iterator<Item = cosmic_text::LayoutRun<'a>>,
    starts: &[usize],
    rich: Option<&lumine::rich_text::RichText>,
    font: &lumine::text_layout::FontStyle,
    scale: f32,
) -> Vec<Glyph> {
    let mut glyphs = Vec::new();
    for run in runs {
        for glyph in run.glyphs {
            let p = glyph.physical((0., 0.), scale);
            let run_index = rich.map_or(0, |rich| {
                text::run_at(rich, starts[run.line_i] + glyph.start)
            });
            let line_height = rich.map_or(font.line_height, |rich| {
                rich.runs()[run_index].font.line_height
            });
            glyphs.push(Glyph {
                key: p.cache_key,
                color: match rich {
                    Some(rich) => Some(rich.runs()[run_index].color),
                    None => glyph
                        .color_opt
                        .map(|c| lumine::scene::Color(c.r(), c.g(), c.b(), c.a())),
                },
                run: run_index,
                x: (glyph.x + glyph.font_size * glyph.x_offset) * scale,
                y: text::glyph_y(glyph, run.line_y + line_height.baseline_offset()) * scale,
            });
        }
    }
    glyphs
}
/// A pixel rectangle: x, y, width, height.
type Scissor = (u32, u32, u32, u32);
/// A scroll copy's destination and the offset of its source.
type ScrollCopy = (Scissor, (i32, i32));
fn scroll_pixels(clip: Rect, dx: f32, dy: f32, scale: f32) -> Option<ScrollCopy> {
    let whole = |v: f32| {
        let v = v * scale;
        ((v - v.round()).abs() < 1e-3).then_some(v.round() as i32)
    };
    let (x, y, right, bottom) = (
        whole(clip.x)?,
        whole(clip.y)?,
        whole(clip.x + clip.width)?,
        whole(clip.y + clip.height)?,
    );
    let (dx, dy) = (whole(dx)?, whole(dy)?);
    let (x0, x1, y0, y1) = (
        x.max(x + dx),
        right.min(right + dx),
        y.max(y + dy),
        bottom.min(bottom + dy),
    );
    (x0 >= 0 && y0 >= 0 && x0 < x1 && y0 < y1).then_some((
        (x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32),
        (-dx, -dy),
    ))
}
fn rect_outside(rect: Rect, dest: Rect) -> Vec<Rect> {
    [
        Rect::new(rect.x, rect.y, rect.width, dest.y - rect.y),
        Rect::new(
            rect.x,
            dest.y + dest.height,
            rect.width,
            rect.y + rect.height - dest.y - dest.height,
        ),
        Rect::new(rect.x, dest.y, dest.x - rect.x, dest.height),
        Rect::new(
            dest.x + dest.width,
            dest.y,
            rect.x + rect.width - dest.x - dest.width,
            dest.height,
        ),
    ]
    .into_iter()
    .filter(|r| r.width > 0. && r.height > 0.)
    .collect()
}
/// Partition the surface outside scroll destinations without overlapping copies.
fn unchanged_scissors(width: u32, height: u32, copies: &[ScrollCopy]) -> Vec<Scissor> {
    let mut regions = vec![(0, 0, width, height)];
    for &((cx, cy, cw, ch), _) in copies {
        regions = regions
            .into_iter()
            .flat_map(|(x, y, w, h)| {
                let (left, top, right, bottom) = (
                    x.max(cx),
                    y.max(cy),
                    (x + w).min(cx + cw),
                    (y + h).min(cy + ch),
                );
                if left >= right || top >= bottom {
                    return vec![(x, y, w, h)];
                }
                [
                    (x, y, w, top - y),
                    (x, bottom, w, y + h - bottom),
                    (x, top, left - x, bottom - top),
                    (right, top, x + w - right, bottom - top),
                ]
                .into_iter()
                .filter(|r| r.2 > 0 && r.3 > 0)
                .collect()
            })
            .collect();
    }
    regions
}
/// For each damage region, the index of the topmost quad that paints every
/// pixel of the region opaque: a plain solid rectangle (no texture, radius,
/// border, shadow, fade, mask or transform) at full alpha, unclipped over the
/// region's scissor. Nothing drawn before it, including the clear, can show.
/// 0 when no quad qualifies.
fn occluded_from(
    quads: &[Quad],
    draws: &[Draw],
    regions: &[(Rect, Scissor)],
    scale: f32,
) -> Vec<u32> {
    let covers = |outer: Rect, inner: Rect| {
        outer.x <= inner.x
            && outer.y <= inner.y
            && outer.x + outer.width >= inner.x + inner.width
            && outer.y + outer.height >= inner.y + inner.height
    };
    let plain = |q: &Quad| {
        q.color[3] >= 1.
            && q.options == [0.; 4]
            && q.shape[0] <= 0.
            && q.shape[1] <= 0.
            && q.shape[2] <= 0.
            && q.mask == NO_MASK
    };
    regions
        .iter()
        .map(|(_, (x, y, w, h))| {
            // The pixels the scissor lets through, in logical units.
            let pixels = Rect::new(
                *x as f32 / scale,
                *y as f32 / scale,
                *w as f32 / scale,
                *h as f32 / scale,
            );
            for draw in draws.iter().rev() {
                if draw.geometry_offset != 0
                    || draw.image.is_some()
                    || draw.layer.is_some()
                    || draw.blur.is_some()
                    || !covers(draw.bounds, pixels)
                    || !covers(draw.clip, pixels)
                {
                    continue;
                }
                for index in (draw.start..draw.end).rev() {
                    let q = &quads[index as usize];
                    let rect = Rect::new(q.rect[0], q.rect[1], q.rect[2], q.rect[3]);
                    let opaque_rect = if q.shape[3] > 0.5 {
                        // SDF coverage can be partial at square corners too,
                        // including whole-pixel panels on hardware GPUs.
                        Rect::new(
                            rect.x + 1. / scale,
                            rect.y + 1. / scale,
                            rect.width - 2. / scale,
                            rect.height - 2. / scale,
                        )
                    } else {
                        rect
                    };
                    if plain(q) && covers(opaque_rect, pixels) {
                        return index;
                    }
                }
            }
            0
        })
        .collect()
}
#[derive(Clone)]
struct Glyph {
    key: CacheKey,
    color: Option<lumine::scene::Color>,
    /// The rich run it was shaped from, to recolour paint-only changes.
    run: usize,
    x: f32,
    y: f32,
}
impl Glyph {
    fn at(&self, origin: (f32, f32)) -> (CacheKey, i32, i32) {
        CacheKey::new(
            self.key.font_id,
            self.key.glyph_id,
            f32::from_bits(self.key.font_size_bits),
            (self.x + origin.0, self.y + origin.1),
            self.key.font_weight,
            self.key.flags,
        )
    }
}
#[cfg(test)]
mod glyph_baseline_tests {
    use super::*;
    #[test]
    fn rounded_logical_baselines_preserve_scaled_origin_phase() {
        let (key, _, _) = CacheKey::new(
            cosmic_text::fontdb::ID::dummy(),
            1,
            11.,
            (0.25, 0.),
            cosmic_text::Weight::NORMAL,
            cosmic_text::CacheKeyFlags::empty(),
        );
        let glyph = Glyph {
            key,
            color: None,
            run: 0,
            x: 0.25,
            // collect_glyphs rounds the logical line baseline before scaling.
            y: 11.,
        };
        for (origin, expected, phase) in [
            (0., 11, cosmic_text::SubpixelBin::Zero),
            (0.5, 11, cosmic_text::SubpixelBin::Two),
            (-11.5, -1, cosmic_text::SubpixelBin::Two),
        ] {
            let (raster, _, baseline) = glyph.at((0., origin));
            assert_eq!(baseline, expected);
            assert_eq!(raster.flags, key.flags);
            assert_eq!(raster.font_size_bits, key.font_size_bits);
            assert_eq!(raster.x_bin, key.x_bin);
            assert_eq!(raster.y_bin, phase);
        }
        let scaled = Glyph {
            y: glyph.y * 1.5,
            ..glyph.clone()
        };
        let (raster, _, baseline) = scaled.at((0., 0.));
        assert_eq!(baseline, 16);
        assert_eq!(raster.y_bin, cosmic_text::SubpixelBin::Two);
    }
}
struct Shaped {
    rich: Option<Arc<lumine::rich_text::RichText>>,
    text_options: lumine::text_layout::TextOptions,
    decorations: Vec<text::RichDecoration>,
    font: lumine::text_layout::FontStyle,
    text: Arc<str>,
    size: f32,
    width: f32,
    height: f32,
    unwrapped: bool,
    glyphs: Vec<Glyph>,
    quads: Vec<Quad>,
    atlas_epoch: u64,
    /// Fractional physical node origin used for the cached glyph rasters.
    raster_origin: (f32, f32),
    last_used: u64,
    /// The prepared text these glyphs were laid out from, kept while drawn.
    prepared: Option<u64>,
}
impl Shaped {
    fn bytes(&self) -> usize {
        self.rich
            .as_ref()
            .map_or(self.text.len(), |r| r.storage_bytes())
            + self.font.storage_bytes()
            + self.decorations.capacity() * std::mem::size_of::<text::RichDecoration>()
            + self.glyphs.capacity() * std::mem::size_of::<Glyph>()
            + self.quads.capacity() * std::mem::size_of::<Quad>()
    }
}
#[derive(Clone, Copy)]
struct AtlasEntry {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    left: i32,
    top: i32,
}
struct LayerCache {
    texture: wgpu::Texture,
    bind: wgpu::BindGroup,
    bounds: Rect,
    revision: u64,
    root_origin: (f32, f32),
    bytes: usize,
    /// The content's extent in texture coordinates.
    uv: [f32; 2],
}
#[derive(Clone)]
struct Draw {
    diagnostic_source: Option<draw_diagnostics::Source>,
    transform: lumine::affine::Affine,
    geometry_offset: u32,
    geometry_hash: u64,
    geometry: Option<wgpu::BindGroup>,
    /// How its quads are drawn; set by `push_draw`.
    shading: Shading,
    blur: Option<(NodeId, Rect, lumine::scene::Effects)>,
    blur_mask: Option<lumine::scene::FadeMask>,
    start: u32,
    end: u32,
    clip: Rect,
    /// Union of this draw's quads: nothing outside it is ever touched.
    bounds: Rect,
    image: Option<u64>,
    layer: Option<NodeId>,
}
/// A frame's damage, encoded at presentation so it can be drawn into the
/// next retained target and the drawable in one pass (see `present_single_pass`).
struct Deferred {
    geometry: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    draws: Vec<Draw>,
    regions: Vec<(Rect, (u32, u32, u32, u32))>,
    viewport: Option<wgpu::BindGroup>,
    /// The last filter's vertical half, drawn before `draws`.
    blur: Option<VerticalBlur>,
    /// Clear each region first. False for the tail after a backdrop filter:
    /// the passes before it already cleared and drew the regions.
    clear: bool,
    /// No two regions share a pixel (`merge_damage`), so every clear can
    /// precede every draw without redrawing an overlap twice.
    disjoint: bool,
    /// Per region, the first quad to draw (see `occluded_from`).
    first: Vec<u32>,
}
/// A backdrop filter's vertical half: one draw per damaged output, reading the
/// horizontal pass's scratch textures (see `GpuRenderer::blur`).
#[derive(Clone)]
struct VerticalBlur {
    geometry: Option<(wgpu::BindGroup, u32)>,
    bind: wgpu::BindGroup,
    scissors: Vec<(u32, u32, u32, u32)>,
    prepasses: usize,
    predraws: usize,
    allocations: usize,
}
/// Separable blur pipelines. The horizontal pass reads the target and keeps
/// its original pixels; the vertical draws write the target, or the target
/// and a drawable of `both`'s format at presentation.
struct BlurPipelines {
    geometry_layout: Option<wgpu::BindGroupLayout>,
    shader: wgpu::ShaderModule,
    /// Shared by every vertical pipeline so one bind group serves each.
    layout: wgpu::BindGroupLayout,
    horizontal: wgpu::RenderPipeline,
    /// Single-target horizontal and original crop draws for unequal extents.
    crop: Option<(wgpu::RenderPipeline, wgpu::RenderPipeline)>,
    vertical: wgpu::RenderPipeline,
    both: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    weights: FxHashMap<u32, wgpu::Buffer>,
    sampler: wgpu::Sampler,
}
impl BlurPipelines {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("backdrop blur"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blur.wgsl").into()),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vertical blur"),
            entries: &[
                texture(0),
                texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let target = Some(wgpu::ColorTargetState {
            format: FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        });
        let horizontal = Self::pipeline(
            device,
            &shader,
            Some(&layout),
            "fs_split",
            None,
            &[target.clone(), target.clone()],
        );
        let vertical = Self::pipeline(
            device,
            &shader,
            Some(&layout),
            "fs",
            None,
            std::slice::from_ref(&target),
        );
        Self {
            geometry_layout: None,
            shader,
            layout,
            horizontal,
            crop: None,
            vertical,
            both: None,
            weights: Default::default(),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
        }
    }
    fn crop(&mut self, device: &wgpu::Device) {
        if self.crop.is_some() {
            return;
        }
        let target = [Some(wgpu::ColorTargetState {
            format: FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        self.crop = Some((
            Self::pipeline(
                device,
                &self.shader,
                Some(&self.layout),
                "fs_horizontal",
                None,
                &target,
            ),
            Self::pipeline(
                device,
                &self.shader,
                Some(&self.layout),
                "fs_original",
                None,
                &target,
            ),
        ));
    }
    fn pipeline(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        layout: Option<&wgpu::BindGroupLayout>,
        entry: &str,
        geometry_layout: Option<&wgpu::BindGroupLayout>,
        targets: &[Option<wgpu::ColorTargetState>],
    ) -> wgpu::RenderPipeline {
        let layout = layout.map(|layout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("vertical blur"),
                bind_group_layouts: &([Some(layout)]
                    .into_iter()
                    .chain(geometry_layout.map(Some))
                    .collect::<Vec<_>>()),
                immediate_size: 0,
            })
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("backdrop blur"),
            layout: layout.as_ref(),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets,
            }),
            multiview_mask: None,
            cache: None,
        })
    }
    /// The vertical pipeline writing the target and a `format` drawable.
    fn both(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        if self.both.as_ref().is_none_or(|(f, _)| *f != format) {
            let target = |format| {
                Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })
            };
            let pipeline = Self::pipeline(
                device,
                &self.shader,
                Some(&self.layout),
                "fs_both",
                self.geometry_layout.as_ref(),
                &[target(FORMAT), target(format)],
            );
            self.both = Some((format, pipeline));
        }
        &self.both.as_ref().expect("created above").1
    }
}
/// The spare retained target with the views and bind groups it needs.
struct Retained {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    copy_bind: Option<wgpu::BindGroup>,
    blit_bind: Option<wgpu::BindGroup>,
}
#[derive(Clone)]
struct ScrollState {
    node: NodeId,
    clip: Rect,
    origin: (f32, f32),
    revision: (u64, u64, u64),
    foreground: Vec<Rect>,
    repair: Vec<Rect>,
}
struct ScrollHistory {
    retained: Retained,
    state: ScrollState,
}
/// Pipelines writing the retained target and a drawable of `format` at once.
struct SinglePass {
    format: wgpu::TextureFormat,
    quads: QuadPipelines,
    copy: wgpu::RenderPipeline,
}
/// A run of draws encoded as one instanced call.
struct Batch {
    geometry_offset: u32,
    geometry: Option<wgpu::BindGroup>,
    rect: Rect,
    source: (Option<u64>, Option<NodeId>),
    shading: Shading,
    start: u32,
    end: u32,
}
fn quad_bounds(quads: &[Quad]) -> Rect {
    quads
        .iter()
        .map(quad_paint_bounds)
        .reduce(|a, b| a.union(b))
        .unwrap_or_default()
}
fn quad_paint_bounds(q: &Quad) -> Rect {
    let bounds = Rect::new(q.rect[0], q.rect[1], q.rect[2], q.rect[3]);
    if q.options[1] > 1.5 && q.options[2] > 0.5 {
        lumine::affine::Affine {
            a: q.shape[0],
            c: q.shape[1],
            b: q.shape[2],
            d: q.shape[3],
            tx: q.border[0],
            ty: q.border[1],
        }
        .bounds(bounds)
    } else {
        bounds
    }
}
/// Shapes using exactly the same font selection and line metrics as the renderer.
pub fn measure_text(
    fonts: &mut FontSystem,
    text: &str,
    size: f32,
    width: Option<f32>,
) -> (f32, f32) {
    text::ShapedText::new(fonts, text, size, width).size()
}
/// Shared native GPU device, pipelines, and font database. Clone this cheaply for
/// additional windows; render targets/atlases remain owned by each renderer.
#[derive(Clone)]
pub struct GpuContext {
    inner: Rc<GpuContextInner>,
}
struct GpuContextInner {
    geometry_layout: wgpu::BindGroupLayout,
    indexed_geometry: std::cell::OnceCell<IndexedGeometryPipelines>,
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: wgpu::AdapterInfo,
    fonts: Rc<RefCell<FontSystem>>,
    /// Shaped text shared by layout measurement and every renderer.
    text_cache: Rc<RefCell<prepared::TextCache>>,
    quads: QuadPipelines,
    /// Bind group layouts of the quad pipelines: texture and sampler, viewport.
    texture_layout: wgpu::BindGroupLayout,
    viewport_layout: wgpu::BindGroupLayout,
    /// Vertex and small uniform uploads are CPU-mapped instead of copied.
    mapped_uploads: bool,
}
impl GpuContext {
    pub fn new() -> Result<Self, GpuError> {
        Self::create(None)
    }
    fn create(window: Option<Arc<winit::window::Window>>) -> Result<Self, GpuError> {
        // Avoid initializing secondary graphics stacks merely to enumerate
        // adapters. On Linux this can create an unused EGL driver and worker
        // pool even when Vulkan is selected; macOS uses Metal.
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::PRIMARY;
        let instance = wgpu::Instance::new(descriptor.with_env());
        let surface = window
            .map(|window| instance.create_surface(window))
            .transpose()
            .map_err(|e| GpuError(e.to_string()))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: surface.as_ref(),
            force_fallback_adapter: false,
        }))
        .map_err(|e| GpuError(e.to_string()))?;
        let info = adapter.get_info();
        // CPU-written vertex buffers avoid a GPU copy per frame; only worth it
        // where CPU and GPU share memory.
        let mapped_uploads = info.device_type == wgpu::DeviceType::IntegratedGpu
            && adapter
                .features()
                .contains(wgpu::Features::MAPPABLE_PRIMARY_BUFFERS);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("lumine shared device"),
            required_features: (adapter.features()
                & (wgpu::Features::TIMESTAMP_QUERY
                    | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS))
                | if mapped_uploads {
                    wgpu::Features::MAPPABLE_PRIMARY_BUFFERS
                } else {
                    wgpu::Features::empty()
                },
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        }))
        .map_err(|e| GpuError(e.to_string()))?;
        let (texture_layout, viewport_layout, geometry_layout, quad_shader, quad_layout) =
            create_quad_pipelines(&device, false);
        let quads = QuadPipelines {
            device: device.clone(),
            shader: quad_shader.clone(),
            layout: quad_layout.clone(),
            full: Default::default(),
            regular_full: Default::default(),
            nearest_image: Default::default(),
            full_clear: Default::default(),
            basic: Default::default(),
            basic_clear: Default::default(),
            drawable: None,
        };
        Ok(Self {
            inner: Rc::new(GpuContextInner {
                instance,
                adapter,
                device,
                queue,
                info,
                fonts: Rc::new(RefCell::new(FontSystem::new())),
                text_cache: Default::default(),
                quads,
                texture_layout,
                viewport_layout,
                geometry_layout,
                indexed_geometry: Default::default(),
                mapped_uploads,
            }),
        })
    }
    fn indexed_geometry(&self) -> &IndexedGeometryPipelines {
        self.inner.indexed_geometry.get_or_init(|| {
            let (_, _, geometry_layout, shader, layout) =
                create_quad_pipelines(&self.inner.device, true);
            IndexedGeometryPipelines {
                geometry_layout,
                quads: QuadPipelines {
                    device: self.inner.device.clone(),
                    shader,
                    layout,
                    full: Default::default(),
                    regular_full: Default::default(),
                    nearest_image: Default::default(),
                    full_clear: Default::default(),
                    basic: Default::default(),
                    basic_clear: Default::default(),
                    drawable: None,
                },
            }
        })
    }
    pub fn shares_device(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
    pub fn text_system(&self) -> Rc<RefCell<FontSystem>> {
        self.inner.fonts.clone()
    }
    /// Prepared (shaped once) rich text, shared with the renderers.
    pub fn text_cache(&self) -> Rc<RefCell<prepared::TextCache>> {
        self.inner.text_cache.clone()
    }
}
impl GpuError {
    pub fn is_surface_incompatible(&self) -> bool {
        self.0 == "surface is incompatible with the shared GPU adapter"
    }
}
/// Opt-in geometry transport experiment. The default retains dynamic uniforms.
/// With the `benchmark` feature, `LUMINE_GPU_GEOMETRY_BATCHING=indexed|merged`
/// selects the experiment when constructing a renderer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GeometryBatching {
    #[default]
    DynamicUniform,
    /// Storage lookups with exactly the original draw batches.
    Indexed,
    /// Storage lookups, merging only contiguous geometry-only batch boundaries.
    IndexedMerged,
}
impl GeometryBatching {
    fn configured() -> Self {
        #[cfg(feature = "benchmark")]
        match std::env::var("LUMINE_GPU_GEOMETRY_BATCHING").as_deref() {
            Ok("indexed") => return Self::Indexed,
            Ok("merged") => return Self::IndexedMerged,
            _ => {}
        }
        Self::DynamicUniform
    }
}
struct IndexedGeometryPipelines {
    geometry_layout: wgpu::BindGroupLayout,
    quads: QuadPipelines,
}

pub struct GpuRenderer {
    geometry_batching: GeometryBatching,
    geometry: paint_geometry::FrameGeometry,
    profiler: RefCell<Option<profiling::Profiler>>,
    context: GpuContext,
    scene_identity: Option<u64>,
    /// Scene content revision the per-node caches were last pruned at.
    pruned_revision: Option<u64>,
    /// The last render's still-open encoder. Present records its blit into it,
    /// so a frame is one command buffer and one submission. Flushed before
    /// anything that rewrites buffers or textures the commands read.
    pending: RefCell<Option<wgpu::CommandEncoder>>,
    /// Viewport uniforms by logical size (f32 bits). Each is written once at
    /// creation; layers and the window switch between them per pass.
    viewports: FxHashMap<(u32, u32), wgpu::BindGroup>,
    /// The raster each canvas, decoration or SVG node currently draws. A
    /// resized or restyled node gets a new raster; the old texture goes at
    /// once instead of waiting for a structural prune.
    node_images: FxHashMap<NodeId, u64>,
    /// Textures referenced by draws already assembled during this outer frame.
    frame_image_ids: std::collections::HashSet<u64>,
    /// The viewport of the target currently being rendered.
    viewport_bind: Option<wgpu::BindGroup>,
    /// Reused upload memory for per-frame vertices; `write_buffer` would
    /// allocate and free a staging buffer every frame.
    belt: RefCell<wgpu::util::StagingBelt>,
    /// Replaces the belt on unified-memory adapters.
    mapped: Option<RefCell<upload::MappedRing>>,
    /// Texture writes awaiting the next render's upload passes (mapped adapters).
    texture_uploads: upload::TextureUploads,
    instance: wgpu::Instance,
    window: Option<Arc<winit::window::Window>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: wgpu::AdapterInfo,
    surface: Option<wgpu::Surface<'static>>,
    config: Option<wgpu::SurfaceConfiguration>,
    width: u32,
    height: u32,
    image_budget: usize,
    scale: f32,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    /// Single-pass presentation alternates two retained targets: each frame
    /// copies `target` and draws its damage into `spare` and the drawable in
    /// one render pass, then swaps. Two passes (damage, then a present blit)
    /// cost a second Metal command buffer per frame. Dropped by `trim`.
    spare: Option<Retained>,
    /// Bind group sampling `target` for the single-pass copy; swapped with
    /// the spare's so neither is rebuilt per frame.
    target_copy_bind: Option<wgpu::BindGroup>,
    deferred: RefCell<Option<Deferred>>,
    single_pass: Option<SinglePass>,
    /// Non-sRGB view format of the drawable. Through it, drawing directly
    /// stores the same bytes the present blit would for opaque windows (whose
    /// pixels all have alpha 1, so post-multiplied presentation is a no-op).
    single_pass_format: Option<wgpu::TextureFormat>,
    /// Rendering an isolated layer's texture, never presented directly.
    in_layer: bool,
    /// The present blit converts to straight alpha (see `unpremultiplies`).
    /// Single-pass presentation then only matches for opaque windows.
    present_unpremultiplies: bool,
    /// Present by copying the retained target into the drawable: same
    /// format, no alpha conversion. A copy is a memcpy for a software
    /// rasterizer, where a full-window shader pass costs as much as drawing.
    copy_present: bool,
    /// Frames whose damage was drawn by `present_single_pass`.
    single_pass_frames: u64,
    /// Work encoded at presentation (single-pass frames), reported with the
    /// next `render`'s statistics.
    present_stats: GpuStats,
    /// Quads use the shader their features need (see `push_draw`).
    split_shading: bool,
    regular_full_shader: bool,
    nearest_image_shader: bool,
    clip_interiors: bool,
    /// Texts shaped since the word cache last started over, and the
    /// prepared text cache's count when last read.
    shaped_since_reset: u64,
    cache_shaped: u64,
    bind: wgpu::BindGroup,
    blit: Option<wgpu::RenderPipeline>,
    blit_bind: Option<wgpu::BindGroup>,
    atlas: wgpu::Texture,
    atlas_size: u32,
    entries: FxHashMap<CacheKey, Option<AtlasEntry>>,
    cursor: (u32, u32, u32),
    fonts: Rc<RefCell<FontSystem>>,
    text_cache: Rc<RefCell<prepared::TextCache>>,
    swash: SwashCache,
    layers: FxHashMap<NodeId, LayerCache>,
    layer_origin: (f32, f32),
    shapes: FxHashMap<NodeId, Shaped>,
    shaped_bytes: usize,
    fresh: bool,
    /// The scene flush the retained target was last rendered from.
    rendered_serial: Option<u64>,
    /// Scroll copies for the next root frame: destination pixel rectangle
    /// and the source offset (see `scroll_copies`).
    copies: Vec<ScrollCopy>,
    scroll_state: Option<ScrollState>,
    scroll_history: Option<ScrollHistory>,
    scroll_source: Option<wgpu::Texture>,
    retain_scroll_frame: bool,
    scroll_phase_cache: bool,
    opaque_interiors: bool,
    atlas_epoch: u64,
    frame: u64,
    vertices: wgpu::Buffer,
    vertex_capacity: usize,
    frame_quads: Vec<Quad>,
    background: Color,
    images: FxHashMap<u64, (wgpu::BindGroup, usize)>,
    procedural: procedural::Cache,
    chain_cache: effect_chain::Cache,
    image_textures: FxHashMap<u64, wgpu::Texture>,
    procedural_encoder: Option<wgpu::CommandEncoder>,
    #[cfg(target_os = "macos")]
    native_surfaces: Option<native_surface::SurfaceCache>,
    /// The next present joins the Core Animation transaction that resized the layer.
    #[cfg(target_os = "macos")]
    transaction_present: bool,
    /// Until when the layer keeps a third drawable after a resize (see
    /// `widen_drawables`).
    #[cfg(target_os = "macos")]
    wide_until: Option<std::time::Instant>,
    canvases: canvas::CanvasCache,
    svgs: svg::SvgCache,
    /// Small untransformed SVG rasters packed into the glyph atlas by image
    /// id, so icons batch with text instead of binding a texture each.
    image_cells: FxHashMap<u64, (u32, u32)>,
    /// Frame of the last atlas recycle, to detect thrashing.
    atlas_recycled_at: u64,
    /// Compiled on the first backdrop filter.
    blur_pipelines: Option<BlurPipelines>,
    blur_composite: Option<BlurPipelines>,
    blur_algorithm: BlurAlgorithm,
    kawase: Option<dual_kawase::Kawase>,
    blur_cache: FxHashMap<NodeId, blur_cache::CachedBlur>,
    /// Horizontal Gaussian intermediate; completed filters retain only raw
    /// and fully filtered pixels, and never sample this shared scratch.
    gaussian_scratch: Option<wgpu::Texture>,
}
impl GpuRenderer {
    pub fn new(width: u32, height: u32) -> Result<Self, GpuError> {
        let context = GpuContext::new()?;
        Self::create(None, width, height, &context)
    }
    pub fn for_window(window: Arc<winit::window::Window>) -> Result<Self, GpuError> {
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        let context = GpuContext::create(Some(window.clone()))?;
        let mut renderer = Self::create(Some(window), size.width, size.height, &context)?;
        renderer.set_scale_factor(scale);
        Ok(renderer)
    }
    pub fn context(&self) -> GpuContext {
        self.context.clone()
    }
    pub fn new_with_context(
        width: u32,
        height: u32,
        context: &GpuContext,
    ) -> Result<Self, GpuError> {
        Self::create(None, width, height, context)
    }
    pub fn for_window_with_context(
        window: Arc<winit::window::Window>,
        context: &GpuContext,
    ) -> Result<Self, GpuError> {
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        let mut renderer = Self::create(Some(window), size.width, size.height, context)?;
        renderer.set_scale_factor(scale);
        Ok(renderer)
    }
    fn create(
        window: Option<Arc<winit::window::Window>>,
        width: u32,
        height: u32,
        context: &GpuContext,
    ) -> Result<Self, GpuError> {
        let instance = context.inner.instance.clone();
        let device = context.inner.device.clone();
        let queue = context.inner.queue.clone();
        let adapter = &context.inner.adapter;
        let info = context.inner.info.clone();
        let surface = window
            .clone()
            .map(|window| instance.create_surface(window))
            .transpose()
            .map_err(|e| GpuError(e.to_string()))?;
        if surface
            .as_ref()
            .is_some_and(|surface| !adapter.is_surface_supported(surface))
        {
            return Err(GpuError(
                "surface is incompatible with the shared GPU adapter".into(),
            ));
        }
        let width = width.max(1);
        let height = height.max(1);
        let config = surface.as_ref().map(|s| {
            let mut c = s
                .get_default_config(adapter, width, height)
                .expect("supported surface");
            let caps = s.get_capabilities(adapter);
            c.alpha_mode = [
                wgpu::CompositeAlphaMode::PreMultiplied,
                wgpu::CompositeAlphaMode::PostMultiplied,
                wgpu::CompositeAlphaMode::Inherit,
                wgpu::CompositeAlphaMode::Opaque,
            ]
            .into_iter()
            .find(|mode| caps.alpha_modes.contains(mode))
            .unwrap_or(c.alpha_mode);
            c.present_mode = wgpu::PresentMode::AutoVsync;
            // Two drawables: measured no frame-rate or CPU cost against three
            // on Apple silicon, and one fewer window-sized buffer (~13 MB at
            // 5K) while animating. Resizing keeps a third (`widen_drawables`).
            if cfg!(target_os = "macos") {
                c.desired_maximum_frame_latency = 1;
            }
            // The retained target already holds sRGB-encoded bytes, which the
            // compositor reads the same from either format. A non-sRGB drawable
            // takes them as they are, where an sRGB one needs a per-frame
            // aliasing view and cannot be framebuffer-only.
            let plain = c.format.remove_srgb_suffix();
            if caps.formats.contains(&plain) {
                c.format = plain;
            } else if c.format.is_srgb() {
                c.view_formats = vec![plain];
            }
            // Off macOS, where drawables must stay framebuffer-only to be
            // cheap, present by copy when the surface allows it.
            if !cfg!(target_os = "macos")
                && c.format == FORMAT
                && !unpremultiplies(&c)
                && caps.usages.contains(wgpu::TextureUsages::COPY_DST)
            {
                c.usage |= wgpu::TextureUsages::COPY_DST;
            }
            s.configure(&device, &c);
            c
        });
        let copy_present = config
            .as_ref()
            .is_some_and(|c| c.usage.contains(wgpu::TextureUsages::COPY_DST));
        // Single-pass presentation writes the drawable in a non-sRGB format,
        // storing the retained target's bytes unchanged, exactly what the
        // present blit produces for opaque pixels.
        let single_pass_format = config.as_ref().map(|c| c.format.remove_srgb_suffix());
        let target = texture(&device, width, height, "retained output");
        let view = target.create_view(&Default::default());
        let atlas = texture(&device, 1, 1, "lazy glyph atlas placeholder");
        let split_shading = matches!(
            context.inner.info.device_type,
            wgpu::DeviceType::Cpu | wgpu::DeviceType::IntegratedGpu
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas"),
            layout: &context.inner.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &atlas.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(
                        &device.create_sampler(&wgpu::SamplerDescriptor::default()),
                    ),
                },
            ],
        });
        let present_unpremultiplies = config.as_ref().is_some_and(unpremultiplies);
        let blit = config
            .as_ref()
            .map(|c| blit_pipeline(&device, c.format, unpremultiplies(c)));
        let blit_bind = blit.as_ref().map(|p| blit_group(&device, p, &view));
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("retained frame vertices"),
            size: 4096,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let profiler = std::env::var_os("LUMINE_GPU_PROFILE")
            .and_then(|_| profiling::Profiler::new(&device, &queue));
        let geometry = paint_geometry::FrameGeometry::new(&device, &context.inner.geometry_layout);
        Ok(Self {
            geometry_batching: GeometryBatching::configured(),
            geometry,
            context: context.clone(),
            profiler: RefCell::new(profiler),
            scene_identity: None,
            pruned_revision: None,
            pending: RefCell::new(None),
            viewports: FxHashMap::default(),
            node_images: FxHashMap::default(),
            frame_image_ids: Default::default(),
            viewport_bind: None,
            belt: RefCell::new(wgpu::util::StagingBelt::new(device.clone(), 256 * 1024)),
            mapped: context
                .inner
                .mapped_uploads
                .then(|| RefCell::new(upload::MappedRing::new(&device))),
            texture_uploads: Default::default(),
            instance,
            window,
            device,
            queue,
            info,
            surface,
            config,
            width,
            height,
            image_budget: Self::image_budget_for(width, height),
            scale: 1.,
            target,
            view,
            split_shading,
            regular_full_shader: true,
            nearest_image_shader: true,
            clip_interiors: false,
            shaped_since_reset: 0,
            cache_shaped: 0,
            bind,
            blit,
            blit_bind,
            spare: None,
            target_copy_bind: None,
            deferred: RefCell::new(None),
            single_pass: None,
            single_pass_format,
            in_layer: false,
            present_unpremultiplies,
            copy_present,
            single_pass_frames: 0,
            present_stats: GpuStats::default(),
            atlas,
            atlas_size: 1,
            entries: FxHashMap::default(),
            cursor: (0, 0, 0),
            fonts: context.text_system(),
            text_cache: context.text_cache(),
            swash: SwashCache::new(),
            layers: FxHashMap::default(),
            layer_origin: (0., 0.),
            shapes: FxHashMap::default(),
            shaped_bytes: 0,
            fresh: true,
            rendered_serial: None,
            copies: Vec::new(),
            scroll_state: None,
            scroll_history: None,
            scroll_source: None,
            retain_scroll_frame: false,
            scroll_phase_cache: true,
            opaque_interiors: true,
            atlas_epoch: 1,
            frame: 0,
            vertices,
            vertex_capacity: 4096,
            frame_quads: Vec::new(),
            background: Color(0, 0, 0, 0),
            images: FxHashMap::default(),
            procedural: Default::default(),
            chain_cache: Default::default(),
            image_textures: Default::default(),
            procedural_encoder: None,
            #[cfg(target_os = "macos")]
            native_surfaces: None,
            #[cfg(target_os = "macos")]
            transaction_present: false,
            #[cfg(target_os = "macos")]
            wide_until: None,
            canvases: Default::default(),
            svgs: Default::default(),
            blur_pipelines: None,
            blur_composite: None,
            image_cells: FxHashMap::default(),
            // As if last recycled long ago: the first fill recycles.
            atlas_recycled_at: 0_u64.wrapping_sub(ATLAS_THRASH_FRAMES),
            blur_algorithm: BlurAlgorithm::Gaussian,
            kawase: None,
            blur_cache: Default::default(),
            gaussian_scratch: None,
        })
    }
    /// Bytes the native device reports as allocated (Metal only), including
    /// driver-owned resources that the cache counters cannot see.
    pub fn debug_device_allocated_bytes(&self) -> Option<u64> {
        #[cfg(target_os = "macos")]
        {
            use objc2_metal::MTLDevice;
            // SAFETY: The HAL guard only borrows this live wgpu device.
            let metal = unsafe { self.device.as_hal::<wgpu::hal::api::Metal>() }?;
            Some(metal.raw_device().currentAllocatedSize() as u64)
        }
        #[cfg(not(target_os = "macos"))]
        None
    }
    pub fn debug_cache_stats(&self) -> DebugCacheStats {
        DebugCacheStats {
            shaped_nodes: self.shapes.len(),
            shaped_bytes: self.shapes.values().map(Shaped::bytes).sum(),
            cached_quads: self.shapes.values().map(|s| s.quads.len()).sum(),
            atlas_entries: self.entries.len(),
            atlas_bytes: (self.atlas_size * self.atlas_size * 4) as usize,
            swash_images: self.swash.image_cache.len(),
            swash_outlines: self.swash.outline_command_cache.len(),
            image_textures: self.images.len(),
            image_bytes: self.images.values().map(|i| i.1).sum(),
            shader_resource_bytes: self.procedural.bytes(),
            effect_chain_bytes: self.chain_cache.bytes(),
            blur_cache_bytes: self.blur_cache.values().map(|c| c.bytes()).sum(),
            blur_scratch_bytes: self.kawase.as_ref().map_or(0, |k| k.bytes())
                + self.gaussian_scratch.as_ref().map_or(0, |texture| {
                    texture.width() as usize * texture.height() as usize * 4
                }),
            canvas_raster_bytes: self.canvases.bytes(),
            svg_raster_bytes: self.svgs.bytes(),
            vertex_buffer_bytes: self.vertex_capacity,
            paint_geometry_bytes: self.geometry.bytes(),
            layer_textures: self.layers.len(),
            layer_bytes: self.layers.values().map(|l| l.bytes).sum(),
            scroll_cache_bytes: self.scroll_history.as_ref().map_or(0, |h| {
                h.retained.texture.width() as usize * h.retained.texture.height() as usize * 4
            }),
        }
    }
    pub fn set_background(&mut self, color: Color) {
        if self.background != color {
            self.background = color;
            self.fresh = true;
        }
    }
    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.info.clone()
    }
    pub fn text_system(&self) -> Rc<RefCell<FontSystem>> {
        self.fonts.clone()
    }
    /// Prepared rich text shared with layout measurement.
    pub fn text_cache(&self) -> Rc<RefCell<prepared::TextCache>> {
        self.text_cache.clone()
    }
    /// Install this renderer's text services on `scene`: native shaping,
    /// and measurement from the prepared text it draws from, so layout and
    /// drawing break lines identically. Call `Ui::refresh_text_geometry`
    /// afterwards for a scene with mounted text.
    pub fn install_text(&self, scene: &mut Scene) {
        let fonts = self.text_system();
        scene.set_text_measurer(move |text: &str, size: f32, max: Option<f32>| {
            crate::measure_text(&mut fonts.borrow_mut(), text, size, max)
        });
        let fonts = self.text_system();
        scene.set_font_text_shaper(
            move |text: &str,
                  size: f32,
                  width: Option<f32>,
                  font: &lumine::text_layout::FontStyle|
                  -> Box<dyn lumine::text_layout::TextLayout> {
                Box::new(crate::text::ShapedText::with_font(
                    &mut fonts.borrow_mut(),
                    text,
                    size,
                    width,
                    font,
                ))
            },
        );
        let fonts = self.text_system();
        scene.set_rich_text_shaper(move |rich, width| {
            Box::new(crate::text::ShapedText::with_runs(
                &mut fonts.borrow_mut(),
                rich,
                width,
            ))
        });
        // Prepared text: shaped once, then measured at any width by line
        // breaking alone; the renderer draws from the same lines.
        let (fonts, cache) = (self.text_system(), self.text_cache());
        scene.set_rich_text_measurer(move |rich, width| {
            cache
                .borrow_mut()
                .measure(&mut fonts.borrow_mut(), rich, width)
        });
        // Plain text nodes, from the same prepared text.
        let (fonts, cache) = (self.text_system(), self.text_cache());
        scene.set_font_text_measurer(move |text, size, width, font| {
            cache
                .borrow_mut()
                .measure_plain(&mut fonts.borrow_mut(), text, size, font, width)
        });
        // Apps measuring what they have not mounted keep no glyphs for it.
        let (fonts, cache) = (self.text_system(), self.text_cache());
        scene.set_detached_rich_text_measurer(move |rich, width| {
            cache
                .borrow_mut()
                .measure_detached(&mut fonts.borrow_mut(), rich, width)
        });
    }
    /// Bound icon subpixel rasters while display-paced motion is active. Clear
    /// before the settling frame to recover exact resting device-grid pixels.
    pub fn set_svg_motion(&mut self, active: bool) {
        self.svgs.set_animating(active);
    }
    pub fn set_scale_factor(&mut self, scale: f32) {
        let scale = if scale.is_finite() {
            scale.max(0.1)
        } else {
            1.
        };
        if self.scale != scale {
            self.flush_pending();
            self.scale = scale;
            self.layers.clear();
            self.shapes.clear();
            self.entries.clear();
            self.image_cells.clear();
            self.atlas_epoch = self.atlas_epoch.wrapping_add(1);
            self.cursor = (0, 0, 0);
            self.fresh = true;
        }
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.width, self.height) {
            return;
        }
        self.flush_pending();
        self.layers.clear();
        // Scratch grows on demand; release it when the window shrinks a lot.
        if let Some(kawase) = &mut self.kawase {
            kawase.trim_to(width, height);
        }
        if self.gaussian_scratch.as_ref().is_some_and(|texture| {
            texture.width() > width.next_multiple_of(64)
                || texture.height() > height.next_multiple_of(64)
        }) {
            self.gaussian_scratch = None;
        }
        self.width = width;
        self.height = height;
        self.image_budget = Self::image_budget_for(width, height);
        self.spare = None;
        self.target_copy_bind = None;
        self.target = texture(&self.device, width, height, "retained output");
        self.view = self.target.create_view(&Default::default());
        if let (Some(s), Some(c)) = (&self.surface, &mut self.config) {
            c.width = width;
            c.height = height;
            s.configure(&self.device, c);
        }
        // AppKit shows the resized layer when its transaction commits. A
        // drawable presented outside it lands a frame late, so the window
        // briefly shows stretched or empty content.
        #[cfg(target_os = "macos")]
        {
            self.set_transaction_present(true);
            self.widen_drawables();
        }
        self.blit_bind = self
            .blit
            .as_ref()
            .map(|p| blit_group(&self.device, p, &self.view));
        self.fresh = true;
    }
    pub fn render(&mut self, scene: &Scene, damage: &[Rect]) -> Result<GpuStats, GpuError> {
        self.flush_pending_inner(true);
        if let Some(profiler) = self.profiler.get_mut() {
            profiler.begin(&self.device);
        }
        let presented = std::mem::take(&mut self.present_stats);
        let mut stats = self.render_inner(scene, damage)?;
        stats.shader_dispatches = std::mem::take(&mut self.procedural.dispatches)
            + std::mem::take(&mut self.chain_cache.dispatches);
        stats.shader_resource_allocations = std::mem::take(&mut self.procedural.allocations)
            + std::mem::take(&mut self.chain_cache.allocations);
        stats.effect_stage_cache_hits = std::mem::take(&mut self.chain_cache.cache_hits);
        if stats.shaped_nodes > 0 {
            // Words shaped by measurement or this frame are reused by the next
            // shaping frame (streaming text repeats nearly all of them); older
            // ones are dropped so the cache stays the size of visible text.
            // Trimming keeps the table at its peak size; after much shaping
            // (a transcript's history, measured or drawn), start a new one.
            let shaped = self.text_cache.borrow().shaped;
            self.shaped_since_reset +=
                shaped.saturating_sub(self.cache_shaped) + stats.shaped_nodes as u64;
            self.cache_shaped = shaped;
            let mut fonts = self.fonts.borrow_mut();
            if self.shaped_since_reset > SHAPE_RESET {
                fonts.shape_run_cache = Default::default();
                self.shaped_since_reset = 0;
            } else {
                fonts.shape_run_cache.trim(SHAPE_RUN_AGES);
            }
            drop(fonts);
            // Prepared text likewise: what is drawn, and a little of the rest.
            let mut cache = self.text_cache.borrow_mut();
            if cache.needs_trim() {
                cache.trim(self.shapes.values().filter_map(|shaped| shaped.prepared));
            }
        }
        stats.draw_calls += presented.draw_calls;
        stats.instances += presented.instances;
        Ok(stats)
    }
    fn render_inner(&mut self, scene: &Scene, damage: &[Rect]) -> Result<GpuStats, GpuError> {
        loop {
            match self.render_once(scene, damage) {
                Err(error)
                    if error.0 == "glyph atlas capacity exceeded in one frame"
                        && self.atlas_size < MAX_ATLAS =>
                {
                    self.reset_atlas((self.atlas_size * 2).min(MAX_ATLAS))
                }
                result => return result,
            }
        }
    }
    fn render_once(&mut self, scene: &Scene, damage: &[Rect]) -> Result<GpuStats, GpuError> {
        // A frame deferred but never presented draws into the retained target
        // before this one prunes images or layers it may reference.
        self.flush_pending();
        self.frame_image_ids.clear();
        if self.scene_identity != Some(scene.identity()) {
            self.scene_identity = Some(scene.identity());
            self.layers.clear();
            self.shapes.clear();
            self.node_images.clear();
            self.blur_cache.clear();
            self.shaped_bytes = 0;
            self.fresh = true;
            self.pruned_revision = None;
        }
        // Caches only go stale when nodes are added, removed or change kind.
        if self.pruned_revision != Some(scene.content_revision()) {
            self.pruned_revision = Some(scene.content_revision());
            self.prune(scene);
        }
        self.layers
            .retain(|id, _| scene.contains(*id) && scene.is_isolated(*id));
        // Scroll moves this frame can copy; the rest repaint their clip.
        let next_scroll = self.scroll_state_for(scene, damage);
        let (mut copies, mut repaint) = self.scroll_copies(scene, damage, next_scroll.as_ref());
        self.retain_scroll_frame = false;
        self.scroll_source = None;
        let mut phase_hit = false;
        if let Some(next) = &next_scroll
            && let Some(previous) = &self.scroll_state
            && previous.node == next.node
            && previous.clip == next.clip
            && previous.revision == next.revision
            && !self.fresh
            && self.scroll_phase_cache
            && !cfg!(target_os = "macos")
            && self.width as u64 * self.height as u64 * 4 <= 64 * 1024 * 1024
        {
            let delta = (
                (next.origin.0 - previous.origin.0) * self.scale,
                (next.origin.1 - previous.origin.1) * self.scale,
            );
            let whole = |v: f32| (v - v.round()).abs() < 1e-3;
            // Retain only the common alternating half-pixel case. Continuous
            // subpixel origins fall back instead of accumulating raster phases.
            if whole(delta.0 * 2.) && whole(delta.1 * 2.) && (!whole(delta.0) || !whole(delta.1)) {
                self.retain_scroll_frame = true;
                if let Some(history) = &self.scroll_history
                    && history.state.node == next.node
                    && history.state.clip == next.clip
                    && history.state.revision == next.revision
                {
                    let shift = (
                        next.origin.0 - history.state.origin.0,
                        next.origin.1 - history.state.origin.1,
                    );
                    if let Some(copy) = scroll_pixels(next.clip, shift.0, shift.1, self.scale) {
                        let mut repair = next.foreground.clone();
                        for rect in &next.foreground {
                            if let Some(ghost) = Rect::new(
                                rect.x + shift.0,
                                rect.y + shift.1,
                                rect.width,
                                rect.height,
                            )
                            .intersection(next.clip)
                            {
                                repair.push(ghost);
                            }
                        }
                        for rect in &history.state.foreground {
                            if let Some(ghost) = Rect::new(
                                rect.x + shift.0,
                                rect.y + shift.1,
                                rect.width,
                                rect.height,
                            )
                            .intersection(next.clip)
                            {
                                repair.push(ghost);
                            }
                        }
                        let (x, y, w, h) = copy.0;
                        let dest = Rect::new(
                            x as f32 / self.scale,
                            y as f32 / self.scale,
                            w as f32 / self.scale,
                            h as f32 / self.scale,
                        );
                        repair.extend(rect_outside(next.clip, dest));
                        copies = vec![copy];
                        repaint = repair;
                        self.scroll_source = Some(history.retained.texture.clone());
                        phase_hit = true;
                    }
                }
            }
        }
        self.copies = copies;
        let extended;
        let damage = if repaint.is_empty() {
            damage
        } else {
            extended = [damage, &repaint].concat();
            &extended[..]
        };
        if damage.is_empty() && !self.fresh && self.copies.is_empty() {
            return Ok(GpuStats::default());
        }
        // Replay only damaged subtrees and the bounded sources sampled by blur.
        let viewport = Rect::new(
            0.,
            0.,
            self.width as f32 / self.scale,
            self.height as f32 / self.scale,
        );
        let items = if self.fresh {
            scene.layer_items(None)
        } else {
            let dependencies: Vec<_> = scene.backdrop_regions().collect();
            let regions = blur_damage(damage, &dependencies, viewport, self.scale);
            scene.layer_items_within(None, Some(&regions))
        };
        let mut layer_stats = GpuStats::default();
        for item in &items {
            if item.isolated && item.effects.opacity > 0. {
                self.prepare_layer(scene, item.id, &mut layer_stats)?;
            }
        }
        let mut stats = self.render_flat(scene, items, damage)?;
        if self.retain_scroll_frame {
            let retained = self.spare.take().expect("scroll frame preserved in spare");
            let state = self.scroll_state.take().expect("previous frame exists");
            let old_history = self
                .scroll_history
                .replace(ScrollHistory { retained, state });
            self.spare = old_history.map(|h| h.retained);
        } else {
            self.scroll_history = None;
        }
        self.scroll_state = next_scroll;
        stats.scroll_phase_hits = usize::from(phase_hit);
        self.rendered_serial = Some(scene.flush_serial());
        stats.layer_repaints = layer_stats.layer_repaints;
        stats.layer_cache_hits = layer_stats.layer_cache_hits;
        stats.layer_texture_allocations = layer_stats.layer_texture_allocations;
        stats.damaged_pixels += layer_stats.damaged_pixels;
        stats.draw_calls += layer_stats.draw_calls;
        stats.render_passes += layer_stats.render_passes;
        stats.blur_passes += layer_stats.blur_passes;
        stats.blur_cache_hits += layer_stats.blur_cache_hits;
        stats.blur_texture_allocations += layer_stats.blur_texture_allocations;
        stats.filtered_pixels += layer_stats.filtered_pixels;
        stats.instances += layer_stats.instances;
        stats.shaped_nodes += layer_stats.shaped_nodes;
        stats.glyph_uploads += layer_stats.glyph_uploads;
        stats.image_uploads += layer_stats.image_uploads;
        stats.native_surface_imports += layer_stats.native_surface_imports;
        stats.native_surface_conversions += layer_stats.native_surface_conversions;
        stats.canvas_rasterizations += layer_stats.canvas_rasterizations;
        stats.svg_rasterizations += layer_stats.svg_rasterizations;
        stats.geometry_rebuilds += layer_stats.geometry_rebuilds;
        stats.vertex_buffer_allocations += layer_stats.vertex_buffer_allocations;
        stats.paint_geometry_buffer_allocations += layer_stats.paint_geometry_buffer_allocations;
        Ok(stats)
    }
    /// Drop cached shapes, canvases, SVGs and textures of nodes that are gone.
    fn prune(&mut self, scene: &Scene) {
        self.blur_cache
            .retain(|id, _| scene.contains(*id) && scene.effects(*id).blur_radius > 0.);
        let mut snapshots = std::collections::HashSet::new();
        let mut instances = std::collections::HashSet::new();
        let mut chains = std::collections::HashSet::new();
        let mut nodes = vec![scene.root()];
        while let Some(node) = nodes.pop() {
            nodes.extend_from_slice(scene.children(node));
            if let NodeKind::Image(image) = scene.kind(node) {
                let mut image = image.as_ref();
                loop {
                    snapshots.insert(image.id());
                    if let Some(id) = image.procedural().and_then(|p| p.instance) {
                        instances.insert(id);
                    }
                    let Some(chain) = image.effect_chain() else {
                        break;
                    };
                    chains.insert(chain.instance);
                    image = chain.input.as_ref();
                }
            }
            #[cfg(target_os = "macos")]
            if let NodeKind::NativeSurface(frame) = scene.kind(node) {
                snapshots.insert(frame.id());
            }
        }
        self.procedural.retain(snapshots.clone(), &instances);
        self.chain_cache.retain(snapshots.clone(), &chains);
        self.node_images.retain(|id, _| scene.contains(*id));
        self.shapes.retain(|id, _| {
            scene.contains(*id)
                && matches!(
                    scene.kind(*id),
                    NodeKind::Text { .. } | NodeKind::RichText { .. }
                )
        });
        self.shaped_bytes = self.shapes.values().map(Shaped::bytes).sum();
        self.canvases.retain(scene);
        self.svgs.retain(scene);
        if !self.images.is_empty() {
            let mut live_images = snapshots;
            live_images.extend(self.canvases.image_ids());
            live_images.extend(self.svgs.image_ids());
            #[cfg(target_os = "macos")]
            if let Some(cache) = &mut self.native_surfaces {
                cache.retain(&live_images);
            }
            self.images.retain(|id, _| live_images.contains(id));
            self.image_textures.retain(|id, _| live_images.contains(id));
        }
    }
    fn prepare_layer(
        &mut self,
        scene: &Scene,
        id: NodeId,
        stats: &mut GpuStats,
    ) -> Result<(), GpuError> {
        let revision = scene.layer_revision(id);
        let root = scene.bounds(id);
        if let Some(cache) = self.layers.get_mut(&id)
            && cache.revision == revision
            && [root.x - cache.root_origin.0, root.y - cache.root_origin.1]
                .into_iter()
                .all(|delta| {
                    let pixels = delta * self.scale;
                    (pixels - pixels.round()).abs() < 1e-3
                })
        {
            cache.bounds.x += root.x - cache.root_origin.0;
            cache.bounds.y += root.y - cache.root_origin.1;
            cache.root_origin = (root.x, root.y);
            stats.layer_cache_hits += 1;
            return Ok(());
        }
        // Align the layer to device pixels so compositing does not resample
        // already-antialiased text/icons. Fractional movement repaints above;
        // whole-pixel movement can still reuse the retained texture.
        let bounds = scene.layer_bounds(id).expand(1. / self.scale);
        let x = (bounds.x * self.scale).floor() / self.scale;
        let y = (bounds.y * self.scale).floor() / self.scale;
        let width = (((bounds.x + bounds.width) * self.scale).ceil()
            - (bounds.x * self.scale).floor())
        .max(1.) as u32;
        let height = (((bounds.y + bounds.height) * self.scale).ceil()
            - (bounds.y * self.scale).floor())
        .max(1.) as u32;
        let bounds = Rect::new(x, y, width as f32 / self.scale, height as f32 / self.scale);
        let items = scene.layer_items(Some(id));
        for item in &items {
            if item.isolated && item.effects.opacity > 0. {
                self.prepare_layer(scene, item.id, stats)?;
            }
        }
        let existing = self.layers.get(&id).map_or(0, |l| l.bytes);
        if !self.layers.contains_key(&id) && self.layers.len() >= 1024 {
            return Err(GpuError("isolated layers exceed 1024 texture limit".into()));
        }
        if width > self.device.limits().max_texture_dimension_2d
            || height > self.device.limits().max_texture_dimension_2d
        {
            return Err(GpuError(
                "isolated layer exceeds GPU texture dimensions".into(),
            ));
        }
        // Animated layers change size every frame: round allocations up and
        // reuse any texture that fits without wasting more than half of it.
        let limit = self.device.limits().max_texture_dimension_2d;
        let (alloc_w, alloc_h) = (
            width.next_multiple_of(64).min(limit),
            height.next_multiple_of(64).min(limit),
        );
        let reuse = self.layers.get(&id).is_some_and(|cache| {
            let (w, h) = (cache.texture.width(), cache.texture.height());
            w >= width
                && h >= height
                && u64::from(w) * u64::from(h) <= 2 * u64::from(alloc_w) * u64::from(alloc_h)
        });
        // Budget the allocation we will actually retain, including rounding or
        // a larger reusable texture. A steady/shrinking allocation cannot grow
        // the cache and needs no full map scan.
        let bytes = if reuse {
            existing
        } else {
            alloc_w as usize * alloc_h as usize * 4
        };
        if bytes > 64 * 1024 * 1024
            || (bytes > existing
                && self.layers.values().map(|l| l.bytes).sum::<usize>() - existing + bytes
                    > 64 * 1024 * 1024)
        {
            return Err(GpuError("isolated layers exceed 64 MiB budget".into()));
        }
        let existing = self.layers.remove(&id);
        let (texture, bind) = if let Some(cache) = existing.filter(|_| reuse) {
            (cache.texture, cache.bind)
        } else {
            let texture = texture(&self.device, alloc_w, alloc_h, "isolated subtree");
            let bind = self.texture_bind(
                &texture.create_view(&Default::default()),
                "isolated subtree",
            );
            stats.layer_texture_allocations += 1;
            (texture, bind)
        };
        let bytes = texture.width() as usize * texture.height() as usize * 4;
        // The content occupies the top-left corner; the quad samples just that.
        let uv = [
            width as f32 / texture.width() as f32,
            height as f32 / texture.height() as f32,
        ];
        let view = texture.create_view(&Default::default());
        let target = std::mem::replace(&mut self.target, texture);
        let old_view = std::mem::replace(&mut self.view, view);
        let old_size = (self.width, self.height);
        self.width = self.target.width();
        self.height = self.target.height();
        let background = self.background;
        self.background = Color(0, 0, 0, 0);
        let fresh = self.fresh;
        self.fresh = true;
        let local: Vec<_> = items
            .into_iter()
            .map(|mut item| {
                let affine = item.transform != lumine::affine::Affine::IDENTITY
                    || !item.clip_regions.is_empty()
                    || item.fade_transform != lumine::affine::Affine::IDENTITY;
                if affine {
                    let shift = lumine::affine::Affine::translation(-bounds.x, -bounds.y);
                    let unshift = lumine::affine::Affine::translation(bounds.x, bounds.y);
                    item.transform = item.transform.then(shift);
                    item.fade_transform = unshift.then(item.fade_transform);
                    item.clip_regions = item
                        .clip_regions
                        .iter()
                        .map(|clip| {
                            let mut clip = *clip;
                            clip.inverse = unshift.then(clip.inverse);
                            clip
                        })
                        .collect();
                } else {
                    item.bounds.x -= bounds.x;
                    item.bounds.y -= bounds.y;
                    item.mask = item.mask.map(|(r, bands)| {
                        (
                            Rect::new(r.x - bounds.x, r.y - bounds.y, r.width, r.height),
                            bands,
                        )
                    });
                }
                item.clip = item
                    .clip
                    .map(|r| Rect::new(r.x - bounds.x, r.y - bounds.y, r.width, r.height));
                item
            })
            .collect();
        // Nested layer images remain world-positioned in their cache; pass the offset
        // separately so the same texture can be composed into any ancestor layer.
        let old_origin = self.layer_origin;
        self.layer_origin = (bounds.x, bounds.y);
        let in_layer = std::mem::replace(&mut self.in_layer, true);
        // `fresh` repaints the whole texture, slack included, so an earlier
        // larger frame never bleeds into sampling at the content's edge.
        let result = self.render_flat(
            scene,
            local,
            &[Rect::new(0., 0., bounds.width, bounds.height)],
        );
        self.in_layer = in_layer;
        self.layer_origin = old_origin;
        let texture = std::mem::replace(&mut self.target, target);
        let _view = std::mem::replace(&mut self.view, old_view);
        self.width = old_size.0;
        self.height = old_size.1;
        self.background = background;
        self.fresh = fresh;
        let result = result?;
        stats.draw_calls += result.draw_calls;
        stats.render_passes += result.render_passes;
        stats.blur_passes += result.blur_passes;
        stats.blur_cache_hits += result.blur_cache_hits;
        stats.blur_texture_allocations += result.blur_texture_allocations;
        stats.filtered_pixels += result.filtered_pixels;
        stats.instances += result.instances;
        stats.shaped_nodes += result.shaped_nodes;
        stats.glyph_uploads += result.glyph_uploads;
        stats.image_uploads += result.image_uploads;
        stats.canvas_rasterizations += result.canvas_rasterizations;
        stats.svg_rasterizations += result.svg_rasterizations;
        stats.geometry_rebuilds += result.geometry_rebuilds;
        stats.vertex_buffer_allocations += result.vertex_buffer_allocations;
        stats.paint_geometry_buffer_allocations += result.paint_geometry_buffer_allocations;
        stats.layer_repaints += 1;
        stats.damaged_pixels += result.damaged_pixels;
        self.layers.insert(
            id,
            LayerCache {
                texture,
                bind,
                bounds,
                revision,
                root_origin: (root.x, root.y),
                bytes,
                uv,
            },
        );
        Ok(())
    }
    fn select_viewport(&mut self, width: f32, height: f32) {
        let key = (width.to_bits(), height.to_bits());
        if !self.viewports.contains_key(&key) {
            // Layer sizes come and go; keep the map small.
            if self.viewports.len() >= 32 {
                self.viewports.clear();
            }
            let buffer = upload::init_buffer(
                &self.device,
                self.mapped.is_some(),
                "viewport",
                bytemuck::cast_slice(&[width, height, 0., 0.]),
                wgpu::BufferUsages::UNIFORM,
            );
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("viewport"),
                layout: &self.context.inner.viewport_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            self.viewports.insert(key, bind);
        }
        self.viewport_bind = self.viewports.get(&key).cloned();
    }
    fn texture_bind(&self, view: &wgpu::TextureView, label: &str) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.context.inner.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.device.create_sampler(
                        &wgpu::SamplerDescriptor {
                            mag_filter: wgpu::FilterMode::Linear,
                            min_filter: wgpu::FilterMode::Linear,
                            ..Default::default()
                        },
                    )),
                },
            ],
        })
    }
    fn render_flat(
        &mut self,
        scene: &Scene,
        items: Vec<lumine::scene::PaintItem<'_>>,
        damage: &[Rect],
    ) -> Result<GpuStats, GpuError> {
        let mut stats = GpuStats::default();
        let copies = if self.in_layer || self.fresh {
            Vec::new()
        } else {
            std::mem::take(&mut self.copies)
        };
        if damage.is_empty() && !self.fresh && copies.is_empty() {
            return Ok(stats);
        }
        // Earlier commands read the vertex and uniform buffers rewritten below.
        self.flush_pending();
        let viewport = Rect::new(
            0.,
            0.,
            self.width as f32 / self.scale,
            self.height as f32 / self.scale,
        );
        self.select_viewport(viewport.width, viewport.height);
        let has_blur = items
            .iter()
            .any(|item| item.effects.opacity > 0. && item.effects.blur_radius > 0.);
        // Reconstruct every sampled backdrop pixel before replaying filters. A
        // dependency can reach another filter, so expand to a fixed point.
        let damage = if self.fresh {
            vec![viewport]
        } else if has_blur {
            let dependencies: Vec<_> = items
                .iter()
                .filter_map(|item| {
                    if item.effects.opacity <= 0. || item.effects.blur_radius <= 0. {
                        return None;
                    }
                    let output = item
                        .transform
                        .bounds(item.bounds)
                        .intersection(item.clip.unwrap_or(viewport))?
                        .intersection(viewport)?;
                    Some((output, item.effects.blur_radius))
                })
                .collect();
            blur_damage(damage, &dependencies, viewport, self.scale)
        } else {
            merge_damage(damage, viewport, self.scale)
        };
        if damage.is_empty() && copies.is_empty() {
            return Ok(stats);
        }
        self.frame = self.frame.wrapping_add(1);
        if self.cursor.1 + self.cursor.2 > self.atlas_size * 3 / 4 || self.entries.len() >= 8192 {
            // Filling again soon after the last recycle means the visible
            // working set does not fit: grow rather than re-rasterize every
            // glyph and icon each time it fills.
            let thrashing = self.frame.wrapping_sub(self.atlas_recycled_at) < ATLAS_THRASH_FRAMES;
            if thrashing && self.atlas_size < MAX_ATLAS && self.entries.len() < 8192 {
                self.reset_atlas((self.atlas_size * 2).min(MAX_ATLAS));
            } else {
                self.entries.clear();
                self.image_cells.clear();
                self.atlas_epoch = self.atlas_epoch.wrapping_add(1);
                self.cursor = (0, 0, 0);
            }
            self.atlas_recycled_at = self.frame;
        }
        let mut quads = std::mem::take(&mut self.frame_quads);
        quads.clear();
        quads.push(Quad {
            rect: [0., 0., viewport.width, viewport.height],
            uv: [0.; 4],
            color: rgba(self.background, 1.),
            fade: [0.; 4],
            options: [0.; 4],
            shape: [0.; 4],
            border: [0.; 4],
            mask: NO_MASK,
        });
        let diagnose = !self.in_layer && self.profiler.get_mut().as_mut()
            .is_some_and(|p| p.begin_draw_diagnostics());
        let mut draws = Vec::new();
        self.geometry.begin();
        let element_profiler = scene.profiler();
        for mut item in items {
            let _span = element_profiler.span("paint/prepare", Some(item.id));
            // Retained hidden subtrees must not allocate textures, shape text,
            // or trigger filters until their effective opacity is visible.
            if item.effects.opacity <= 0. {
                continue;
            }
            // Bake direct primitive placement into its allocation so siblings
            // can share the container's affine binding and remain one batch.
            // Filters and world-rasterized isolated layers need the original map.
            if !item.isolated
                && item.effects.blur_radius <= 0.
                && item.transform != lumine::affine::Affine::IDENTITY
            {
                let linear = lumine::affine::Affine {
                    tx: 0.,
                    ty: 0.,
                    ..item.transform
                };
                if let Some(inverse) = linear.inverse() {
                    let (x, y) = inverse.point(item.transform.tx, item.transform.ty);
                    let shifted = Rect::new(
                        item.bounds.x + x,
                        item.bounds.y + y,
                        item.bounds.width,
                        item.bounds.height,
                    );
                    // Ill-conditioned maps can amplify rounding into visible
                    // movement. Keep their original representation instead.
                    let equivalent = [
                        (0., 0.),
                        (item.bounds.width, 0.),
                        (0., item.bounds.height),
                        (item.bounds.width, item.bounds.height),
                    ]
                    .into_iter()
                    .all(|(dx, dy)| {
                        let a = item.transform.point(item.bounds.x + dx, item.bounds.y + dy);
                        let b = linear.point(shifted.x + dx, shifted.y + dy);
                        b.0.is_finite()
                            && b.1.is_finite()
                            && (a.0 - b.0).abs() * self.scale < 0.01
                            && (a.1 - b.1).abs() * self.scale < 0.01
                    });
                    if equivalent {
                        item.bounds = shifted;
                        item.transform = linear;
                    }
                }
            }
            let world_bounds = item.transform.bounds(item.bounds);
            if item.transform.inverse().is_none() {
                continue;
            }
            let (geometry_offset, geometry_hash) = self.geometry.item(&item, self.scale)?;
            let node_clip = if geometry_offset == 0
                && !item.isolated
                && matches!(item.kind, NodeKind::Text { .. } | NodeKind::RichText { .. })
            {
                item.clip
                    .unwrap_or(viewport)
                    .intersection(item.bounds)
                    .unwrap_or_default()
            } else {
                item.clip.unwrap_or(viewport)
            };
            let Some(clip) = node_clip.intersection(viewport) else {
                continue;
            };

            if !damage.iter().any(|d| {
                d.intersects(clip)
                    && d.intersects(if item.isolated {
                        let b = self.layers[&item.id].bounds;
                        Rect::new(
                            b.x - self.layer_origin.0,
                            b.y - self.layer_origin.1,
                            b.width,
                            b.height,
                        )
                    } else {
                        item.transform.bounds(
                            if let NodeKind::Quad(style) | NodeKind::Panel { quad: style, .. } =
                                item.kind
                            {
                                style.paint_bounds(item.bounds)
                            } else if let NodeKind::Svg(svg) = item.kind {
                                svg.paint_bounds(item.bounds)
                            } else if let NodeKind::Image(image) = item.kind {
                                image.paint_bounds(item.bounds)
                            } else {
                                item.bounds
                            },
                        )
                    })
            }) {
                continue;
            }
            let start = quads.len() as u32;
            if item.isolated {
                let layer = self.layers.get(&item.id).expect("prepared layer");
                let bounds = Rect::new(
                    layer.bounds.x - self.layer_origin.0,
                    layer.bounds.y - self.layer_origin.1,
                    layer.bounds.width,
                    layer.bounds.height,
                );
                quads.push(Quad {
                    rect: [bounds.x, bounds.y, bounds.width, bounds.height],
                    uv: [0., 0., layer.uv[0], layer.uv[1]],
                    color: [1., 1., 1., item.effects.opacity],
                    fade: [
                        item.bounds.x,
                        item.bounds.y,
                        item.bounds.width,
                        item.bounds.height,
                    ],
                    options: [item.effects.edge_fade, 2., 0., 0.],
                    shape: [0.; 4],
                    border: [0.; 4],
                    mask: item.mask.map_or(NO_MASK, |(rect, [top, bottom])| {
                        fade_parameters(rect, [top, bottom])
                    }),
                });
                draws.push(Draw {
                    diagnostic_source: diagnose.then(|| draw_diagnostics::Source::new(
                        &item, self.geometry.clip_count(geometry_offset))),
                    transform: lumine::affine::Affine::IDENTITY,
                    geometry_offset,
                    geometry_hash,
                    geometry: None,
                    shading: Shading::Full,
                    blur_mask: item.mask,
                    start,
                    end: quads.len() as u32,
                    clip,
                    bounds: quad_bounds(&quads[start as usize..]),
                    image: None,
                    layer: Some(item.id),
                    blur: (item.effects.blur_radius > 0.).then_some((
                        item.id,
                        world_bounds,
                        item.effects,
                    )),
                });
                continue;
            }
            let bounds = item.bounds;
            let fade = [bounds.x, bounds.y, bounds.width, bounds.height];
            let mut canvas_image = None;
            match item.kind {
                NodeKind::Canvas(canvas) => {
                    if bounds.width <= 0. || bounds.height <= 0. {
                        continue;
                    }
                    let before = self.canvases.rasterizations();
                    let (width, height, scale) = (bounds.width, bounds.height, self.scale);
                    let id = self.canvas_texture(item.id, &mut stats, |cache| {
                        cache.get(item.id, canvas, width, height, scale)
                    })?;
                    stats.canvas_rasterizations +=
                        (self.canvases.rasterizations() - before) as usize;
                    canvas_image = Some(id);
                    quads.push(Quad {
                        rect: fade,
                        uv: [0., 0., 1., 1.],
                        color: [1., 1., 1., item.effects.opacity],
                        fade,
                        options: [item.effects.edge_fade, 2., 0., 0.],
                        shape: [0.; 4],
                        border: [0.; 4],
                        mask: NO_MASK,
                    });
                }
                NodeKind::Rect(color) => quads.push(Quad {
                    rect: fade,
                    uv: [0.; 4],
                    color: rgba(*color, item.effects.opacity),
                    fade,
                    options: [item.effects.edge_fade, 0., 0., 0.],
                    shape: [0.; 4],
                    border: [0.; 4],
                    mask: NO_MASK,
                }),
                NodeKind::Quad(style) | NodeKind::Panel { quad: style, .. } => {
                    if style.shadows().len() > 32 {
                        return Err(GpuError("panel exceeds 32 shadows".into()));
                    }
                    for shadow in style.shadows() {
                        let core = bounds.expand(shadow.spread);
                        if core.width <= 0. || core.height <= 0. {
                            continue;
                        }
                        let core = Rect::new(
                            core.x + shadow.offset.x,
                            core.y + shadow.offset.y,
                            core.width,
                            core.height,
                        );
                        let outer = core.expand(shadow.blur_radius.max(0.) * 3.);
                        let corners =
                            decoration::shadow_corners(style, bounds.width, bounds.height, 0.);
                        // Outer shadows reuse untextured UVs for the original box,
                        // shape.x/y/w and options.x for its four knockout radii.
                        quads.push(Quad {
                            rect: [outer.x, outer.y, outer.width, outer.height],
                            uv: fade,
                            color: rgba(shadow.color, item.effects.opacity),
                            fade: [core.x, core.y, core.width, core.height],
                            options: [corners[3], 0., 0., 2.],
                            shape: [
                                corners[0],
                                corners[1],
                                shadow.blur_radius.max(0.001),
                                corners[2],
                            ],
                            border: decoration::shadow_corners(
                                style,
                                core.width,
                                core.height,
                                shadow.spread,
                            ),
                            mask: NO_MASK,
                        });
                    }
                    // Per-corner radii over a flat fill need no raster: the quad
                    // shader draws per-corner rounded rects. Borders, gradients
                    // and patterns still rasterize.
                    let flat_fill = style.decoration.as_deref().and_then(|detail| {
                        let fill = match &detail.background {
                            None => style.fill,
                            Some(lumine::decoration::Background::Brush(
                                lumine::canvas::Brush::Solid(color),
                            )) => *color,
                            Some(_) => return None,
                        };
                        let widths = detail
                            .border_widths
                            .unwrap_or(lumine::scene::Insets::all(style.border_width));
                        let border = widths
                            .left
                            .max(widths.right)
                            .max(widths.top)
                            .max(widths.bottom);
                        (border <= 0. || style.border_color.3 == 0).then_some(fill)
                    });
                    if let Some(fill) = flat_fill {
                        self.canvases.forget(item.id);
                        if let Some(old) = self.node_images.remove(&item.id) {
                            self.images.remove(&old);
                            self.image_textures.remove(&old);
                        }
                        // Empty draws below still carry backdrop filters and masks.
                        if fill.3 != 0 {
                            quads.push(Quad {
                                rect: fade,
                                uv: [0.; 4],
                                color: rgba(fill, item.effects.opacity),
                                fade,
                                options: [item.effects.edge_fade, 0., 0., 1.],
                                shape: [0., 0., 0., 1.],
                                border: decoration::shadow_corners(
                                    style,
                                    bounds.width,
                                    bounds.height,
                                    0.,
                                ),
                                mask: NO_MASK,
                            });
                        }
                    } else if style.decoration.is_some() && bounds.width > 0. && bounds.height > 0.
                    {
                        let before = self.canvases.rasterizations();
                        let scale = self.scale;
                        let grid =
                            decoration::stretch_grid(style, bounds.width, bounds.height, scale);
                        let (width, height) = grid.map_or((bounds.width, bounds.height), |c| {
                            (c[0] + c[2] + 1. / scale, c[1] + c[3] + 1. / scale)
                        });
                        let id = self.canvas_texture(item.id, &mut stats, |cache| {
                            cache.get_decoration(item.id, style, width, height, scale)
                        })?;
                        stats.canvas_rasterizations +=
                            (self.canvases.rasterizations() - before) as usize;
                        canvas_image = Some(id);
                        let (xs, ys, us, vs) = if let Some(c) = grid {
                            (
                                [
                                    bounds.x,
                                    bounds.x + c[0],
                                    bounds.x + bounds.width - c[2],
                                    bounds.x + bounds.width,
                                ],
                                [
                                    bounds.y,
                                    bounds.y + c[1],
                                    bounds.y + bounds.height - c[3],
                                    bounds.y + bounds.height,
                                ],
                                [0., c[0] / width, 1. - c[2] / width, 1.],
                                [0., c[1] / height, 1. - c[3] / height, 1.],
                            )
                        } else {
                            (
                                [bounds.x, bounds.x, bounds.x, bounds.x + bounds.width],
                                [bounds.y, bounds.y, bounds.y, bounds.y + bounds.height],
                                [0., 0., 0., 1.],
                                [0., 0., 0., 1.],
                            )
                        };
                        for y in 0..3 {
                            for x in 0..3 {
                                if xs[x + 1] <= xs[x] || ys[y + 1] <= ys[y] {
                                    continue;
                                }
                                quads.push(Quad {
                                    rect: [xs[x], ys[y], xs[x + 1] - xs[x], ys[y + 1] - ys[y]],
                                    uv: [us[x], vs[y], us[x + 1] - us[x], vs[y + 1] - vs[y]],
                                    color: [1., 1., 1., item.effects.opacity],
                                    fade,
                                    options: [item.effects.edge_fade, 2., 0., 0.],
                                    shape: [0.; 4],
                                    border: [0.; 4],
                                    mask: NO_MASK,
                                });
                            }
                        }
                    } else {
                        quads.push(Quad {
                            rect: fade,
                            uv: [0.; 4],
                            color: rgba(style.fill, item.effects.opacity),
                            fade,
                            options: [item.effects.edge_fade, 0., 0., 0.],
                            shape: [style.radius.max(0.), style.border_width.max(0.), 0., 1.],
                            border: rgba(style.border_color, item.effects.opacity),
                            mask: NO_MASK,
                        });
                    }
                }
                NodeKind::Svg(svg) => {
                    if svg.paint_bounds(bounds).width <= 0. || svg.paint_bounds(bounds).height <= 0.
                    {
                        continue;
                    }
                    let before = self.svgs.rasterizations();
                    let (image, raster_bounds) = self.svgs.get_at(item.id, svg, bounds, self.scale)?;
                    stats.svg_rasterizations += (self.svgs.rasterizations() - before) as usize;
                    // Nodes share SVG rasters: `prune` releases their textures
                    // once no node shows them (a new source prunes).
                    let transform = image.paint_transform(raster_bounds);
                    let transformed = image.transform() != lumine::affine::Affine::IDENTITY;
                    // Icon-sized rasters live in the glyph atlas and batch with
                    // text; larger ones keep a texture of their own.
                    let uv =
                        if image.width() <= ATLAS_IMAGE_MAX && image.height() <= ATLAS_IMAGE_MAX {
                            let (x, y) = match self.image_cells.get(&image.id()) {
                                Some(cell) => *cell,
                                None => {
                                    let (width, height) = (image.width(), image.height());
                                    let cell = self.atlas_cell(width, height, |rgba| {
                                        premultiply_into_cell(rgba, image.pixels(), width);
                                    })?;
                                    self.image_cells.insert(image.id(), cell);
                                    stats.image_uploads += 1;
                                    cell
                                }
                            };
                            let atlas = self.atlas_size as f32;
                            [
                                x as f32 / atlas,
                                y as f32 / atlas,
                                image.width() as f32 / atlas,
                                image.height() as f32 / atlas,
                            ]
                        } else {
                            if !self.images.contains_key(&image.id()) {
                                self.upload_image(&image)?;
                                stats.image_uploads += 1;
                            }
                            canvas_image = Some(image.id());
                            [0., 0., 1., 1.]
                        };
                    quads.push(Quad {
                        rect: [
                            raster_bounds.x,
                            raster_bounds.y,
                            raster_bounds.width,
                            raster_bounds.height,
                        ],
                        uv,
                        color: [1., 1., 1., item.effects.opacity],
                        fade,
                        options: [
                            item.effects.edge_fade,
                            2.,
                            if transformed { 1. } else { 0. },
                            0.,
                        ],
                        shape: if transformed {
                            [transform.a, transform.c, transform.b, transform.d]
                        } else {
                            [0.; 4]
                        },
                        border: if transformed {
                            [transform.tx, transform.ty, 0., 0.]
                        } else {
                            [0.; 4]
                        },
                        mask: NO_MASK,
                    });
                }
                #[cfg(target_os = "macos")]
                NodeKind::NativeSurface(frame) => {
                    if !self.images.contains_key(&frame.id()) {
                        let bytes = frame.width() as usize
                            * frame.height() as usize
                            * if frame.format() == lumine::native_surface::SurfaceFormat::Bgra {
                                4
                            } else {
                                6
                            };
                        self.ensure_image_budget(bytes).map_err(|_| {
                            GpuError(format!(
                                "visible image and native surface textures exceed {} MiB",
                                self.image_budget / (1024 * 1024)
                            ))
                        })?;
                        if self.native_surfaces.is_none() {
                            self.native_surfaces = Some(native_surface::SurfaceCache::new(
                                &self.device,
                                &self.queue,
                            )?);
                        }
                        let cache = self.native_surfaces.as_mut().unwrap();
                        cache.import(frame.clone())?;
                        stats.native_surface_imports += 1;
                        stats.native_surface_conversions += usize::from(
                            frame.format() == lumine::native_surface::SurfaceFormat::Nv12FullRange,
                        );
                        let view = cache.texture(frame.id()).create_view(&Default::default());
                        let bytes = cache.bytes(frame.id());
                        let bind = self.texture_bind(&view, "native CoreVideo frame");
                        self.images.insert(frame.id(), (bind, bytes));
                    }
                    canvas_image = Some(frame.id());
                    quads.push(Quad {
                        rect: fade,
                        uv: [0., 0., 1., 1.],
                        color: [1., 1., 1., item.effects.opacity],
                        fade,
                        options: [item.effects.edge_fade, 2., 0., 0.],
                        shape: [0.; 4],
                        border: [0.; 4],
                        mask: NO_MASK,
                    });
                }
                NodeKind::Image(image) => {
                    if image.paint_bounds(bounds).width <= 0.
                        || image.paint_bounds(bounds).height <= 0.
                    {
                        continue;
                    }
                    let mipmapped = image_mips::needed(image, bounds, item.transform, self.scale);
                    // Sampling variants share an ID; upgrade level-zero uploads on demand.
                    if !self.images.contains_key(&image.id())
                        || (mipmapped && self.image_textures[&image.id()].mip_level_count() == 1)
                    {
                        self.upload_image_levels(image, mipmapped)?;
                        stats.image_uploads += 1;
                    }
                    let transform = image.paint_transform(bounds);
                    let transformed = image.transform() != lumine::affine::Affine::IDENTITY;
                    // Images have no procedural border/shape. Reuse those
                    // attribute lanes rather than growing every glyph instance.
                    quads.push(Quad {
                        rect: fade,
                        uv: [0., 0., 1., 1.],
                        color: [1., 1., 1., item.effects.opacity],
                        fade,
                        options: [
                            item.effects.edge_fade,
                            2.,
                            if transformed { 1. } else { 0. },
                            if image.sampling() == lumine::image::ImageSampling::Nearest {
                                1.
                            } else {
                                0.
                            },
                        ],
                        shape: if transformed {
                            [transform.a, transform.c, transform.b, transform.d]
                        } else {
                            [0.; 4]
                        },
                        border: if transformed {
                            [transform.tx, transform.ty, 0., 0.]
                        } else {
                            [0.; 4]
                        },
                        mask: NO_MASK,
                    });
                }
                kind @ (NodeKind::Text { .. } | NodeKind::RichText { .. }) => {
                    let rich = match kind {
                        NodeKind::RichText { text } => Some(text.clone()),
                        _ => None,
                    };
                    let rich_content = rich.as_ref().map(|r| r.text_arc());
                    let (text, color, font_size) = match kind {
                        NodeKind::Text {
                            text,
                            color,
                            font_size,
                        } => (text, *color, *font_size),
                        NodeKind::RichText { .. } => (
                            rich_content.as_ref().unwrap(),
                            lumine::scene::Color(255, 255, 255, 255),
                            16.,
                        ),
                        _ => unreachable!(),
                    };
                    if !font_size.is_finite() || font_size <= 0. {
                        return Err(GpuError("font size must be finite and positive".into()));
                    }
                    let unwrapped = !scene.style(item.id).text_wrap
                        && item.text_options == Default::default()
                        && rich
                            .as_ref()
                            .is_none_or(|r| r.options() == Default::default());
                    // Colour-only rich changes (fades, highlights) keep glyph
                    // positions: recolour the cached shape instead of re-shaping.
                    if let (Some(new), Some(shaped)) = (&rich, self.shapes.get_mut(&item.id))
                        && let Some(old) = &shaped.rich
                        && !Arc::ptr_eq(old, new)
                        && old.same_shape(new)
                        && shaped.width == bounds.width
                        && shaped.height == bounds.height
                        && shaped.unwrapped == unwrapped
                        && shaped.text_options == item.text_options
                        && shaped.font == *item.font
                    {
                        for glyph in &mut shaped.glyphs {
                            glyph.color = Some(new.runs()[glyph.run].color);
                        }
                        for decoration in &mut shaped.decorations {
                            decoration.color = decoration.color_in(new);
                        }
                        shaped.rich = Some(new.clone());
                        shaped.atlas_epoch = self.atlas_epoch.wrapping_sub(1);
                    }
                    let stale = self.shapes.get(&item.id).is_none_or(|s| {
                        s.rich != rich
                            || s.text_options != item.text_options
                            || s.text != *text
                            || s.font != *item.font
                            || s.size != font_size
                            || s.width != bounds.width
                            || s.height != bounds.height
                            || s.unwrapped != unwrapped
                    });
                    if stale {
                        if let Some(old) = self.shapes.remove(&item.id) {
                            self.shaped_bytes -= old.bytes();
                        }
                        let mut fonts = self.fonts.borrow_mut();
                        // Default-option rich text: the prepared lines layout
                        // measured with, laid out at this width, no reshaping.
                        let prepared = rich
                            .as_ref()
                            .filter(|rich| prepared::TextCache::handles(rich));
                        let plain = rich.is_none()
                            && item.text_options == Default::default()
                            && prepared::TextCache::handles_plain(text, font_size);
                        let b = if plain {
                            let mut cache = self.text_cache.borrow_mut();
                            let laid = if unwrapped {
                                cache.layout_plain_unwrapped(
                                    &mut fonts,
                                    text,
                                    font_size,
                                    item.font,
                                    bounds.width,
                                )
                            } else {
                                cache.layout_plain(
                                    &mut fonts,
                                    text,
                                    font_size,
                                    item.font,
                                    Some(bounds.width),
                                )
                            };
                            let starts = laid.line_starts();
                            let glyphs = glyphs_of(
                                laid.runs(Some(bounds.height)),
                                &starts,
                                None,
                                &item.font,
                                self.scale,
                            );
                            Err((glyphs, Vec::new(), laid.key()))
                        } else if let Some(rich) = prepared {
                            let mut cache = self.text_cache.borrow_mut();
                            let laid = if unwrapped {
                                cache.layout_unwrapped(&mut fonts, rich, bounds.width)
                            } else {
                                cache.layout(&mut fonts, rich, Some(bounds.width))
                            };
                            let starts = laid.line_starts();
                            let glyphs = glyphs_of(
                                laid.runs(Some(bounds.height)),
                                &starts,
                                Some(rich),
                                &item.font,
                                self.scale,
                            );
                            let decorations = text::rich_decorations_in(
                                laid.runs(Some(bounds.height)),
                                &starts,
                                rich,
                            );
                            Err((glyphs, decorations, laid.key()))
                        } else if let Some(rich) = &rich {
                            Ok(if unwrapped {
                                text::rich_buffer_unwrapped(
                                    &mut fonts,
                                    rich,
                                    bounds.width,
                                    bounds.height,
                                )
                            } else {
                                text::rich_buffer(
                                    &mut fonts,
                                    rich,
                                    Some(bounds.width),
                                    Some(bounds.height),
                                )
                            })
                        } else if item.text_options != Default::default() {
                            Ok(text::display_buffer(
                                &mut fonts,
                                text,
                                item.font,
                                font_size,
                                Some(bounds.width),
                                Some(bounds.height),
                                item.text_options,
                            ))
                        } else {
                            let mut b = Buffer::new(
                                &mut fonts,
                                Metrics::new(
                                    font_size.max(1.),
                                    item.font.line_height.resolve(font_size),
                                ),
                            );
                            b.set_size(Some(bounds.width), Some(bounds.height));
                            if unwrapped {
                                b.set_wrap(cosmic_text::Wrap::None);
                            }
                            text::set_buffer_text_shaping(
                                &mut fonts,
                                &mut b,
                                text,
                                item.font,
                                font_size.max(1.),
                                text::shaping(unwrapped),
                            );
                            b.shape_until_scroll(&mut fonts, false);
                            Ok(b)
                        };
                        let (glyphs, decorations, prepared) = match b {
                            Err((glyphs, decorations, key)) => (glyphs, decorations, Some(key)),
                            Ok(b) => (
                                glyphs_of(
                                    b.layout_runs(),
                                    &text::line_starts(&b),
                                    rich.as_deref(),
                                    &item.font,
                                    self.scale,
                                ),
                                rich.as_ref()
                                    .map_or_else(Vec::new, |rich| text::rich_decorations(&b, rich)),
                                None,
                            ),
                        };
                        let shaped = Shaped {
                            decorations,
                            rich: rich.clone(),
                            text_options: item.text_options,
                            font: item.font.clone(),
                            text: text.clone(),
                            size: font_size,
                            width: bounds.width,
                            height: bounds.height,
                            unwrapped,
                            glyphs,
                            quads: Vec::new(),
                            atlas_epoch: 0,
                            raster_origin: (f32::NAN, f32::NAN),
                            last_used: self.frame,
                            prepared,
                        };
                        self.shaped_bytes += shaped.bytes();
                        self.shapes.insert(item.id, shaped);
                        stats.shaped_nodes += 1;
                    }
                    let mut shaped = self.shapes.remove(&item.id).unwrap();
                    self.shaped_bytes -= shaped.bytes();
                    let origin = (
                        (bounds.x * self.scale).rem_euclid(1.),
                        (bounds.y * self.scale).rem_euclid(1.),
                    );
                    if shaped.atlas_epoch != self.atlas_epoch || shaped.raster_origin != origin {
                        shaped.quads.clear();
                        shaped.quads.extend(
                            shaped
                                .decorations
                                .iter()
                                .filter(|d| d.background)
                                .map(rich_decoration_quad),
                        );
                        for glyph in &shaped.glyphs {
                            // Rasterize at the final physical phase. Translating an
                            // antialiased bitmap by a fraction of a pixel blurs it
                            // again; include the baseline before vertical hinting.
                            let (key, x, y) = glyph.at(origin);
                            if let Some(a) = self.glyph(key, &mut stats)? {
                                shaped.quads.push(Quad {
                                    rect: [
                                        (x as f32 + a.left as f32 - origin.0) / self.scale,
                                        (y as f32 - a.top as f32 - origin.1) / self.scale,
                                        a.width as f32 / self.scale,
                                        a.height as f32 / self.scale,
                                    ],
                                    uv: [
                                        a.x as f32 / self.atlas_size as f32,
                                        a.y as f32 / self.atlas_size as f32,
                                        a.width as f32 / self.atlas_size as f32,
                                        a.height as f32 / self.atlas_size as f32,
                                    ],
                                    color: glyph.color.map_or([0.; 4], |c| rgba(c, 1.)),
                                    fade: [0.; 4],
                                    options: [0., 1., 0., 0.],
                                    shape: [0.; 4],
                                    border: [0.; 4],
                                    mask: NO_MASK,
                                });
                            }
                        }
                        shaped.quads.extend(
                            shaped
                                .decorations
                                .iter()
                                .filter(|d| !d.background)
                                .map(rich_decoration_quad),
                        );
                        shaped.atlas_epoch = self.atlas_epoch;
                        shaped.raster_origin = origin;
                        stats.geometry_rebuilds += 1;
                    }
                    for cached in &shaped.quads {
                        let mut quad = *cached;
                        quad.rect[0] += bounds.x;
                        quad.rect[1] += bounds.y;
                        quad.color = if rich.is_some() {
                            [
                                cached.color[0],
                                cached.color[1],
                                cached.color[2],
                                cached.color[3] * item.effects.opacity,
                            ]
                        } else {
                            rgba(color, item.effects.opacity)
                        };
                        quad.fade = fade;
                        quad.options[0] = item.effects.edge_fade;
                        quads.push(quad);
                    }
                    shaped.last_used = self.frame;
                    let bytes = shaped.bytes();
                    if bytes <= SHAPE_BUDGET {
                        let mut used = self.shaped_bytes;
                        while self.shapes.len() >= 1024 || used + bytes > SHAPE_BUDGET {
                            let Some(id) = self
                                .shapes
                                .iter()
                                .min_by_key(|(_, s)| s.last_used)
                                .map(|(id, _)| *id)
                            else {
                                break;
                            };
                            used -= self.shapes.remove(&id).unwrap().bytes();
                        }
                        self.shaped_bytes = used + bytes;
                        self.shapes.insert(item.id, shaped);
                    }
                }
                NodeKind::Container(_) => {}
            }
            // Under a `fade_edges` container, fade by its bands in the shader.
            if let Some((rect, [top, bottom])) = item.mask {
                let mask = fade_parameters(rect, [top, bottom]);
                for quad in &mut quads[start as usize..] {
                    quad.mask = mask;
                }
            }
            // When the clip only culls whole quads, apply it on the CPU and
            // draw under the viewport, so a change of clip never splits a
            // batch: neighbouring text, icons and rows become one draw. Quads
            // it would cut, transformed quads and filters keep the scissor.
            if geometry_offset == 0
                && self.split_shading
                && self.opaque_interiors
                && item.effects.blur_radius <= 0.
            {
                opaque_interior(&mut quads, start, self.scale, clip, &damage);
            }
            let mut draw_clip = clip;
            let limit = snap_out(clip, self.scale);
            if geometry_offset == 0
                && item.effects.blur_radius <= 0.
                && quads[start as usize..]
                    .iter()
                    .all(|quad| quad.options[2] < 0.5 && !crosses(quad, limit))
            {
                let mut kept = start as usize;
                for index in start as usize..quads.len() {
                    if inside(&quads[index], limit) {
                        quads[kept] = quads[index];
                        kept += 1;
                    }
                }
                quads.truncate(kept);
                draw_clip = viewport;
            }
            if quads.len() as u32 > start || item.effects.blur_radius > 0. {
                let draw = Draw {
                    diagnostic_source: diagnose.then(|| draw_diagnostics::Source::new(
                        &item, self.geometry.clip_count(geometry_offset))),
                    transform: item.transform,
                    geometry_offset,
                    geometry_hash,
                    geometry: None,
                    shading: Shading::Full,
                    blur_mask: item.mask,
                    layer: None,
                    blur: (item.effects.blur_radius > 0.).then_some((
                        item.id,
                        world_bounds,
                        item.effects,
                    )),
                    start,
                    end: quads.len() as u32,
                    clip: draw_clip,
                    bounds: item.transform.bounds(quad_bounds(&quads[start as usize..])),
                    image: if let Some(image) = canvas_image {
                        Some(image)
                    } else if let NodeKind::Image(image) = item.kind {
                        Some(image.id())
                    } else {
                        None
                    },
                };
                if let Some(image) = draw.image {
                    self.frame_image_ids.insert(image);
                }
                push_draw(&mut draws, &quads, draw, self.split_shading, self.scale);
            }
        }
        let needed = quads.len() * std::mem::size_of::<Quad>();
        if needed > VERTEX_BUDGET {
            return Err(GpuError("frame geometry exceeds 32 MiB budget".into()));
        }
        let mut encoder = self
            .procedural_encoder
            .take()
            .unwrap_or_else(|| self.device.create_command_encoder(&Default::default()));
        stats.paint_geometry_buffer_allocations += self.geometry.upload(
            &self.device,
            &self.context.inner.geometry_layout,
            self.belt.get_mut(),
            &mut encoder,
        );
        if self.geometry_batching != GeometryBatching::DynamicUniform {
            stats.paint_geometry_buffer_allocations += self.geometry.upload_instances(
                &self.device,
                &self.context.indexed_geometry().geometry_layout,
                &draws,
                quads.len(),
                self.belt.get_mut(),
                &mut encoder,
            );
        }
        for draw in &mut draws {
            draw.geometry = Some(self.geometry.bind.clone());
        }
        // Glyphs rasterized while building this frame land before any pass
        // samples them. Counted as glyph uploads, not scene render passes.
        let upload_stamp = self.profile_start(&mut encoder, "uploads");
        self.texture_uploads.encode(&self.device, &mut encoder);
        profiling::Profiler::end(&mut encoder, upload_stamp);
        // Build the next retained frame directly in the spare: shifted scroll
        // destinations plus unchanged pixels outside them cover the surface.
        // Swapping avoids copying the scrolled pixels back a second time.
        if !copies.is_empty() || self.retain_scroll_frame && !self.in_layer {
            let copy_stamp = self.profile_start(&mut encoder, "scroll_copy");
            let spare = self.spare_texture();
            let mut replaced = copies.clone();
            replaced.extend(
                damage
                    .iter()
                    .filter_map(|r| scissor(*r, self.scale, self.width, self.height))
                    .map(|r| (r, (0, 0))),
            );
            for (x, y, w, h) in unchanged_scissors(self.width, self.height, &replaced) {
                encoder.copy_texture_to_texture(
                    texel_at(&self.target, x, y),
                    texel_at(&spare, x, y),
                    wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                );
                stats.copied_pixels += u64::from(w) * u64::from(h);
            }
            for ((x, y, w, h), (sx, sy)) in &copies {
                let extent = wgpu::Extent3d {
                    width: *w,
                    height: *h,
                    depth_or_array_layers: 1,
                };
                let (from_x, from_y) = ((*x as i32 + sx) as u32, (*y as i32 + sy) as u32);
                encoder.copy_texture_to_texture(
                    texel_at(
                        self.scroll_source.as_ref().unwrap_or(&self.target),
                        from_x,
                        from_y,
                    ),
                    texel_at(&spare, *x, *y),
                    extent,
                );
                stats.copied_pixels += u64::from(*w) * u64::from(*h);
                stats.scroll_copies += 1;
            }
            profiling::Profiler::end(&mut encoder, copy_stamp);
            let next = self.spare.take().expect("created above");
            self.swap_retained(next);
        }
        let bytes: &[u8] = bytemuck::cast_slice(&quads);
        let vertices = if let Some(mapped) = &mut self.mapped {
            let mapped = mapped.get_mut();
            let (vertices, allocated) = mapped.write(bytes);
            stats.vertex_buffer_allocations += usize::from(allocated);
            self.vertex_capacity = mapped.bytes();
            vertices
        } else {
            if needed > self.vertex_capacity {
                self.vertex_capacity = needed.next_power_of_two();
                self.vertices = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("retained frame vertices"),
                    size: self.vertex_capacity as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                stats.vertex_buffer_allocations += 1;
            }
            let vertices = self.vertices.clone();
            self.belt
                .get_mut()
                .write_buffer(
                    &mut encoder,
                    &vertices,
                    0,
                    wgpu::BufferSize::new(bytes.len() as u64).expect("background quad"),
                )
                .copy_from_slice(bytes);
            vertices
        };
        // Geometry uses encoded belt copies even when glyph vertices are mapped.
        self.belt.get_mut().finish();
        // A clean filter elsewhere in the retained target needs no replay and
        // must not force every unrelated quad into its own render pass.
        let has_damaged_blur = has_blur
            && draws.iter().any(|draw| {
                draw.blur.is_some_and(|(_, bounds, _)| {
                    damage.iter().any(|region| {
                        region
                            .intersection(draw.clip)
                            .and_then(|rect| rect.intersection(bounds))
                            .and_then(|output| scissor(output, self.scale, self.width, self.height))
                            .is_some()
                    })
                })
            });
        let regions: Vec<(Rect, (u32, u32, u32, u32))> = damage
            .iter()
            .filter_map(|region| {
                scissor(*region, self.scale, self.width, self.height).map(|s| (*region, s))
            })
            .collect();
        for (_, (_, _, w, h)) in &regions {
            stats.damaged_pixels += u64::from(*w) * u64::from(*h);
        }

        // The part of a filter's output inside `region`, if it reaches it.
        let filtered = |draw: &Draw, region: &Rect| {
            let (_, bounds, _) = draw.blur?;
            region.intersection(draw.clip)?.intersection(bounds)
        };
        // A filter samples everything painted before it, so each filter a
        // damaged region reaches ends a pass. Between filters, every region
        // shares one pass: a scissor per region, draws batched within it.
        let splits: Vec<usize> = if has_damaged_blur {
            (0..draws.len())
                .filter(|&i| {
                    regions
                        .iter()
                        .any(|(region, _)| filtered(&draws[i], region).is_some())
                })
                .collect()
        } else {
            Vec::new()
        };
        // The window's own frame can be drawn at presentation in the same pass
        // as the drawable (`present_single_pass`). With damaged filters, only
        // the draws after the last one wait: everything before it must land
        // in the target for the filter to sample.
        // Under the topmost opaque quad covering a whole region, nothing shows:
        // skip the clear and every quad below it. Filters sample what lies
        // beneath them, so frames that replay one draw everything.
        let first = if splits.is_empty() {
            occluded_from(&quads, &draws, &regions, self.scale)
        } else {
            vec![0; regions.len()]
        };
        let defer = !self.in_layer
            && (!self.present_unpremultiplies || self.background.3 == 255)
            && self.single_pass_format.is_some()
            && !self.copy_present
            && copies.is_empty();
        let mut from = 0;
        // Each filter's vertical half opens the pass after its horizontal one.
        let mut vertical = None;
        let fingerprints = blur_cache::fingerprints(
            &draws,
            &quads,
            &self.layers,
            self.scale,
            self.background,
            self.atlas_epoch,
            self.blur_algorithm,
        );
        for (segment, end) in splits.iter().copied().chain([draws.len()]).enumerate() {
            if defer && !diagnose && end == draws.len() {
                stats.render_passes += 1;
                let geometry = self.quad_geometry().clone();
                *self.deferred.get_mut() = Some(Deferred {
                    vertices: vertices.clone(),
                    draws: draws.split_off(from),
                    regions,
                    viewport: self.viewport_bind.clone(),
                    geometry,
                    blur: vertical.take(),
                    clear: segment == 0,
                    disjoint: !has_blur,
                    first,
                });
                break;
            }
            if diagnose && (segment == 0 || end > from) {
                let stage = if splits.is_empty() { "repaint" }
                    else if segment == 0 { "pre_blur" } else { "post_blur" };
                self.encode_diagnostic_segment(
                    &mut encoder, &vertices, &quads, &regions, &draws[from..end],
                    &first, segment == 0, vertical.take(), stage, &mut stats,
                );
            } else if segment == 0 || end > from {
                stats.render_passes += 1;
                let stamp = self.profile_pass(if self.in_layer {
                    "layer_repaint"
                } else if splits.is_empty() {
                    "repaint"
                } else if segment == 0 {
                    "repaint_pre_blur"
                } else {
                    "repaint_post_blur"
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("damage"),
                    timestamp_writes: stamp.as_ref().map(|s| s.writes()),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                if let Some(blur) = vertical.take() {
                    let pipeline = self.blur_vertical(&blur);
                    Self::encode_vertical(&mut pass, pipeline, &blur, &mut stats);
                }
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.set_bind_group(0, &self.bind, &[]);
                pass.set_bind_group(1, self.viewport_bind.as_ref(), &[]);
                pass.set_bind_group(2, self.quad_geometry(), &[0]);
                let quads = self.quad_pipelines();
                for (index, (region, (x, y, w, h))) in regions.iter().enumerate() {
                    pass.set_scissor_rect(*x, *y, *w, *h);
                    let first = first[index];
                    if segment == 0 && first == 0 {
                        pass.set_pipeline(quads.clear(self.split_shading));
                        pass.set_bind_group(2, self.quad_geometry(), &[0]);
                        pass.set_bind_group(0, &self.bind, &[]);
                        pass.draw(0..6, 0..1);
                        stats.draw_calls += 1;
                    }
                    self.encode_draws(
                        &mut pass,
                        &|shading| {
                            quads.get(
                                selected_shading(
                                    shading,
                                    self.regular_full_shader,
                                    self.nearest_image_shader,
                                ),
                                self.clip_interiors,
                            )
                        },
                        *region,
                        &draws[from..end],
                        first,
                        &mut stats,
                    );
                }
            }
            let Some(draw) = draws.get(end) else {
                break;
            };
            let (id, _, _) = draw.blur.expect("split at a filter");
            let outputs = regions
                .iter()
                .filter_map(|(region, _)| filtered(draw, region));
            vertical = self.blur(&mut encoder, fingerprints[&id], draw, outputs);
            if let Some(blur) = &vertical {
                stats.draw_calls += blur.predraws;
                stats.render_passes += blur.prepasses;
                stats.blur_passes += blur.prepasses + 1;
                stats.blur_cache_hits += usize::from(blur.prepasses == 0);
                stats.blur_texture_allocations += blur.allocations;
                if blur.prepasses > 0 {
                    for &(_, _, w, h) in &blur.scissors {
                        stats.filtered_pixels += u64::from(w) * u64::from(h);
                    }
                }
            }
            // The filtered draw's own quads open the next pass.
            from = end;
        }
        *self.pending.get_mut() = Some(encoder);
        self.fresh = false;
        self.frame_quads = quads;
        Ok(stats)
    }
    /// The spare retained texture (single-pass presentation's other target),
    /// created at the window's size if needed.
    fn spare_texture(&mut self) -> wgpu::Texture {
        if self
            .spare
            .as_ref()
            .is_none_or(|t| t.texture.width() != self.width || t.texture.height() != self.height)
        {
            let texture = texture(&self.device, self.width, self.height, "retained output");
            let view = texture.create_view(&Default::default());
            self.spare = Some(Retained {
                texture,
                view,
                copy_bind: None,
                blit_bind: None,
            });
        }
        self.spare.as_ref().expect("created above").texture.clone()
    }
    /// Split the last flush's scroll moves into pixel copies this frame can
    /// apply and clips it must repaint instead. A copy is exact only when the
    /// retained target holds the previous flush, both the clip and the shift
    /// land on whole pixels, and within the clip nothing but the content
    /// changes position: whatever lies under it is hidden by one opaque
    /// rectangle, and anything over it is repainted anyway.
    fn scroll_copies(
        &self,
        scene: &Scene,
        damage: &[Rect],
        known: Option<&ScrollState>,
    ) -> (Vec<ScrollCopy>, Vec<Rect>) {
        let serial = scene.flush_serial();
        // A no-op scene flush retains its last moves and serial. Each renderer
        // consumes those moves once; replaying them would repaint a clean clip.
        if !self.fresh && self.rendered_serial == Some(serial) {
            return (Vec::new(), Vec::new());
        }
        let mut copies = Vec::new();
        let mut repaint = Vec::new();
        // On Apple GPUs a process's first texture blit makes the driver hold
        // over 100 MB of graphics memory (see vendor/wgpu-hal/LUMINE_PATCH.md),
        // and redrawing a scrolled view costs the GPU little. Repaint there.
        let copyable = !cfg!(target_os = "macos");
        for scroll in scene.scroll_moves() {
            if scroll.serial != serial {
                continue;
            }
            let whole = |value: f32| {
                let scaled = value * self.scale;
                ((scaled - scaled.round()).abs() < 1e-3).then_some(scaled.round() as i32)
            };
            let pixels = (|| {
                let clip = scroll.clip.intersection(Rect::new(
                    0.,
                    0.,
                    self.width as f32 / self.scale,
                    self.height as f32 / self.scale,
                ))?;
                let (x, y) = (whole(clip.x)?, whole(clip.y)?);
                let (right, bottom) = (whole(clip.x + clip.width)?, whole(clip.y + clip.height)?);
                let (dx, dy) = (whole(scroll.dx)?, whole(scroll.dy)?);
                // Destination: the clip minus the exposed strip; source: the
                // same pixels before the move.
                let (x0, x1) = (x.max(x + dx), right.min(right + dx));
                let (y0, y1) = (y.max(y + dy), bottom.min(bottom + dy));
                (x0 < x1 && y0 < y1 && x0 >= 0 && y0 >= 0).then(|| {
                    (
                        (x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32),
                        (-dx, -dy),
                    )
                })
            })();
            if let Some(copy) = pixels
                && copyable
                && !self.fresh
                && self.rendered_serial == Some(serial - 1)
                && let Some(repair) = known
                    .filter(|s| s.node == scroll.node)
                    .map(|s| s.repair.clone())
                    .or_else(|| copy_repair(scene, scroll, damage).map(|r| r.regions))
            {
                copies.push(copy);
                repaint.extend(repair);
            } else {
                repaint.push(scroll.clip);
            }
        }
        (copies, repaint)
    }
    fn scroll_state_for(&self, scene: &Scene, damage: &[Rect]) -> Option<ScrollState> {
        if (!self.fresh && self.rendered_serial == Some(scene.flush_serial()))
            || !self.scroll_phase_cache || scene.scroll_moves().len() != 1 || scene.has_backdrop_blur()
        {
            return None;
        }
        let scroll = &scene.scroll_moves()[0];
        if scroll.serial != scene.flush_serial() {
            return None;
        }
        let repair = copy_repair(scene, scroll, damage)?;
        let clip = scroll.clip.intersection(Rect::new(
            0.,
            0.,
            self.width as f32 / self.scale,
            self.height as f32 / self.scale,
        ))?;
        let origin = scene.bounds(scroll.node);
        Some(ScrollState {
            node: scroll.node,
            clip,
            origin: (origin.x, origin.y),
            revision: (
                scene.content_revision(),
                scene.raster_revision(),
                scene.layer_revision(scroll.node),
            ),
            foreground: repair.foreground,
            repair: repair.regions,
        })
    }
    /// Ablate the bounded fractional-origin cache without changing scrolling.
    pub fn set_scroll_phase_cache(&mut self, enabled: bool) {
        self.flush_pending();
        self.scroll_phase_cache = enabled;
        self.scroll_history = None;
        self.scroll_state = None;
    }
    /// Submit commands recorded by the last render, if any.
    fn flush_pending(&self) {
        self.flush_pending_inner(false);
    }
    fn flush_pending_inner(&self, finish_profile: bool) {
        let Some(mut pending) = self.pending.borrow_mut().take() else {
            return;
        };
        // Not presented (skipped, hidden or read back): draw it the two-pass
        // way so the retained target is complete.
        if let Some(deferred) = self.deferred.borrow_mut().take() {
            let stamp = self.profile_pass(if deferred.blur.is_some() {
                "deferred_repaint_post_blur"
            } else { "deferred_repaint" });
            let mut pass = pending.begin_render_pass(&wgpu::RenderPassDescriptor {
                timestamp_writes: stamp.as_ref().map(|s| s.writes()),
                label: Some("damage"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            let mut stats = GpuStats::default();
            if let Some(blur) = &deferred.blur {
                let pipeline = self.blur_vertical(blur);
                Self::encode_vertical(&mut pass, pipeline, blur, &mut stats);
            }
            self.encode_damage(&mut pass, &deferred, self.quad_pipelines(), &mut stats);
        }
        self.submit_profiled(pending, finish_profile);
        self.belt.borrow_mut().recall();
        self.submitted();
        if let Some(mapped) = &self.mapped {
            mapped.borrow_mut().submitted();
        }
    }
    fn submitted(&self) {
        #[cfg(target_os = "macos")]
        if let Some(cache) = &self.native_surfaces {
            cache.submitted();
        }
    }
    /// Clear and redraw every damaged region of a deferred frame.
    fn encode_damage(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &Deferred,
        pipelines: &QuadPipelines,
        stats: &mut GpuStats,
    ) {
        pass.set_vertex_buffer(0, frame.vertices.slice(..));
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_bind_group(1, frame.viewport.as_ref(), &[]);
        pass.set_bind_group(2, &frame.geometry, &[0]);
        let clear = pipelines.clear(self.split_shading);
        let quads = pipelines.get(Shading::Full, self.clip_interiors);
        let pick = |shading| {
            pipelines.get(
                selected_shading(
                    shading,
                    self.regular_full_shader,
                    self.nearest_image_shader,
                ),
                self.clip_interiors,
            )
        };
        let first = |index: usize| frame.first.get(index).copied().unwrap_or(0);
        if !frame.clear {
            pass.set_pipeline(quads);
            for (index, (region, (x, y, w, h))) in frame.regions.iter().enumerate() {
                pass.set_scissor_rect(*x, *y, *w, *h);
                self.encode_draws(pass, &pick, *region, &frame.draws, first(index), stats);
            }
            return;
        }
        if frame.disjoint {
            // One pipeline switch per phase instead of two per region; many
            // small animations each damage a region of their own.
            pass.set_pipeline(clear);
            pass.set_bind_group(2, &frame.geometry, &[0]);
            for (index, (_, (x, y, w, h))) in frame.regions.iter().enumerate() {
                if first(index) == 0 {
                    pass.set_scissor_rect(*x, *y, *w, *h);
                    pass.draw(0..6, 0..1);
                    stats.draw_calls += 1;
                }
            }
            pass.set_pipeline(quads);
            for (index, (region, _)) in frame.regions.iter().enumerate() {
                self.encode_draws(pass, &pick, *region, &frame.draws, first(index), stats);
            }
            return;
        }
        for (index, (region, (x, y, w, h))) in frame.regions.iter().enumerate() {
            pass.set_scissor_rect(*x, *y, *w, *h);
            if first(index) == 0 {
                pass.set_pipeline(clear);
                pass.set_bind_group(2, &frame.geometry, &[0]);
                pass.set_bind_group(0, &self.bind, &[]);
                pass.draw(0..6, 0..1);
                stats.draw_calls += 1;
            }
            pass.set_pipeline(quads);
            self.encode_draws(pass, &pick, *region, &frame.draws, first(index), stats);
        }
    }
    /// One render pass per frame: copy the retained image into the spare
    /// target and `output`, then redraw the damage into both, and swap. Every
    /// pixel matches the two-pass route because `output`'s format needs no
    /// conversion (see `single_pass_format`).
    fn present_single_pass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Deferred,
        output: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) {
        if self.single_pass.as_ref().is_none_or(|p| p.format != format) {
            let quad_shader = self.quad_pipelines().shader.clone();
            let quad_layout = self.quad_pipelines().layout.clone();
            let shader = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("single-pass copy"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("copy_both.wgsl").into()),
                });
            let copy = self
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("single-pass copy"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs"),
                        compilation_options: Default::default(),
                        targets: &[Some(FORMAT.into()), Some(format.into())],
                    }),
                    multiview_mask: None,
                    cache: None,
                });
            self.target_copy_bind = None;
            if let Some(spare) = &mut self.spare {
                spare.copy_bind = None;
            }
            self.single_pass = Some(SinglePass {
                format,
                quads: QuadPipelines {
                    device: self.device.clone(),
                    shader: quad_shader,
                    layout: quad_layout,
                    full: Default::default(),
                    regular_full: Default::default(),
                    nearest_image: Default::default(),
                    full_clear: Default::default(),
                    basic: Default::default(),
                    basic_clear: Default::default(),
                    drawable: Some(format),
                },
                copy,
            });
        }
        if self
            .spare
            .as_ref()
            .is_none_or(|t| t.texture.width() != self.width || t.texture.height() != self.height)
        {
            let texture = texture(&self.device, self.width, self.height, "retained output");
            let view = texture.create_view(&Default::default());
            self.spare = Some(Retained {
                texture,
                view,
                copy_bind: None,
                blit_bind: None,
            });
        }
        if frame.blur.is_some() {
            let device = &self.device;
            self.blur_composite
                .as_mut()
                .expect("filtered")
                .both(device, format);
        }
        let pipelines = self.single_pass.as_ref().expect("created above");
        let previous = self
            .target_copy_bind
            .get_or_insert_with(|| blit_group(&self.device, &pipelines.copy, &self.view))
            .clone();
        let spare = self.spare.take().expect("created above");
        let clear = wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            store: wgpu::StoreOp::Store,
        };
        {
            let stamp = self.profile_pass("presentation_and_repaint");
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                timestamp_writes: stamp.as_ref().map(|s| s.writes()),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &spare.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: clear,
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: output,
                        depth_slice: None,
                        resolve_target: None,
                        ops: clear,
                    }),
                ],
                ..Default::default()
            });
            pass.set_pipeline(&pipelines.copy);
            pass.set_bind_group(0, &previous, &[]);
            pass.draw(0..3, 0..1);
            let mut stats = GpuStats::default();
            if let Some(blur) = &frame.blur {
                let both = self.blur_composite.as_ref().and_then(|b| b.both.as_ref());
                let (_, pipeline) = both.expect("created above");
                Self::encode_vertical(&mut pass, pipeline, blur, &mut stats);
            }
            self.encode_damage(&mut pass, frame, &pipelines.quads, &mut stats);
            self.present_stats.draw_calls += stats.draw_calls + 1;
            self.present_stats.instances += stats.instances;
        }
        if !frame.regions.is_empty() {
            self.single_pass_frames += 1;
        }
        self.swap_retained(spare);
    }
    fn swap_retained(&mut self, spare: Retained) {
        // The spare now holds this frame; the old target becomes the spare,
        // keeping its view and bind groups for when it is drawn into again.
        let Retained {
            texture,
            view,
            copy_bind,
            blit_bind,
        } = spare;
        let old = Retained {
            texture: std::mem::replace(&mut self.target, texture),
            view: std::mem::replace(&mut self.view, view),
            copy_bind: std::mem::replace(&mut self.target_copy_bind, copy_bind),
            blit_bind: std::mem::replace(&mut self.blit_bind, blit_bind),
        };
        self.spare = Some(old);
        if self.blit_bind.is_none() {
            self.blit_bind = self
                .blit
                .as_ref()
                .map(|p| blit_group(&self.device, p, &self.view));
        }
    }
    /// Encode `draws` clipped to `region`. Consecutive draws sharing a
    /// texture and clip become one instanced call; their quads are contiguous,
    /// so paint order and each draw's affine binding hold.
    fn encode_draws<'p>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipelines: &dyn Fn(Shading) -> &'p wgpu::RenderPipeline,
        region: Rect,
        draws: &[Draw],
        first: u32,
        stats: &mut GpuStats,
    ) {
        let mut current = None;
        let unblended = region.width * region.height * self.scale * self.scale >= SPLIT_PIXELS;
        for batch in draw_diagnostics::batches(
            draws,
            region,
            first,
            unblended,
            self.geometry_batching == GeometryBatching::IndexedMerged,
        ) {
            self.encode_batch(pass, pipelines, &mut current, batch, stats);
        }
    }

    fn encode_batch<'p>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipelines: &dyn Fn(Shading) -> &'p wgpu::RenderPipeline,
        current: &mut Option<Shading>,
        batch: Batch,
        stats: &mut GpuStats,
    ) {
        let Some((x, y, w, h)) = scissor(batch.rect, self.scale, self.width, self.height) else {
            return;
        };
        if *current != Some(batch.shading) {
            pass.set_pipeline(pipelines(batch.shading));
            *current = Some(batch.shading);
        }
        pass.set_scissor_rect(x, y, w, h);
        let (image, layer) = batch.source;
        pass.set_bind_group(
            0,
            image.and_then(|id| self.images.get(&id)).map_or_else(
                || {
                    layer
                        .and_then(|id| self.layers.get(&id))
                        .map_or(&self.bind, |l| &l.bind)
                },
                |entry| &entry.0,
            ),
            &[],
        );
        if self.geometry_batching == GeometryBatching::DynamicUniform {
            pass.set_bind_group(2, batch.geometry.as_ref(), &[batch.geometry_offset]);
        } else {
            pass.set_bind_group(2, self.quad_geometry(), &[0]);
        }
        pass.draw(0..6, batch.start..batch.end);
        stats.draw_calls += 1;
        stats.instances += (batch.end - batch.start) as usize;
    }
    fn blur(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        fingerprint: u64,
        draw: &Draw,
        outputs: impl IntoIterator<Item = Rect>,
    ) -> Option<VerticalBlur> {
        let (id, bounds, effects) = draw.blur.expect("filter draw");
        let clip = draw.clip;
        let mask = draw.blur_mask;
        let sigma = (effects.blur_radius * self.scale).clamp(0.1, 64.);
        // Samples stay inside the filter's visible pixels (see blur.wgsl).
        let (ax, ay, aw, ah) = bounds
            .intersection(clip)
            .and_then(|area| scissor(area, self.scale, self.width, self.height))?;
        let scissors: Vec<_> = outputs
            .into_iter()
            .filter_map(|output| scissor(output, self.scale, self.width, self.height))
            .collect();
        if scissors.is_empty() {
            return None;
        }
        let composite = blur_cache::composite_key(bounds, effects, mask);
        if self
            .blur_cache
            .get(&id)
            .is_some_and(|c| c.fingerprint == fingerprint)
        {
            let mut cached = self.blur_cache.remove(&id).expect("checked above");
            if cached.composite != composite {
                cached.filter.bind = self.blur_composite_bind(
                    &cached.textures[0],
                    cached.textures.last().unwrap(),
                    bounds,
                    (ax, ay),
                    effects,
                    mask,
                );
                cached.composite = composite;
            }
            cached.used = self.frame;
            let mut filter = cached.filter.clone();
            filter.scissors = scissors;
            filter.prepasses = 0;
            filter.predraws = 0;
            filter.allocations = 0;
            filter.geometry = draw
                .geometry
                .clone()
                .map(|bind| (bind, draw.geometry_offset));
            self.blur_cache.insert(id, cached);
            return Some(filter);
        }
        let mut textures = self
            .blur_cache
            .remove(&id)
            .map_or_else(Vec::new, |c| c.textures);
        if self.blur_algorithm == BlurAlgorithm::DualKawase {
            let (mut filter, textures) =
                self.kawase_blur(encoder, (ax, ay, aw, ah), scissors, sigma, textures);
            filter.bind = self.blur_composite_bind(
                &textures[0],
                &textures[1],
                bounds,
                (ax, ay),
                effects,
                mask,
            );
            filter.geometry = draw
                .geometry
                .clone()
                .map(|bind| (bind, draw.geometry_offset));
            self.cache_blur(id, fingerprint, filter.clone(), textures, composite);
            return Some(filter);
        }
        if self.blur_pipelines.is_none() {
            self.blur_pipelines = Some(BlurPipelines::new(&self.device));
        }
        let pipelines = self.blur_pipelines.as_mut().unwrap();
        if !pipelines.weights.contains_key(&sigma.to_bits()) {
            if pipelines.weights.len() >= 64 {
                pipelines.weights.clear();
            }
            let buffer = upload::init_buffer(
                &self.device,
                self.mapped.is_some(),
                "Gaussian paired weights",
                bytemuck::cast_slice(&gaussian::samples(sigma)),
                wgpu::BufferUsages::STORAGE,
            );
            pipelines.weights.insert(sigma.to_bits(), buffer);
        }
        let allocations = blur_cache::reserve(&self.device, &mut textures, 0, aw, ah)
            + blur_cache::reserve(&self.device, &mut textures, 1, aw, ah)
            + blur_cache::reserve_scratch(&self.device, &mut self.gaussian_scratch, aw, ah);
        let source = &textures[0];
        let intermediate = self.gaussian_scratch.as_ref().expect("reserved above");
        let split =
            (source.width(), source.height()) == (intermediate.width(), intermediate.height());
        if !split {
            self.blur_pipelines
                .as_mut()
                .expect("created above")
                .crop(&self.device);
        }
        let pipelines = self.blur_pipelines.as_ref().expect("created above");
        let source_view = source.create_view(&Default::default());
        let intermediate_view = intermediate.create_view(&Default::default());
        let uniform = |vertical: bool| {
            let params: [f32; 16] = [
                if vertical { 0. } else { 1. },
                if vertical { 1. } else { 0. },
                sigma,
                if vertical { 3. } else { -1. },
                bounds.x * self.scale,
                bounds.y * self.scale,
                bounds.width * self.scale,
                bounds.height * self.scale,
                effects.opacity,
                effects.edge_fade * self.scale,
                // The filtered target's size: scratch textures may be larger.
                self.width as f32,
                self.height as f32,
                ax as f32,
                ay as f32,
                (ax + aw) as f32,
                (ay + ah) as f32,
            ];
            upload::init_buffer(
                &self.device,
                self.mapped.is_some(),
                "blur parameters",
                bytemuck::cast_slice(&params),
                wgpu::BufferUsages::UNIFORM,
            )
        };
        let texture = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        let horizontal = uniform(false);
        let horizontal = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("horizontal blur"),
            layout: &pipelines.horizontal.get_bind_group_layout(0),
            entries: &[
                texture(0, &self.view),
                texture(1, &self.view),
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: horizontal.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: pipelines.weights[&sigma.to_bits()].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&pipelines.sampler),
                },
            ],
        });
        let vertical = uniform(true);
        let vertical = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vertical blur"),
            layout: &pipelines.layout,
            entries: &[
                texture(0, &intermediate_view),
                texture(1, &source_view),
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: vertical.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: pipelines.weights[&sigma.to_bits()].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&pipelines.sampler),
                },
            ],
        });
        // Horizontal: read the target, write the blur and the original pixels
        // for every output at once. The target is unchanged until the vertical
        // draws, so outputs sharing scratch texels compute the same values.
        let load = wgpu::Operations {
            load: wgpu::LoadOp::Load,
            store: wgpu::StoreOp::Store,
        };
        let attachment = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: load,
            })
        };
        let stamp = self.profile_pass("blur_horizontal");
        let horizontal_attachments = [attachment(&intermediate_view), attachment(&source_view)];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("horizontal blur"),
            timestamp_writes: stamp.as_ref().map(|s| s.writes()),
            color_attachments: if split {
                &horizontal_attachments
            } else {
                &horizontal_attachments[..1]
            },
            ..Default::default()
        });
        pass.set_pipeline(if split {
            &pipelines.horizontal
        } else {
            &pipelines.crop.as_ref().expect("created above").0
        });
        pass.set_bind_group(0, &horizontal, &[]);
        pass.set_viewport(0., 0., aw as f32, ah as f32, 0., 1.);
        pass.set_scissor_rect(0, 0, aw, ah);
        pass.draw(0..3, 0..1);
        drop(pass);
        if !split {
            let stamp = self.profile_pass("blur_original");
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("original blur crop"),
                timestamp_writes: stamp.as_ref().map(|s| s.writes()),
                color_attachments: &[attachment(&source_view)],
                ..Default::default()
            });
            pass.set_pipeline(&pipelines.crop.as_ref().expect("created above").1);
            pass.set_bind_group(0, &horizontal, &[]);
            pass.set_viewport(0., 0., aw as f32, ah as f32, 0., 1.);
            pass.set_scissor_rect(0, 0, aw, ah);
            pass.draw(0..3, 0..1);
        }
        let filtered_view = textures[1].create_view(&Default::default());
        let stamp = self.profile_pass("blur_vertical");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cached vertical blur"),
            timestamp_writes: stamp.as_ref().map(|s| s.writes()),
            color_attachments: &[attachment(&filtered_view)],
            ..Default::default()
        });
        pass.set_pipeline(&pipelines.vertical);
        pass.set_bind_group(0, &vertical, &[]);
        pass.set_viewport(0., 0., aw as f32, ah as f32, 0., 1.);
        pass.set_scissor_rect(0, 0, aw, ah);
        pass.draw(0..3, 0..1);
        drop(pass);
        let bind =
            self.blur_composite_bind(&textures[0], &textures[1], bounds, (ax, ay), effects, mask);
        let filter = VerticalBlur {
            geometry: draw
                .geometry
                .clone()
                .map(|bind| (bind, draw.geometry_offset)),
            bind,
            predraws: 2 + usize::from(!split),
            allocations,
            scissors,
            prepasses: 2 + usize::from(!split),
        };
        self.cache_blur(id, fingerprint, filter.clone(), textures, composite);
        Some(filter)
    }
    /// Composite a completed cached filter over its original backdrop.
    fn encode_vertical(
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        blur: &VerticalBlur,
        stats: &mut GpuStats,
    ) {
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &blur.bind, &[]);
        if let Some((bind, offset)) = &blur.geometry {
            pass.set_bind_group(1, bind, &[*offset]);
        }
        for &(x, y, width, height) in &blur.scissors {
            pass.set_scissor_rect(x, y, width, height);
            pass.draw(0..3, 0..1);
            stats.draw_calls += 1;
        }
    }
    /// Record `id` as `node`'s raster, dropping the texture it replaces.
    fn retire_node_image(&mut self, node: NodeId, id: u64) {
        if let Some(old) = self.node_images.insert(node, id)
            && old != id
            && !self.frame_image_ids.contains(&old)
        {
            self.images.remove(&old);
            self.image_textures.remove(&old);
        }
    }
    // Four full-window RGBA textures allow layered HiDPI UI without unbounded caches.
    fn image_budget_for(width: u32, height: u32) -> usize {
        (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(16)
            .clamp(64 * 1024 * 1024, 256 * 1024 * 1024)
    }
    /// Reclaim uploads not sampled by this frame under pressure. Raster metadata
    /// and mounted images can recreate their textures when they become visible.
    fn ensure_image_budget(&mut self, bytes: usize) -> Result<(), GpuError> {
        let budget = self.image_budget;
        if bytes + self.images.values().map(|entry| entry.1).sum::<usize>() > budget {
            let retained = self.frame_image_ids.clone();
            self.images.retain(|id, _| retained.contains(id));
            self.image_textures.retain(|id, _| retained.contains(id));
            #[cfg(target_os = "macos")]
            if let Some(cache) = &mut self.native_surfaces {
                cache.retain(&retained);
            }
        }
        if bytes + self.images.values().map(|entry| entry.1).sum::<usize>() > budget {
            return Err(GpuError(format!(
                "visible image textures exceed the {} MiB budget",
                budget / (1024 * 1024)
            )));
        }
        Ok(())
    }
    /// The texture of `node`'s canvas raster, uploaded if it is new; the
    /// cache then keeps no pixels. A lost texture rasterizes again.
    fn canvas_texture(
        &mut self,
        node: NodeId,
        stats: &mut GpuStats,
        mut raster: impl FnMut(&mut canvas::CanvasCache) -> Result<canvas::Raster, GpuError>,
    ) -> Result<u64, GpuError> {
        let mut found = raster(&mut self.canvases)?;
        self.retire_node_image(node, found.id);
        if !self.images.contains_key(&found.id) {
            if found.pixels.is_none() {
                self.canvases.forget(node);
                found = raster(&mut self.canvases)?;
                self.retire_node_image(node, found.id);
            }
            let pixels = found.pixels.expect("fresh raster");
            self.upload_image(&pixels)?;
            stats.image_uploads += 1;
        }
        self.canvases.uploaded(node);
        Ok(found.id)
    }
    fn upload_image(&mut self, image: &lumine::image::ImageData) -> Result<(), GpuError> {
        self.upload_image_levels(image, false)
    }
    fn upload_image_levels(
        &mut self,
        image: &lumine::image::ImageData,
        mipmapped: bool,
    ) -> Result<(), GpuError> {
        if image.width() > self.device.limits().max_texture_dimension_2d
            || image.height() > self.device.limits().max_texture_dimension_2d
        {
            return Err(GpuError("image exceeds GPU texture dimensions".into()));
        }
        let levels = if mipmapped {
            image.width().max(image.height()).ilog2() + 1
        } else {
            1
        };
        let bytes = image_mips::bytes(image.width(), image.height(), levels);
        let additional_bytes =
            bytes.saturating_sub(self.images.get(&image.id()).map_or(0, |entry| entry.1));
        self.frame_image_ids.insert(image.id());
        self.ensure_image_budget(additional_bytes)?;
        let mut display = None;
        let texture = if let Some(chain) = image.effect_chain() {
            if !self.image_textures.contains_key(&chain.input.id()) {
                self.upload_image(&chain.input)?;
            }
            self.ensure_image_budget(bytes)?;
            let mut encoder = self
                .procedural_encoder
                .take()
                .unwrap_or_else(|| self.device.create_command_encoder(&Default::default()));
            // Mapped uploads must precede any stage sampling its source.
            self.texture_uploads.encode(&self.device, &mut encoder);
            let result = self
                .chain_cache
                .render(
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    &self.image_textures[&chain.input.id()],
                    image.id(),
                    chain,
                    &self.profiler,
                )
                .map_err(|error| GpuError(error.into()));
            // Earlier recursive uploads and procedural fills may already be
            // cached. Keep their commands even if this chain rejects its budget.
            self.procedural_encoder = Some(encoder);
            result?
        } else if let Some(program) = image.procedural() {
            let limits = self.device.limits();
            if program.parameters.is_empty()
                || program.parameters.len() as u64 * 4 > limits.max_storage_buffer_binding_size
                || program
                    .dispatch
                    .iter()
                    .any(|n| *n == 0 || *n > limits.max_compute_workgroups_per_dimension)
            {
                return Err(GpuError(
                    "procedural parameters or dispatch exceed GPU limits".into(),
                ));
            }
            let mut encoder = self
                .procedural_encoder
                .take()
                .unwrap_or_else(|| self.device.create_command_encoder(&Default::default()));
            let result = self.procedural.render(
                &self.device,
                &self.queue,
                &mut encoder,
                &self.context.inner.texture_layout,
                image,
                self.image_budget,
                &self.profiler,
            );
            self.procedural_encoder = Some(encoder);
            let (texture, bind) = result.map_err(|error| GpuError(error.into()))?;
            display = Some(bind);
            texture
        } else {
            let texture =
                texture_levels(&self.device, image.width(), image.height(), "image", levels);
            let mut pixels = image.pixels().to_vec();
            for pixel in pixels.as_chunks_mut::<4>().0 {
                let alpha = u16::from(pixel[3]);
                for channel in &mut pixel[..3] {
                    *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
                }
            }
            let (mut width, mut height) = (image.width(), image.height());
            for level in 0..levels {
                let next = (level + 1 < levels).then(|| image_mips::halve(&pixels, width, height));
                if self.mapped.is_some() {
                    self.texture_uploads
                        .push_level(&texture, level, (0, 0, width, height), &pixels);
                } else {
                    // Direct writes use BGRA; mapped uploads convert RGBA in a render pass.
                    for pixel in pixels.as_chunks_mut::<4>().0 {
                        pixel.swap(0, 2);
                    }
                    self.queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            mip_level: level,
                            ..texture.as_image_copy()
                        },
                        &pixels,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(width * 4),
                            rows_per_image: None,
                        },
                        wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                    );
                }
                if let Some(next) = next {
                    pixels = next;
                    width = (width / 2).max(1);
                    height = (height / 2).max(1);
                }
            }
            texture
        };
        let bind = display.unwrap_or_else(|| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("image"),
                layout: &self.context.inner.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(
                            &texture.create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.device.create_sampler(
                            &wgpu::SamplerDescriptor {
                                mag_filter: wgpu::FilterMode::Linear,
                                min_filter: wgpu::FilterMode::Linear,
                                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                                ..Default::default()
                            },
                        )),
                    },
                ],
            })
        });
        self.images.insert(image.id(), (bind, bytes));
        self.image_textures.insert(image.id(), texture);
        Ok(())
    }
    fn reset_atlas(&mut self, size: u32) {
        self.atlas = texture(&self.device, size, size, "glyph atlas");
        self.atlas_size = size;
        self.bind = self.texture_bind(&self.atlas.create_view(&Default::default()), "glyph atlas");
        self.entries.clear();
        self.image_cells.clear();
        self.cursor = (0, 0, 0);
        self.atlas_epoch = self.atlas_epoch.wrapping_add(1);
    }
    fn glyph(
        &mut self,
        key: CacheKey,
        stats: &mut GpuStats,
    ) -> Result<Option<AtlasEntry>, GpuError> {
        let size = f32::from_bits(key.font_size_bits);
        if !size.is_finite() || size > MAX_ATLAS as f32 {
            return Err(GpuError(
                "glyph font size exceeds atlas raster limit".into(),
            ));
        }
        if let Some(entry) = self.entries.get(&key) {
            return Ok(*entry);
        }
        if self.entries.len() >= 8192 {
            return Err(GpuError(
                "glyph metadata exceeds 8192 entries in one frame".into(),
            ));
        }
        let Some(image) = self
            .swash
            .get_image_uncached(&mut self.fonts.borrow_mut(), key)
        else {
            self.entries.insert(key, None);
            return Ok(None);
        };
        let p = image.placement;
        if p.width == 0 || p.height == 0 {
            self.entries.insert(key, None);
            return Ok(None);
        }
        let cell_width = p.width + 2;
        let (x, y) = self.atlas_cell(p.width, p.height, |rgba| {
            let row_start = |row: u32| ((row + 1) * cell_width + 1) as usize * 4;
            match image.content {
                SwashContent::Mask => {
                    for (row, alphas) in image.data.chunks_exact(p.width as usize).enumerate() {
                        let start = row_start(row as u32);
                        for (texel, alpha) in
                            rgba[start..].as_chunks_mut::<4>().0.iter_mut().zip(alphas)
                        {
                            texel.copy_from_slice(&[255, 255, 255, *alpha]);
                        }
                    }
                }
                SwashContent::Color => {
                    let bytes = p.width as usize * 4;
                    for (row, texels) in image.data.chunks_exact(bytes).enumerate() {
                        let start = row_start(row as u32);
                        rgba[start..start + bytes].copy_from_slice(texels);
                    }
                }
                SwashContent::SubpixelMask => {
                    let bytes = p.width as usize * 4;
                    for (row, pixels) in image.data.chunks_exact(bytes).enumerate() {
                        let start = row_start(row as u32);
                        for (texel, pixel) in rgba[start..]
                            .as_chunks_mut::<4>()
                            .0
                            .iter_mut()
                            .zip(pixels.as_chunks::<4>().0)
                        {
                            texel.copy_from_slice(&[
                                255,
                                255,
                                255,
                                pixel[0].max(pixel[1]).max(pixel[2]),
                            ]);
                        }
                    }
                }
            }
        })?;
        let entry = AtlasEntry {
            x,
            y,
            width: p.width,
            height: p.height,
            left: p.left,
            top: p.top,
        };
        self.entries.insert(key, Some(entry));
        stats.glyph_uploads += 1;
        Ok(Some(entry))
    }
    /// Reserve a `width`×`height` atlas cell with a transparent 1-texel border
    /// and upload it; `fill` writes the bordered RGBA cell. Returns the
    /// interior's origin. Linear sampling at fractional positions reads one
    /// texel past the content, and a recycled atlas is not cleared, so shared
    /// gutters could otherwise hold stale texels.
    fn atlas_cell(
        &mut self,
        width: u32,
        height: u32,
        fill: impl FnOnce(&mut [u8]),
    ) -> Result<(u32, u32), GpuError> {
        if self.atlas_size == 1 {
            self.reset_atlas(INITIAL_ATLAS);
        }
        let (cell_width, cell_height) = (width + 2, height + 2);
        if self.cursor.0 + cell_width > self.atlas_size {
            self.cursor.0 = 0;
            self.cursor.1 += self.cursor.2;
            self.cursor.2 = 0;
        }
        if self.cursor.1 + cell_height > self.atlas_size || cell_width > self.atlas_size {
            return Err(GpuError(
                "glyph atlas capacity exceeded in one frame".into(),
            ));
        }
        let mut rgba = vec![0; (cell_width * cell_height * 4) as usize];
        fill(&mut rgba);
        if self.mapped.is_some() {
            self.texture_uploads.push(
                &self.atlas,
                (self.cursor.0, self.cursor.1, cell_width, cell_height),
                &rgba,
            );
        } else {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: self.cursor.0,
                        y: self.cursor.1,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(cell_width * 4),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: cell_width,
                    height: cell_height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let origin = (self.cursor.0 + 1, self.cursor.1 + 1);
        self.cursor.0 += cell_width;
        self.cursor.2 = self.cursor.2.max(cell_height);
        Ok(origin)
    }
    /// Present the retained target. Use `present_with_status` when scheduling retries.
    pub fn present(&mut self) -> Result<(), GpuError> {
        self.present_with_status().map(|_| ())
    }
    /// Present without discarding whether native acquisition skipped this frame.
    pub fn present_with_status(&mut self) -> Result<PresentationStatus, GpuError> {
        self.present_with_notify(|| {})
    }
    /// Notify the native host immediately before committing an acquired frame.
    /// Skipped acquisition never invokes the callback, so Wayland hosts do not
    /// wait for a frame callback on a surface commit that never happened.
    pub fn present_with_notify(
        &mut self,
        before_present: impl FnOnce(),
    ) -> Result<PresentationStatus, GpuError> {
        if self.surface.is_none() {
            return Ok(PresentationStatus::Offscreen);
        }
        let frame = match surface::acquire(self)? {
            surface::Acquired::Frame(frame) => frame,
            surface::Acquired::Skipped(status) => {
                // The retained target must still be complete for the retry.
                self.flush_pending();
                return Ok(status);
            }
        };
        let mut encoder = self
            .pending
            .get_mut()
            .take()
            .unwrap_or_else(|| self.device.create_command_encoder(&Default::default()));
        if let Some(deferred) = self.deferred.get_mut().take() {
            let format = self
                .single_pass_format
                .expect("deferred only with a view format");
            let raw = frame.texture.create_view(&wgpu::TextureViewDescriptor {
                format: Some(format),
                ..Default::default()
            });
            self.present_single_pass(&mut encoder, &deferred, &raw, format);
        } else if self.copy_present {
            let stamp = self.profile_start(&mut encoder, "presentation_copy");
            encoder.copy_texture_to_texture(
                self.target.as_image_copy(),
                frame.texture.as_image_copy(),
                self.target.size(),
            );
            profiling::Profiler::end(&mut encoder, stamp);
        } else {
            let view = frame.texture.create_view(&Default::default());
            let stamp = self.profile_pass("presentation_blit");
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                timestamp_writes: stamp.as_ref().map(|s| s.writes()),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(self.blit.as_ref().unwrap());
            pass.set_bind_group(0, self.blit_bind.as_ref().unwrap(), &[]);
            pass.draw(0..3, 0..1);
        }
        self.submit_profiled(encoder, true);
        self.belt.get_mut().recall();
        self.submitted();
        if let Some(mapped) = self.mapped.as_mut() {
            mapped.get_mut().submitted();
        }
        before_present();
        frame.present();
        // Transactional presents block on scheduling; keep them to resizes.
        #[cfg(target_os = "macos")]
        self.set_transaction_present(false);
        #[cfg(target_os = "macos")]
        if self
            .wide_until
            .is_some_and(|until| std::time::Instant::now() >= until)
        {
            self.narrow_drawables();
            // Free the third drawable now rather than when the pool turns over.
            self.release_drawables();
        }
        Ok(PresentationStatus::Presented)
    }
    /// Release memory that only animation needs: filter scratch textures,
    /// idle vertex buffers and, on macOS, Core Animation's pool of window-sized
    /// drawables (two of three, ~13 MB each at 5K). The window keeps showing
    /// its last frame; the next frame recreates whatever it uses. Call when a
    /// window has stopped presenting for a while.
    pub fn trim(&mut self) {
        self.flush_pending();
        self.blur_cache.clear();
        self.gaussian_scratch = None;
        if let Some(kawase) = &mut self.kawase {
            kawase.scratch = None;
        }
        self.spare = None;
        self.scroll_history = None;
        self.scroll_state = None;
        if let Some(mapped) = &mut self.mapped {
            mapped.get_mut().trim();
        }
        // Word shapes kept for the next frame, and the table they peaked in
        // (streaming fills it): the next frame shapes what it needs again.
        self.fonts.borrow_mut().shape_run_cache = Default::default();
        #[cfg(target_os = "macos")]
        {
            self.narrow_drawables();
            self.release_drawables();
        }
    }
    #[cfg(target_os = "macos")]
    fn release_drawables(&mut self) {
        let Some(surface) = &self.surface else {
            return;
        };
        // SAFETY: The HAL guard only borrows this live surface for the call.
        let Some(hal) = (unsafe { surface.as_hal::<wgpu::hal::api::Metal>() }) else {
            return;
        };
        let layer = hal.render_layer().lock();
        // Changing drawableSize discards pooled drawables; restoring it at once
        // leaves the configuration and the displayed contents untouched.
        // SAFETY: `drawableSize` is a CGSize property of CAMetalLayer.
        unsafe {
            let size: objc2_core_foundation::CGSize = objc2::msg_send![&**layer, drawableSize];
            let nudged = objc2_core_foundation::CGSize::new(size.width, size.height + 1.);
            let _: () = objc2::msg_send![&**layer, setDrawableSize: nudged];
            let _: () = objc2::msg_send![&**layer, setDrawableSize: size];
        }
    }
    /// A resized layer holds its transactional drawable until the window
    /// server shows the new frame size, up to a second later. With two
    /// drawables the next frame waits that long for one (every step of a
    /// drag resize would stall): keep a third until resizing settles.
    #[cfg(target_os = "macos")]
    fn widen_drawables(&mut self) {
        // Each resize configures the surface, which restores two.
        self.set_drawable_count(3);
        self.wide_until = Some(std::time::Instant::now() + WIDE_DRAWABLES);
    }
    #[cfg(target_os = "macos")]
    fn narrow_drawables(&mut self) {
        if self.wide_until.take().is_some() {
            self.set_drawable_count(2);
        }
    }
    #[cfg(target_os = "macos")]
    fn set_drawable_count(&self, count: usize) {
        let Some(surface) = &self.surface else {
            return;
        };
        // SAFETY: The HAL guard only borrows this live surface for the call.
        let Some(hal) = (unsafe { surface.as_hal::<wgpu::hal::api::Metal>() }) else {
            return;
        };
        let layer = hal.render_layer().lock();
        // SAFETY: `maximumDrawableCount` is an NSUInteger property of CAMetalLayer.
        let _: () = unsafe { objc2::msg_send![&**layer, setMaximumDrawableCount: count] };
    }
    #[cfg(target_os = "macos")]
    fn set_transaction_present(&mut self, enabled: bool) {
        if self.transaction_present == enabled {
            return;
        }
        let Some(surface) = &self.surface else {
            return;
        };
        // SAFETY: The HAL guard only borrows this live surface for the call.
        let Some(hal) = (unsafe { surface.as_hal::<wgpu::hal::api::Metal>() }) else {
            return;
        };
        let layer = hal.render_layer().lock();
        // SAFETY: `presentsWithTransaction` is a BOOL property of CAMetalLayer.
        let _: () = unsafe { objc2::msg_send![&**layer, setPresentsWithTransaction: enabled] };
        self.transaction_present = enabled;
    }
    /// Frames whose damage was drawn in the same pass as presentation.
    pub fn debug_single_pass_frames(&self) -> u64 {
        self.single_pass_frames
    }
    /// Test hook: specialize quads by their features (see `push_draw`), or use
    /// the original Gaussian-capable full shader for an exact pixel A/B.
    pub fn debug_split_shading(&mut self, split: bool) {
        self.split_shading = split;
    }
    /// Compare geometry lookup cost separately from geometry-only draw merging.
    /// Flush pending presentation before switching either transport or batching.
    pub fn debug_geometry_batching(&mut self, mode: GeometryBatching) {
        if self.geometry_batching != mode {
            self.flush_pending();
            self.geometry_batching = mode;
            self.single_pass = None;
            self.fresh = true;
        }
    }
    fn quad_pipelines(&self) -> &QuadPipelines {
        if self.geometry_batching == GeometryBatching::DynamicUniform {
            &self.context.inner.quads
        } else {
            &self.context.indexed_geometry().quads
        }
    }
    fn quad_geometry(&self) -> &wgpu::BindGroup {
        if self.geometry_batching == GeometryBatching::DynamicUniform {
            &self.geometry.bind
        } else {
            self.geometry
                .indexed_bind
                .as_ref()
                .expect("indexed geometry uploaded")
        }
    }
    /// Compare full shader specialization with identical batches and Basic/opaque paths.
    pub fn debug_regular_full_shader(&mut self, enabled: bool) {
        self.regular_full_shader = enabled;
    }
    /// Compare nearest-image specialization with RegularFull using identical batches.
    /// Disabling regular-full specialization also disables this specialization.
    pub fn debug_nearest_image_shader(&mut self, enabled: bool) {
        self.nearest_image_shader = enabled;
    }
    /// Opt in to exact quad clip-interior shortcuts with identical batches.
    /// Blur-composite coverage retains the reference algorithm.
    pub fn debug_clip_interiors(&mut self, enabled: bool) {
        self.clip_interiors = enabled;
    }
    /// Ablate opaque-fill decomposition while retaining the same shader selection.
    pub fn debug_opaque_interiors(&mut self, enabled: bool) {
        self.opaque_interiors = enabled;
    }
    /// Test hook: treat an offscreen `Bgra8Unorm` texture as the drawable, so
    /// single-pass presentation runs without a window.
    pub fn debug_enable_single_pass(&mut self) {
        self.single_pass_format = Some(wgpu::TextureFormat::Bgra8Unorm);
        self.present_unpremultiplies = !cfg!(target_os = "macos");
    }
    /// Test hook: present into a fresh offscreen drawable configured like a
    /// window (`Bgra8UnormSrgb`, non-opaque) and return its
    /// bytes in RGBA order. Single-pass renderers draw the pending frame
    /// through a non-sRGB view; others use the present blit, as on screen.
    pub fn debug_present_offscreen(&mut self) -> Result<Vec<u8>, GpuError> {
        let srgb = wgpu::TextureFormat::Bgra8UnormSrgb;
        let drawable = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen drawable"),
            size: wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: srgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[wgpu::TextureFormat::Bgra8Unorm],
        });
        let mut encoder = self
            .pending
            .get_mut()
            .take()
            .unwrap_or_else(|| self.device.create_command_encoder(&Default::default()));
        if let Some(frame) = self.deferred.get_mut().take() {
            let raw = drawable.create_view(&wgpu::TextureViewDescriptor {
                format: Some(wgpu::TextureFormat::Bgra8Unorm),
                ..Default::default()
            });
            self.present_single_pass(&mut encoder, &frame, &raw, wgpu::TextureFormat::Bgra8Unorm);
        } else {
            let pipeline = blit_pipeline(&self.device, srgb, !cfg!(target_os = "macos"));
            let bind = blit_group(&self.device, &pipeline, &self.view);
            let view = drawable.create_view(&Default::default());
            let stamp = self.profile_pass("presentation_blit");
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                timestamp_writes: stamp.as_ref().map(|s| s.writes()),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.submit_profiled(encoder, true);
        self.submitted();
        if let Some(mapped) = self.mapped.as_mut() {
            mapped.get_mut().submitted();
        }
        let mut pixels = self.read_texture(&drawable)?;
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        Ok(pixels)
    }
    pub fn readback(&self) -> Result<Vec<u8>, GpuError> {
        self.flush_pending_inner(true);
        self.read_texture(&self.target)
    }
    /// Finish submitted rendering without a texture readback. For benchmarks
    /// and diagnostics only: normal frames must stay asynchronous.
    pub fn wait_idle(&self) -> Result<(), GpuError> {
        self.flush_pending_inner(true);
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError(e.to_string()))?;
        Ok(())
    }
    /// Submit an offscreen frame asynchronously; unlike wait_idle this never waits.
    pub fn submit(&self) {
        self.flush_pending_inner(true);
    }
    /// Enable bounded GPU pass timestamps. Returns false on unsupported adapters.
    /// No device waits or readbacks are added when profiling is disabled.
    /// Disable presentation vsync for developer throughput measurements. The OS compositor
    /// can still limit delivered frames. Restore AutoVsync when focus is lost.
    pub fn set_vsync(&mut self, enabled: bool) {
        let mode = if enabled { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
        if let (Some(surface), Some(config)) = (&self.surface, &mut self.config) {
            if config.present_mode != mode {
                config.present_mode = mode;
                surface.configure(&self.device, config);
            }
        }
    }
    pub fn set_gpu_profiling(&mut self, enabled: bool) -> bool {
        self.flush_pending_inner(true);
        *self.profiler.get_mut() = if enabled {
            profiling::Profiler::new(&self.device, &self.queue)
        } else {
            None
        };
        self.profiler.borrow().is_some()
    }
    /// Diagnose the next nonempty offscreen root redraw, once. Requires GPU profiling.
    /// The first 48 batches starting at `first_batch` and up to 16 other large
    /// image batches get individual pass slices. Other batches stay aggregated.
    /// Pass partitioning perturbs GPU cost; disable this for throughput acceptance.
    pub fn arm_gpu_draw_diagnostics(&mut self, first_batch: u32) -> bool {
        if self.surface.is_some() { return false; }
        self.flush_pending_inner(true);
        self.profiler.get_mut().as_mut().is_some_and(|p| p.arm_draw_diagnostics(first_batch))
    }
    /// Poll completed timestamps without waiting for GPU work.
    pub fn take_gpu_profiles(&self) -> Vec<profiling::GpuFrameProfile> {
        if self.profiler.borrow().is_none() {
            return Vec::new();
        }
        let _ = self.device.poll(wgpu::PollType::Poll);
        self.profiler
            .borrow_mut()
            .as_mut()
            .map_or_else(Vec::new, |p| p.take())
    }
    pub fn gpu_profile_frame_id(&self) -> Option<u64> { self.profiler.borrow().as_ref().map(|p|p.frame_id()) }
    pub fn dropped_gpu_profiles(&self) -> u64 {
        self.profiler.borrow().as_ref().map_or(0, |p| p.dropped)
    }
    fn profile_pass(&self, label: &'static str) -> Option<profiling::PassStamp> {
        self.profiler
            .borrow_mut()
            .as_mut()
            .and_then(|p| p.pass(label))
    }
    fn profile_start(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        label: &'static str,
    ) -> Option<profiling::PassStamp> {
        self.profiler
            .borrow_mut()
            .as_mut()
            .and_then(|p| p.start(encoder, label))
    }
    fn submit_profiled(&self, mut encoder: wgpu::CommandEncoder, finish: bool) {
        if finish && let Some(p) = self.profiler.borrow_mut().as_mut() {
            p.resolve(&mut encoder);
        }
        self.queue.submit([encoder.finish()]);
        if finish && let Some(p) = self.profiler.borrow_mut().as_mut() {
            p.submitted();
        }
    }
    fn read_texture(&self, texture: &wgpu::Texture) -> Result<Vec<u8>, GpuError> {
        let stride = (self.width * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(stride) * u64::from(self.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        self.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError(e.to_string()))?;
        rx.recv()
            .map_err(|e| GpuError(e.to_string()))?
            .map_err(|e| GpuError(e.to_string()))?;
        let mapped = buffer.get_mapped_range(..);
        let mut pixels = Vec::with_capacity((self.width * self.height * 4) as usize);
        for row in mapped.chunks(stride as usize) {
            pixels.extend_from_slice(&row[..self.width as usize * 4]);
        }
        // Retained textures are BGRA (see `FORMAT`); callers read RGBA.
        if texture.format() == wgpu::TextureFormat::Bgra8Unorm {
            for pixel in pixels.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
        }
        drop(mapped);
        buffer.unmap();
        Ok(pixels)
    }
}
fn texture(device: &wgpu::Device, width: u32, height: u32, label: &str) -> wgpu::Texture {
    texture_levels(device, width, height, label, 1)
}
fn texture_levels(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    label: &str,
    levels: u32,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}
fn rich_decoration_quad(d: &text::RichDecoration) -> Quad {
    Quad {
        rect: [d.bounds.x, d.bounds.y, d.bounds.width, d.bounds.height],
        color: rgba(d.color, 1.),
        uv: [0.; 4],
        fade: [0.; 4],
        options: [0.; 4],
        shape: [0.; 4],
        border: [0.; 4],
        mask: NO_MASK,
    }
}
fn rgba(c: Color, opacity: f32) -> [f32; 4] {
    [
        c.0 as f32 / 255.,
        c.1 as f32 / 255.,
        c.2 as f32 / 255.,
        c.3 as f32 / 255. * opacity,
    ]
}
/// Quads into the retained target, or with `drawable` also into a second,
/// presentation target in the same pass (`fs_both`).
fn quad_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    blend: bool,
    drawable: Option<wgpu::TextureFormat>,
    shading: Shading,
    clip_interiors: bool,
) -> wgpu::RenderPipeline {
    let basic = matches!(shading, Shading::Basic | Shading::Opaque);
    let shader_constants = [
        ("gaussian_shadows", if shading == Shading::Full { 1. } else { 0. }),
        ("nearest_images", if shading == Shading::NearestImage { 1. } else { 0. }),
        ("clip_interiors", if clip_interiors { 1. } else { 0. }),
    ];
    let blend = blend.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
    let all = wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4,2=>Float32x4,3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4];
    let basic_attributes = [all[0], all[1], all[2], all[3], all[4], all[5]];
    let target = |format| {
        Some(wgpu::ColorTargetState {
            format,
            blend,
            write_mask: wgpu::ColorWrites::ALL,
        })
    };
    let targets = [target(FORMAT), drawable.and_then(target)];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("quads"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(if basic { "vs_basic" } else { "vs" }),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: 128,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: if basic { &basic_attributes } else { &all },
            }],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(match (shading, drawable.is_some()) {
                (Shading::Basic | Shading::Opaque, true) => "fs_basic_both",
                (Shading::Basic | Shading::Opaque, false) => "fs_basic",
                (Shading::RegularFull | Shading::NearestImage, true) => "fs_regular_both",
                (Shading::RegularFull | Shading::NearestImage, false) => "fs_regular",
                (Shading::Full, true) => "fs_both",
                (Shading::Full, false) => "fs",
            }),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: if basic { &[] } else { &shader_constants },
                ..Default::default()
            },
            targets: if drawable.is_some() {
                &targets
            } else {
                &targets[..1]
            },
        }),
        multiview_mask: None,
        cache: None,
    })
}
/// Whether presentation must convert premultiplied pixels to straight alpha.
/// wgpu reports Metal layers as "post-multiplied" merely because they are not
/// opaque, but Core Animation composites `CAMetalLayer` contents as
/// premultiplied (measured: straight 50% red over black shows twice as bright).
fn unpremultiplies(config: &wgpu::SurfaceConfiguration) -> bool {
    config.alpha_mode == wgpu::CompositeAlphaMode::PostMultiplied && !cfg!(target_os = "macos")
}
fn blit_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    post_alpha: bool,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("present"),
        source: wgpu::ShaderSource::Wgsl(
            (format!(
                "const OUTPUT_SRGB:bool={};\nconst POST_ALPHA:bool={};\n{}",
                format.is_srgb(),
                post_alpha,
                include_str!("blit.wgsl")
            ))
            .into(),
        ),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("present"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
fn blit_group(
    device: &wgpu::Device,
    pipeline: &wgpu::RenderPipeline,
    view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("present"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(view),
        }],
    })
}
/// `r` grown to whole physical pixels, as `scissor` rounds it.
fn snap_out(r: Rect, scale: f32) -> Rect {
    let x = (r.x * scale).floor() / scale;
    let y = (r.y * scale).floor() / scale;
    let right = ((r.x + r.width) * scale).ceil() / scale;
    let bottom = ((r.y + r.height) * scale).ceil() / scale;
    Rect::new(x, y, right - x, bottom - y)
}
/// Whether `quad` lies entirely within `clip`.
fn inside(quad: &Quad, clip: Rect) -> bool {
    let [x, y, w, h] = quad.rect;
    x >= clip.x && y >= clip.y && x + w <= clip.x + clip.width && y + h <= clip.y + clip.height
}
/// Whether `clip` would cut `quad` rather than keep or cull it whole.
fn crosses(quad: &Quad, clip: Rect) -> bool {
    let [x, y, w, h] = quad.rect;
    let outside =
        x + w <= clip.x || y + h <= clip.y || x >= clip.x + clip.width || y >= clip.y + clip.height;
    !outside && !inside(quad, clip)
}
fn scissor(r: Rect, scale: f32, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let x = (r.x * scale).floor().max(0.) as u32;
    let y = (r.y * scale).floor().max(0.) as u32;
    let right = ((r.x + r.width) * scale).ceil().min(width as f32) as u32;
    let bottom = ((r.y + r.height) * scale).ceil().min(height as f32) as u32;
    (right > x && bottom > y).then(|| (x, y, right - x, bottom - y))
}
// Each activated filter contributes its complete clipped output and sampling
// halo. Bounding merges may touch previously unrelated filters; rescan until
// all such dependencies are included. Each filter activates at most once.
fn blur_damage(damage: &[Rect], filters: &[(Rect, f32)], viewport: Rect, scale: f32) -> Vec<Rect> {
    let mut regions = merge_damage(damage, viewport, scale);
    let mut dependencies: Vec<_> = filters
        .iter()
        .filter_map(|(output, _radius)| {
            let width = (viewport.width * scale).round() as u32;
            let height = (viewport.height * scale).round() as u32;
            let output = output.intersection(viewport)?;
            // A filter samples only its own visible pixels, so damage
            // anywhere in them re-blurs all of them, and nothing beyond.
            let (x, y, w, h) = scissor(output, scale, width, height)?;
            let (left, top, right, bottom) = (x, y, x + w, y + h);
            let dependency = Rect::new(
                left as f32 / scale,
                top as f32 / scale,
                (right - left) as f32 / scale,
                (bottom - top) as f32 / scale,
            );
            Some((dependency, false))
        })
        .collect();
    loop {
        let mut changed = false;
        for (dependency, processed) in &mut dependencies {
            if !*processed && regions.iter().any(|region| region.intersects(*dependency)) {
                regions.push(*dependency);
                regions = merge_damage(&regions, viewport, scale);
                *processed = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    regions
}
fn rounded_damage(rect: Rect, viewport: Rect, scale: f32) -> Option<Rect> {
    let rect = rect.intersection(viewport)?;
    let x = (rect.x * scale).floor() / scale;
    let y = (rect.y * scale).floor() / scale;
    Some(Rect::new(
        x,
        y,
        ((rect.x + rect.width) * scale).ceil() / scale - x,
        ((rect.y + rect.height) * scale).ceil() / scale - y,
    ))
}
fn merge_damage(damage: &[Rect], viewport: Rect, scale: f32) -> Vec<Rect> {
    // Each region redraws every batch it touches: nearby small ones (the
    // dots of a spinner) are fewer draws as one, for a few more pixels.
    let mut regions: Vec<Rect> = Vec::new();
    for r in damage {
        let Some(r) = rounded_damage(*r, viewport, scale) else {
            continue;
        };
        lumine::scene::merge_damage(&mut regions, r, Some(NEAR_PIXELS / (scale * scale)));
    }
    regions
}

pub mod assets;

pub mod prepared;
pub mod text;

fn create_quad_pipelines(
    device: &wgpu::Device,
    indexed: bool,
) -> (
    wgpu::BindGroupLayout,
    wgpu::BindGroupLayout,
    wgpu::BindGroupLayout,
    wgpu::ShaderModule,
    wgpu::PipelineLayout,
) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("quads"),
        source: wgpu::ShaderSource::Wgsl(paint_geometry::shader(indexed).into()),
    });
    let group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("quads"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let viewport_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("viewport"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let geometry_layout = paint_geometry::layout(device, indexed);
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("quads"),
        bind_group_layouts: &[
            Some(&group_layout),
            Some(&viewport_layout),
            Some(&geometry_layout),
        ],
        immediate_size: 0,
    });
    (
        group_layout,
        viewport_layout,
        geometry_layout,
        shader,
        layout,
    )
}

impl surface::SurfaceSource for GpuRenderer {
    type Frame = wgpu::SurfaceTexture;
    fn acquire(&mut self) -> Result<surface::Acquisition<Self::Frame>, GpuError> {
        use surface::Acquisition;
        Ok(match self.surface.as_ref().unwrap().get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Acquisition::Frame(frame),
            wgpu::CurrentSurfaceTexture::Timeout => Acquisition::Timeout,
            wgpu::CurrentSurfaceTexture::Occluded => Acquisition::Occluded,
            wgpu::CurrentSurfaceTexture::Lost => Acquisition::Lost,
            wgpu::CurrentSurfaceTexture::Outdated => Acquisition::Outdated,
            other => return Err(GpuError(format!("surface acquisition: {other:?}"))),
        })
    }
    fn recreate(&mut self) -> Result<(), GpuError> {
        self.surface = Some(
            self.instance
                .create_surface(self.window.as_ref().unwrap().clone())
                .map_err(|e| GpuError(e.to_string()))?,
        );
        #[cfg(target_os = "macos")]
        {
            self.transaction_present = false;
        }
        Ok(())
    }
    fn configure(&mut self) {
        self.surface
            .as_ref()
            .unwrap()
            .configure(&self.device, self.config.as_ref().unwrap());
        // Configuring restores the default count.
        #[cfg(target_os = "macos")]
        if self.wide_until.is_some() {
            self.set_drawable_count(3);
        }
    }
}

#[cfg(test)]
mod blur_damage_tests {
    use super::*;

    /// A filter samples only its own pixels: damage beside it stays local,
    /// damage inside it re-blurs the whole filter and nothing more.
    #[test]
    fn damage_outside_a_filter_is_local_and_inside_rebuilds_the_filter() {
        let viewport = Rect::new(0., 0., 500., 300.);
        let filter = (Rect::new(100., 100., 40., 30.), 2.);
        let distant = Rect::new(10., 10., 5., 5.);
        assert_eq!(
            blur_damage(&[distant], &[filter], viewport, 1.),
            vec![distant]
        );
        let beside = Rect::new(95., 110., 1., 1.);
        assert_eq!(
            blur_damage(&[beside], &[filter], viewport, 1.),
            vec![beside]
        );
        assert_eq!(
            blur_damage(&[Rect::new(101., 110., 1., 1.)], &[filter], viewport, 1.),
            vec![Rect::new(100., 100., 40., 30.)]
        );
        assert!(blur_damage(&[], &[filter], viewport, 1.).is_empty());
    }

    #[test]
    fn chained_filters_expand_to_fixed_point_independent_of_order() {
        let viewport = Rect::new(0., 0., 500., 300.);
        // Overlapping filters: rebuilding one damages the other.
        let filters = [
            (Rect::new(135., 100., 20., 20.), 2.),
            (Rect::new(120., 100., 20., 20.), 2.),
        ];
        let damage = [Rect::new(121., 105., 1., 1.)];
        let expected = vec![Rect::new(120., 100., 35., 20.)];
        assert_eq!(blur_damage(&damage, &filters, viewport, 1.), expected);
        assert_eq!(
            blur_damage(&damage, &[filters[1], filters[0]], viewport, 1.),
            expected
        );
    }

    #[test]
    fn reconstruction_covers_the_filter_pixels_across_fractional_scales() {
        for scale in [0.7, 1., 1.25, 1.3, 1.5, 1.7, 2.3] {
            let viewport = Rect::new(0., 0., 701. / scale, 503. / scale);
            for index in 0..127 {
                let output = Rect::new(
                    (index * 37 % 680) as f32 / scale + 0.017,
                    (index * 19 % 480) as f32 / scale + 0.031,
                    7.71,
                    8.13,
                );
                for radius in [0.0001, 0.33334, 1.17, 11.31, 1000.] {
                    let (x, y, w, h) = scissor(output, scale, 701, 503).unwrap();
                    let regions = blur_damage(&[output], &[(output, radius)], viewport, scale);
                    assert_eq!(regions.len(), 1);
                    let (dx, dy, dw, dh) = scissor(regions[0], scale, 701, 503).unwrap();
                    assert!(dx <= x && dy <= y);
                    assert!(dx + dw >= x + w && dy + dh >= y + h);
                }
            }
        }
    }

    #[test]
    fn fractional_output_rounds_to_whole_pixels_whatever_the_radius() {
        let viewport = Rect::new(0., 0., 400., 300.);
        // At 1.25x the output spans physical [12, 26).
        let regions = blur_damage(
            &[Rect::new(10.2, 10.2, 1., 1.)],
            &[(Rect::new(10.1, 10.1, 10.1, 10.1), 0.25)],
            viewport,
            1.25,
        );
        let (x, y, width, height) = scissor(regions[0], 1.25, 500, 375).unwrap();
        assert!(x <= 12 && y <= 12 && x + width >= 26 && y + height >= 26);
        assert!(width < 20 && height < 20);
        let huge = blur_damage(
            &[Rect::new(0., 0., 1., 1.)],
            &[(Rect::new(0., 0., 1., 1.), 10000.)],
            viewport,
            1.,
        );
        assert_eq!(huge, vec![Rect::new(0., 0., 1., 1.)]);
    }
}

pub mod canvas;

pub mod decoration;

pub mod svg;

#[cfg(test)]
mod native_surface_shader_tests {
    #[test]
    fn resizing_solid_decorations_reuses_corners_and_edges() {
        use super::*;
        use lumine::scene::{Layout, QuadStyle, Style};
        let mut gpu = GpuRenderer::new(240, 160).unwrap();
        for scale in [1., 1.5, 2.] {
            gpu.set_scale_factor(scale);
            let mut scene = Scene::new(240. / scale, 160. / scale);
            scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
            let paint = QuadStyle {
                fill: Color(30, 40, 50, 255),
                border_color: Color(220, 210, 200, 255),
                decoration: Some(Arc::new(lumine::decoration::Decoration {
                    corners: Some(lumine::decoration::Corners {
                        top_left: 8.,
                        top_right: 4.,
                        bottom_right: 6.,
                        bottom_left: 2.,
                    }),
                    border_widths: Some(lumine::scene::Insets {
                        top: 1.,
                        right: 2.,
                        bottom: 3.,
                        left: 1.,
                    }),
                    ..Default::default()
                })),
                ..Default::default()
            };
            let node = scene.append(
                scene.root(),
                NodeKind::Quad(paint),
                Style {
                    absolute: true,
                    width: Some(40.),
                    height: Some(40.),
                    ..Default::default()
                },
            );
            for frame in 0..20 {
                scene.set_style(
                    node,
                    Style {
                        absolute: true,
                        width: Some(40. + frame as f32),
                        height: Some(40. + frame as f32),
                        ..Default::default()
                    },
                );
                let damage = scene.flush().damage;
                let stats = gpu.render(&scene, &damage).unwrap();
                if frame > 0 {
                    assert_eq!(stats.canvas_rasterizations, 0);
                    assert_eq!(stats.image_uploads, 0);
                }
                let pixels = gpu.readback().unwrap();
                let at = |x: usize, y: usize| &pixels[(y * 240 + x) * 4..(y * 240 + x) * 4 + 3];
                assert_eq!(at(20, 20), [30, 40, 50]);
                assert_eq!(at(15, 0), [220, 210, 200]);
                assert_eq!(at(0, 15), [220, 210, 200]);
            }
        }
    }
    #[test]
    fn scrolling_reclaims_orphan_decoration_uploads_and_keeps_current_draws() {
        use super::*;
        use lumine::scene::{Layout, QuadStyle, Style, Transform};
        let mut gpu = GpuRenderer::new(64, 32).unwrap();
        let mut scene = Scene::new(64., 32.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let group = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(2048.),
                height: Some(3072.),
                ..Default::default()
            },
        );
        for row in 0..3 {
            for column in 0..5 {
                let color = Color(30 + column * 30, 40 + row * 30, 60, 255);
                let node = scene.append(
                    group,
                    NodeKind::Quad(QuadStyle {
                        fill: color,
                        border_color: Color(255, 0, 0, 255),
                        border_width: 1.,
                        decoration: Some(Arc::new(lumine::decoration::Decoration::default())),
                        ..Default::default()
                    }),
                    Style {
                        absolute: true,
                        width: Some(2048.),
                        height: Some(1024.),
                        ..Default::default()
                    },
                );
                scene.set_transform(
                    node,
                    Transform {
                        x: column as f32 * 8.,
                        y: row as f32 * 1024.,
                    },
                );
            }
        }
        scene.flush();
        let revision = scene.content_revision();
        let mut first_uploads = Vec::new();
        for row in 0..3 {
            scene.set_transform(
                group,
                Transform {
                    x: 0.,
                    y: -(row as f32) * 1024.,
                },
            );
            let damage = scene.flush().damage;
            assert_eq!(
                scene.content_revision(),
                revision,
                "scrolling keeps content"
            );
            gpu.render(&scene, &damage).unwrap();
            assert!(gpu.debug_cache_stats().image_bytes <= 64 * 1024 * 1024);
            assert!(gpu.canvases.bytes() <= 32 * 1024 * 1024);
            let pixels = gpu.readback().unwrap();
            for column in 0..5 {
                let offset = (8 * 64 + column * 8 + 4) * 4;
                assert_eq!(
                    &pixels[offset..offset + 4],
                    &[30 + column as u8 * 30, 40 + row as u8 * 30, 60, 255],
                    "row {row}, column {column}: metadata eviction must preserve assembled draws"
                );
            }
            if row == 0 {
                first_uploads.extend(gpu.images.keys().copied());
            }
        }
        assert!(first_uploads.iter().any(|id| !gpu.images.contains_key(id)));
        // The first row is still mounted. Returning to it must rasterize/upload
        // reclaimed entries and reproduce the same pixels.
        scene.set_transform(group, Transform::default());
        let damage = scene.flush().damage;
        let stats = gpu.render(&scene, &damage).unwrap();
        assert!(stats.canvas_rasterizations > 0 && stats.image_uploads > 0);
        assert_eq!(
            &gpu.readback().unwrap()[(8 * 64 + 4) * 4..][..4],
            &[30, 40, 60, 255]
        );
    }

    #[test]
    fn isolated_layer_budget_checks_rounded_texture_allocation_before_creation() {
        use super::*;
        let mut gpu = GpuRenderer::new(4, 4).unwrap();
        let mut scene = Scene::new(5000., 3300.);
        let node = scene.append(
            scene.root(),
            NodeKind::Rect(Color(20, 40, 60, 255)),
            lumine::scene::Style {
                width: Some(5000.),
                height: Some(3300.),
                ..Default::default()
            },
        );
        scene.set_isolated(node, true);
        scene.flush();
        // Requested pixels fit64MiB, but the rounded5056x3328 allocation does not.
        let requested = scene.layer_bounds(node).expand(1.);
        assert!(requested.width * requested.height * 4. < (64 * 1024 * 1024) as f32);
        let mut stats = GpuStats::default();
        assert!(
            gpu.prepare_layer(&scene, node, &mut stats)
                .unwrap_err()
                .0
                .contains("64 MiB")
        );
        assert!(gpu.layers.is_empty());
        assert_eq!(stats.layer_texture_allocations, 0);
    }

    #[test]
    fn flat_decoration_releases_its_previous_raster_resources() {
        use super::*;
        let mut gpu = GpuRenderer::new(16, 16).unwrap();
        let mut scene = Scene::new(16., 16.);
        let mut style = lumine::scene::QuadStyle {
            fill: Color(20, 40, 60, 255),
            border_color: Color(200, 100, 50, 255),
            border_width: 2.,
            decoration: Some(Arc::new(lumine::decoration::Decoration::default())),
            ..Default::default()
        };
        let node = scene.append(
            scene.root(),
            NodeKind::Quad(style.clone()),
            lumine::scene::Style {
                width: Some(16.),
                height: Some(16.),
                ..Default::default()
            },
        );
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        assert_eq!(gpu.image_textures.len(), 1);
        assert!(gpu.canvases.bytes() > 0);
        let old = gpu.node_images[&node];
        gpu.images.remove(&old);
        gpu.image_textures.remove(&old);
        gpu.render(&scene, &damage).unwrap();
        assert!(gpu.image_textures.contains_key(&gpu.node_images[&node]));
        style.border_width = 0.;
        scene.set_kind(node, NodeKind::Quad(style));
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        assert!(gpu.images.is_empty());
        assert!(gpu.image_textures.is_empty());
        assert_eq!(gpu.canvases.bytes(), 0);
    }

    #[test]
    fn nv12_compute_shader_validates_on_the_available_gpu_backend() {
        let gpu = super::GpuRenderer::new(4, 4).unwrap();
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("NV12 portable validation"),
                source: wgpu::ShaderSource::Wgsl(include_str!("native_surface.wgsl").into()),
            });
        let _pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("NV12 validation"),
                layout: None,
                module: &shader,
                entry_point: Some("convert"),
                compilation_options: Default::default(),
                cache: None,
            });
    }
}

#[cfg(test)]
mod fade_tests {
    use super::*;
    #[test]
    fn horizontal_mask_uses_x_bounds_and_requires_full_shading() {
        let rect = Rect::new(17., 29., 80., 40.);
        let mask = fade_parameters(rect, [-12., -8.]);
        assert_eq!(mask, [17., 97., -12., -8.]);
        let quad = Quad {
            mask,
            ..<Quad as bytemuck::Zeroable>::zeroed()
        };
        assert!(matches!(shading(&quad, 1.), Shading::RegularFull));
        assert_eq!(fade_parameters(rect, [12., 8.]), [29., 69., 12., 8.]);
    }
}
