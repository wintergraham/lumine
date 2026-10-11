//! Damage-scissored retained GPU renderer for Vulkan (Linux) and Metal (macOS).
//! The output is premultiplied RGBA8. Text uses cosmic-text shaping and a bounded
//! GPU glyph atlas; stable text never reshapes on paint or compositor updates.
use cosmic_text::{Buffer, CacheKey, FontSystem, Metrics, SwashCache, SwashContent};
use std::{cell::RefCell, collections::HashMap, fmt, rc::Rc, sync::Arc};
use wgpu::util::DeviceExt;
use lumine::scene::{Color, NodeId, NodeKind, Rect, Scene};

mod surface;
pub use surface::PresentationStatus;

const MAX_ATLAS: u32 = 2048;
const INITIAL_ATLAS: u32 = 512;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
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
    /// Separable filter passes (two per applied backdrop blur), included in render_passes.
    pub blur_passes: usize,
    pub instances: usize,
    pub shaped_nodes: usize,
    pub glyph_uploads: usize,
    pub image_uploads: usize,
    pub canvas_rasterizations: usize,
    pub svg_rasterizations: usize,
    pub geometry_rebuilds: usize,
    pub vertex_buffer_allocations: usize,
    pub damaged_pixels: u64,
    pub layer_repaints: usize,
    pub layer_cache_hits: usize,
    pub layer_texture_allocations: usize,
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
    pub canvas_raster_bytes: usize,
    pub svg_raster_bytes: usize,
    pub vertex_buffer_bytes: usize,
    pub layer_textures: usize,
    pub layer_bytes: usize,
}
const SHAPE_BUDGET: usize = 8 * 1024 * 1024;
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
}
#[derive(Clone)]
struct Glyph {
    key: CacheKey,
    color: Option<lumine::scene::Color>,
    x: f32,
    y: f32,
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
    glyphs: Vec<Glyph>,
    quads: Vec<Quad>,
    atlas_epoch: u64,
    last_used: u64,
}
impl Shaped {
    fn bytes(&self) -> usize {
        self.rich.as_ref().map_or(self.text.len(), |r| r.storage_bytes())
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
}
struct Draw {
    blur: Option<(Rect, lumine::scene::Effects)>,
    start: u32,
    end: u32,
    clip: Rect,
    image: Option<u64>,
    layer: Option<NodeId>,
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
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: wgpu::AdapterInfo,
    fonts: Rc<RefCell<FontSystem>>,
    pipeline: wgpu::RenderPipeline,
    clear_pipeline: wgpu::RenderPipeline,
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
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("lumine shared device"),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        }))
        .map_err(|e| GpuError(e.to_string()))?;
        let (pipeline, clear_pipeline) = create_quad_pipelines(&device);
        Ok(Self {
            inner: Rc::new(GpuContextInner {
                instance,
                adapter,
                device,
                queue,
                info,
                fonts: Rc::new(RefCell::new(FontSystem::new())),
                pipeline,
                clear_pipeline,
            }),
        })
    }
    pub fn shares_device(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
    pub fn text_system(&self) -> Rc<RefCell<FontSystem>> {
        self.inner.fonts.clone()
    }
}
impl GpuError {
    pub fn is_surface_incompatible(&self) -> bool {
        self.0 == "surface is incompatible with the shared GPU adapter"
    }
}
pub struct GpuRenderer {
    context: GpuContext,
    scene_identity: Option<u64>,
    instance: wgpu::Instance,
    window: Option<Arc<winit::window::Window>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: wgpu::AdapterInfo,
    surface: Option<wgpu::Surface<'static>>,
    config: Option<wgpu::SurfaceConfiguration>,
    width: u32,
    height: u32,
    scale: f32,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    clear_pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    blit: Option<wgpu::RenderPipeline>,
    blit_bind: Option<wgpu::BindGroup>,
    atlas: wgpu::Texture,
    atlas_size: u32,
    entries: HashMap<CacheKey, Option<AtlasEntry>>,
    cursor: (u32, u32, u32),
    fonts: Rc<RefCell<FontSystem>>,
    swash: SwashCache,
    layers: HashMap<NodeId, LayerCache>,
    layer_origin: (f32, f32),
    shapes: HashMap<NodeId, Shaped>,
    shaped_bytes: usize,
    fresh: bool,
    atlas_epoch: u64,
    frame: u64,
    vertices: wgpu::Buffer,
    vertex_capacity: usize,
    frame_quads: Vec<Quad>,
    background: Color,
    images: HashMap<u64, (wgpu::BindGroup, usize)>,
    canvases: canvas::CanvasCache,
    svgs: svg::SvgCache,
    blur_resources: Option<(wgpu::RenderPipeline, wgpu::Texture, wgpu::Texture)>,
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
            s.configure(&device, &c);
            c
        });
        let target = texture(&device, width, height, "retained output");
        let view = target.create_view(&Default::default());
        let atlas = texture(&device, 1, 1, "lazy glyph atlas placeholder");
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport"),
            contents: bytemuck::cast_slice(&[width as f32, height as f32, 0., 0.]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let pipeline = context.inner.pipeline.clone();
        let clear_pipeline = context.inner.clear_pipeline.clone();
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
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
        let blit = config.as_ref().map(|c| {
            blit_pipeline(
                &device,
                c.format,
                c.alpha_mode == wgpu::CompositeAlphaMode::PostMultiplied,
            )
        });
        let blit_bind = blit.as_ref().map(|p| blit_group(&device, p, &view));
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("retained frame vertices"),
            size: 4096,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            context: context.clone(),
            scene_identity: None,
            instance,
            window,
            device,
            queue,
            info,
            surface,
            config,
            width,
            height,
            scale: 1.,
            target,
            view,
            pipeline,
            clear_pipeline,
            uniform,
            bind,
            blit,
            blit_bind,
            atlas,
            atlas_size: 1,
            entries: HashMap::new(),
            cursor: (0, 0, 0),
            fonts: context.text_system(),
            swash: SwashCache::new(),
            layers: HashMap::new(),
            layer_origin: (0., 0.),
            shapes: HashMap::new(),
            shaped_bytes: 0,
            fresh: true,
            atlas_epoch: 1,
            frame: 0,
            vertices,
            vertex_capacity: 4096,
            frame_quads: Vec::new(),
            background: Color(0, 0, 0, 0),
            images: HashMap::new(),
            canvases: Default::default(),
            svgs: Default::default(),
            blur_resources: None,
        })
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
            canvas_raster_bytes: self.canvases.bytes(),
            svg_raster_bytes: self.svgs.bytes(),
            vertex_buffer_bytes: self.vertex_capacity,
            layer_textures: self.layers.len(),
            layer_bytes: self.layers.values().map(|l| l.bytes).sum(),
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
    pub fn set_scale_factor(&mut self, scale: f32) {
        let scale = if scale.is_finite() {
            scale.max(0.1)
        } else {
            1.
        };
        if self.scale != scale {
            self.scale = scale;
            self.layers.clear();
            self.shapes.clear();
            self.entries.clear();
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
        self.layers.clear();
        self.blur_resources = None;
        self.width = width;
        self.height = height;
        self.target = texture(&self.device, width, height, "retained output");
        self.view = self.target.create_view(&Default::default());
        if let (Some(s), Some(c)) = (&self.surface, &mut self.config) {
            c.width = width;
            c.height = height;
            s.configure(&self.device, c);
        }
        self.blit_bind = self
            .blit
            .as_ref()
            .map(|p| blit_group(&self.device, p, &self.view));
        self.fresh = true;
    }
    pub fn render(&mut self, scene: &Scene, damage: &[Rect]) -> Result<GpuStats, GpuError> {
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
        if self.scene_identity != Some(scene.identity()) {
            self.scene_identity = Some(scene.identity());
            self.layers.clear();
            self.shapes.clear();
            self.shaped_bytes = 0;
            self.fresh = true;
        }
        self.shapes.retain(|id, _| {
            scene.contains(*id) && matches!(scene.kind(*id), NodeKind::Text { .. } | NodeKind::RichText { .. })
        });
        self.shaped_bytes = self.shapes.values().map(Shaped::bytes).sum();
        self.canvases.retain(scene);
        self.svgs.retain(scene);
        if !self.images.is_empty() {
            let mut live_images: std::collections::HashSet<u64> = scene
                .paint_items()
                .filter_map(|item| {
                    if let NodeKind::Image(image) = item.kind {
                        Some(image.id())
                    } else {
                        None
                    }
                })
                .collect();
            live_images.extend(self.canvases.image_ids());
            live_images.extend(self.svgs.image_ids());
            self.images.retain(|id, _| live_images.contains(id));
        }
        self.layers
            .retain(|id, _| scene.contains(*id) && scene.is_isolated(*id));
        if damage.is_empty() && !self.fresh {
            return Ok(GpuStats::default());
        }
        let items = scene.layer_items(None);
        let mut layer_stats = GpuStats::default();
        for item in &items {
            if item.isolated && item.effects.opacity > 0. {
                self.prepare_layer(scene, item.id, &mut layer_stats)?;
            }
        }
        let mut stats = self.render_flat(scene, items, damage)?;
        stats.layer_repaints = layer_stats.layer_repaints;
        stats.layer_cache_hits = layer_stats.layer_cache_hits;
        stats.layer_texture_allocations = layer_stats.layer_texture_allocations;
        stats.damaged_pixels += layer_stats.damaged_pixels;
        stats.draw_calls += layer_stats.draw_calls;
        stats.render_passes += layer_stats.render_passes;
        stats.blur_passes += layer_stats.blur_passes;
        stats.instances += layer_stats.instances;
        stats.shaped_nodes += layer_stats.shaped_nodes;
        stats.glyph_uploads += layer_stats.glyph_uploads;
        stats.image_uploads += layer_stats.image_uploads;
        stats.canvas_rasterizations += layer_stats.canvas_rasterizations;
        stats.svg_rasterizations += layer_stats.svg_rasterizations;
        stats.geometry_rebuilds += layer_stats.geometry_rebuilds;
        stats.vertex_buffer_allocations += layer_stats.vertex_buffer_allocations;
        Ok(stats)
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
        {
            cache.bounds.x += root.x - cache.root_origin.0;
            cache.bounds.y += root.y - cache.root_origin.1;
            cache.root_origin = (root.x, root.y);
            stats.layer_cache_hits += 1;
            return Ok(());
        }
        // A transparent texel border lets linear sampling preserve fractional
        // translation coverage instead of clamping opaque edge pixels.
        let bounds = scene.layer_bounds(id).expand(1. / self.scale);
        let x = bounds.x;
        let y = bounds.y;
        let width = (bounds.width * self.scale).ceil().max(1.) as u32;
        let height = (bounds.height * self.scale).ceil().max(1.) as u32;
        let bounds = Rect::new(x, y, width as f32 / self.scale, height as f32 / self.scale);
        let items = scene.layer_items(Some(id));
        for item in &items {
            if item.isolated && item.effects.opacity > 0. {
                self.prepare_layer(scene, item.id, stats)?;
            }
        }
        let bytes = width as usize * height as usize * 4;
        let existing = self.layers.get(&id).map_or(0, |l| l.bytes);
        let used: usize = self.layers.values().map(|l| l.bytes).sum();
        if !self.layers.contains_key(&id) && self.layers.len() >= 1024 {
            return Err(GpuError("isolated layers exceed 1024 texture limit".into()));
        }
        if bytes > 64 * 1024 * 1024 || used - existing + bytes > 64 * 1024 * 1024 {
            return Err(GpuError("isolated layers exceed 64 MiB budget".into()));
        }
        if width > self.device.limits().max_texture_dimension_2d
            || height > self.device.limits().max_texture_dimension_2d
        {
            return Err(GpuError(
                "isolated layer exceeds GPU texture dimensions".into(),
            ));
        }
        let existing = self.layers.remove(&id);
        let (texture, bind) = if let Some(cache) = existing
            .filter(|cache| cache.texture.width() == width && cache.texture.height() == height)
        {
            (cache.texture, cache.bind)
        } else {
            let texture = texture(&self.device, width, height, "isolated subtree");
            let bind = self.texture_bind(
                &texture.create_view(&Default::default()),
                "isolated subtree",
            );
            stats.layer_texture_allocations += 1;
            (texture, bind)
        };
        let view = texture.create_view(&Default::default());
        let target = std::mem::replace(&mut self.target, texture);
        let old_view = std::mem::replace(&mut self.view, view);
        let old_size = (self.width, self.height);
        self.width = width;
        self.height = height;
        let background = self.background;
        self.background = Color(0, 0, 0, 0);
        let fresh = self.fresh;
        self.fresh = true;
        let blur = self.blur_resources.take();
        let local: Vec<_> = items
            .into_iter()
            .map(|mut item| {
                item.bounds.x -= bounds.x;
                item.bounds.y -= bounds.y;
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
        let result = self.render_flat(
            scene,
            local,
            &[Rect::new(0., 0., bounds.width, bounds.height)],
        );
        self.layer_origin = old_origin;
        let texture = std::mem::replace(&mut self.target, target);
        let _view = std::mem::replace(&mut self.view, old_view);
        self.width = old_size.0;
        self.height = old_size.1;
        self.background = background;
        self.fresh = fresh;
        self.blur_resources = blur;
        let result = result?;
        stats.draw_calls += result.draw_calls;
        stats.render_passes += result.render_passes;
        stats.blur_passes += result.blur_passes;
        stats.instances += result.instances;
        stats.shaped_nodes += result.shaped_nodes;
        stats.glyph_uploads += result.glyph_uploads;
        stats.image_uploads += result.image_uploads;
        stats.canvas_rasterizations += result.canvas_rasterizations;
        stats.svg_rasterizations += result.svg_rasterizations;
        stats.geometry_rebuilds += result.geometry_rebuilds;
        stats.vertex_buffer_allocations += result.vertex_buffer_allocations;
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
            },
        );
        Ok(())
    }
    fn texture_bind(&self, view: &wgpu::TextureView, label: &str) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
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
        _scene: &Scene,
        items: Vec<lumine::scene::PaintItem<'_>>,
        damage: &[Rect],
    ) -> Result<GpuStats, GpuError> {
        let mut stats = GpuStats::default();
        if damage.is_empty() && !self.fresh {
            return Ok(stats);
        }
        let viewport = Rect::new(
            0.,
            0.,
            self.width as f32 / self.scale,
            self.height as f32 / self.scale,
        );
        self.queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::cast_slice(&[viewport.width, viewport.height, 0., 0.]),
        );
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
                        .bounds
                        .intersection(item.clip.unwrap_or(viewport))?
                        .intersection(viewport)?;
                    Some((output, item.effects.blur_radius))
                })
                .collect();
            blur_damage(damage, &dependencies, viewport, self.scale)
        } else {
            merge_damage(damage, viewport, self.scale)
        };
        if damage.is_empty() {
            return Ok(stats);
        }
        self.frame = self.frame.wrapping_add(1);
        if self.cursor.1 + self.cursor.2 > self.atlas_size * 3 / 4 || self.entries.len() >= 8192 {
            self.entries.clear();
            self.atlas_epoch = self.atlas_epoch.wrapping_add(1);
            self.cursor = (0, 0, 0);
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
        });
        let mut draws = Vec::new();
        for item in items {
            // Retained hidden subtrees must not allocate textures, shape text,
            // or trigger filters until their effective opacity is visible.
            if item.effects.opacity <= 0. {
                continue;
            }
            let node_clip = if !item.isolated && matches!(item.kind, NodeKind::Text { .. } | NodeKind::RichText { .. }) {
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
                    } else if let NodeKind::Quad(style) | NodeKind::Panel { quad: style, .. } =
                        item.kind
                    {
                        style.paint_bounds(item.bounds)
                    } else if let NodeKind::Svg(svg) = item.kind {
                        svg.paint_bounds(item.bounds)
                    } else if let NodeKind::Image(image) = item.kind {
                        image.paint_bounds(item.bounds)
                    } else {
                        item.bounds
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
                    uv: [0., 0., 1., 1.],
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
                });
                draws.push(Draw {
                    start,
                    end: quads.len() as u32,
                    clip,
                    image: None,
                    layer: Some(item.id),
                    blur: (item.effects.blur_radius > 0.).then_some((item.bounds, item.effects)),
                });
                continue;
            }
            let bounds = item.bounds;
            let fade = [bounds.x, bounds.y, bounds.width, bounds.height];
            let mut canvas_image = None;
            match item.kind {
                NodeKind::Canvas(canvas) => {
                    if bounds.width <= 0. || bounds.height <= 0. { continue; }
                    let before = self.canvases.rasterizations();
                    let image = self.canvases.get(item.id, canvas, bounds.width, bounds.height, self.scale)?;
                    stats.canvas_rasterizations += (self.canvases.rasterizations() - before) as usize;
                    if !self.images.contains_key(&image.id()) { self.upload_image(&image)?; stats.image_uploads += 1; }
                    canvas_image = Some(image.id());
                    quads.push(Quad { rect: fade, uv: [0.,0.,1.,1.], color: [1.,1.,1.,item.effects.opacity], fade, options: [item.effects.edge_fade,2.,0.,0.], shape:[0.;4], border:[0.;4] });
                }
                NodeKind::Rect(color) => quads.push(Quad {
                    rect: fade,
                    uv: [0.; 4],
                    color: rgba(*color, item.effects.opacity),
                    fade,
                    options: [item.effects.edge_fade, 0., 0., 0.],
                    shape: [0.; 4],
                    border: [0.; 4],
                }),
                NodeKind::Quad(style) | NodeKind::Panel { quad: style, .. } => {
                    if style.shadows().len() > 32 { return Err(GpuError("panel exceeds 32 shadows".into())); }
                    for shadow in style.shadows() {
                        let core = bounds.expand(shadow.spread.max(0.));
                        let core = Rect::new(
                            core.x + shadow.offset.x,
                            core.y + shadow.offset.y,
                            core.width,
                            core.height,
                        );
                        let outer = core.expand(shadow.blur_radius.max(0.) * 3.);
                        quads.push(Quad {
                            rect: [outer.x, outer.y, outer.width, outer.height],
                            uv: [0.; 4],
                            color: rgba(shadow.color, item.effects.opacity),
                            fade: [core.x, core.y, core.width, core.height],
                            options: [0.,0.,0.,1.],
                            shape: [
                                style.radius + shadow.spread.max(0.),
                                0.,
                                shadow.blur_radius.max(0.001),
                                0.,
                            ],
                            border: decoration::shadow_corners(style, core.width, core.height, shadow.spread.max(0.)),
                        });
                    }
                    if style.decoration.is_some() && bounds.width > 0. && bounds.height > 0. {
                        let before = self.canvases.rasterizations();
                        let image = self.canvases.get_decoration(item.id, style, bounds.width, bounds.height, self.scale)?;
                        stats.canvas_rasterizations += (self.canvases.rasterizations() - before) as usize;
                        if !self.images.contains_key(&image.id()) { self.upload_image(&image)?; stats.image_uploads += 1; }
                        canvas_image = Some(image.id());
                        quads.push(Quad {rect:fade,uv:[0.,0.,1.,1.],color:[1.,1.,1.,item.effects.opacity],fade,options:[item.effects.edge_fade,2.,0.,0.],shape:[0.;4],border:[0.;4]});
                    } else {
                    quads.push(Quad {
                        rect: fade,
                        uv: [0.; 4],
                        color: rgba(style.fill, item.effects.opacity),
                        fade,
                        options: [item.effects.edge_fade, 0., 0., 0.],
                        shape: [style.radius.max(0.), style.border_width.max(0.), 0., 1.],
                        border: rgba(style.border_color, item.effects.opacity),
                    });
                    }
                }
                NodeKind::Svg(svg) => {
                    if svg.paint_bounds(bounds).width<=0. || svg.paint_bounds(bounds).height<=0. {continue;}
                    let before=self.svgs.rasterizations();
                    let image=self.svgs.get(item.id,svg,bounds.width,bounds.height,self.scale)?;
                    stats.svg_rasterizations+=(self.svgs.rasterizations()-before) as usize;
                    if !self.images.contains_key(&image.id()) {self.upload_image(&image)?;stats.image_uploads+=1;}
                    canvas_image=Some(image.id());
                    let transform=image.paint_transform(bounds);
                    let transformed=image.transform()!=lumine::affine::Affine::IDENTITY;
                    quads.push(Quad {rect:fade,uv:[0.,0.,1.,1.],color:[1.,1.,1.,item.effects.opacity],fade,options:[item.effects.edge_fade,2.,if transformed {1.}else{0.},0.],shape:if transformed {[transform.a,transform.c,transform.b,transform.d]}else{[0.;4]},border:if transformed {[transform.tx,transform.ty,0.,0.]}else{[0.;4]}});
                }
                NodeKind::Image(image) => {
                    if image.paint_bounds(bounds).width <= 0. || image.paint_bounds(bounds).height <= 0. {
                        continue;
                    }
                    if !self.images.contains_key(&image.id()) {
                        self.upload_image(image)?;
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
                        options: [item.effects.edge_fade, 2., if transformed { 1. } else { 0. }, 0.],
                        shape: if transformed { [transform.a, transform.c, transform.b, transform.d] } else { [0.; 4] },
                        border: if transformed { [transform.tx, transform.ty, 0., 0.] } else { [0.; 4] },
                    });
                }
                kind @ (NodeKind::Text { .. } | NodeKind::RichText { .. }) => {
                    let rich = match kind { NodeKind::RichText { text } => Some(text.clone()), _ => None };
                    let rich_content = rich.as_ref().map(|r| r.text_arc());
                    let (text, color, font_size) = match kind {
                        NodeKind::Text { text, color, font_size } => (text, *color, *font_size),
                        NodeKind::RichText { .. } => (rich_content.as_ref().unwrap(), lumine::scene::Color(255,255,255,255), 16.),
                        _ => unreachable!(),
                    };
                    if !font_size.is_finite() || font_size <= 0. {
                        return Err(GpuError("font size must be finite and positive".into()));
                    }
                    let stale = self.shapes.get(&item.id).is_none_or(|s| {
                        s.rich != rich || s.text_options != item.text_options || s.text != *text
                            || s.font != *item.font
                            || s.size != font_size
                            || s.width != bounds.width
                            || s.height != bounds.height
                    });
                    if stale {
                        if let Some(old) = self.shapes.remove(&item.id) {
                            self.shaped_bytes -= old.bytes();
                        }
                        let mut fonts = self.fonts.borrow_mut();
                        let b = if let Some(rich) = &rich {
                            text::rich_buffer(&mut fonts, rich, Some(bounds.width), Some(bounds.height))
                        } else if item.text_options != Default::default() {
                            text::display_buffer(&mut fonts,text,item.font,font_size,Some(bounds.width),Some(bounds.height),item.text_options)
                        } else {
                            let mut b = Buffer::new(&mut fonts, Metrics::new(font_size.max(1.), item.font.line_height.resolve(font_size)));
                            b.set_size(&mut fonts, Some(bounds.width), Some(bounds.height));
                            text::set_buffer_text(&mut fonts, &mut b, text, item.font, font_size.max(1.));
                            b.shape_until_scroll(&mut fonts, false);
                            b
                        };
                        let mut glyphs = Vec::new();
                        for run in b.layout_runs() {
                            for glyph in run.glyphs {
                                let p = glyph.physical((0., 0.), self.scale);
                                glyphs.push(Glyph {
                                    key: p.cache_key,
                                    color: glyph.color_opt.map(|c| lumine::scene::Color(c.r(),c.g(),c.b(),c.a())),
                                    x: p.x as f32,
                                    y: p.y as f32 + run.line_y * self.scale,
                                });
                            }
                        }
                        let shaped = Shaped {
                            decorations: rich.as_ref().map_or_else(Vec::new, |rich| text::rich_decorations(&b, rich)),
                            rich: rich.clone(),
                            text_options: item.text_options,
                            font: item.font.clone(),
                            text: text.clone(),
                            size: font_size,
                            width: bounds.width,
                            height: bounds.height,
                            glyphs,
                            quads: Vec::new(),
                            atlas_epoch: 0,
                            last_used: self.frame,
                        };
                        self.shaped_bytes += shaped.bytes();
                        self.shapes.insert(item.id, shaped);
                        stats.shaped_nodes += 1;
                    }
                    let mut shaped = self.shapes.remove(&item.id).unwrap();
                    self.shaped_bytes -= shaped.bytes();
                    if shaped.atlas_epoch != self.atlas_epoch {
                        shaped.quads.clear();
                        shaped.quads.extend(shaped.decorations.iter().filter(|d| d.background).map(rich_decoration_quad));
                        for glyph in &shaped.glyphs {
                            if let Some(a) = self.glyph(glyph.key, &mut stats)? {
                                shaped.quads.push(Quad {
                                    rect: [
                                        (glyph.x + a.left as f32) / self.scale,
                                        (glyph.y - a.top as f32) / self.scale,
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
                                });
                            }
                        }
                        shaped.quads.extend(shaped.decorations.iter().filter(|d| !d.background).map(rich_decoration_quad));
                        shaped.atlas_epoch = self.atlas_epoch;
                        stats.geometry_rebuilds += 1;
                    }
                    for cached in &shaped.quads {
                        let mut quad = *cached;
                        quad.rect[0] += bounds.x;
                        quad.rect[1] += bounds.y;
                        quad.color = if rich.is_some() {
                            [cached.color[0], cached.color[1], cached.color[2], cached.color[3] * item.effects.opacity]
                        } else { rgba(color, item.effects.opacity) };
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
            if quads.len() as u32 > start || item.effects.blur_radius > 0. {
                draws.push(Draw {
                    layer: None,
                    blur: (item.effects.blur_radius > 0.).then_some((bounds, item.effects)),
                    start,
                    end: quads.len() as u32,
                    clip,
                    image: if let Some(image) = canvas_image { Some(image) } else if let NodeKind::Image(image) = item.kind {
                        Some(image.id())
                    } else {
                        None
                    },
                });
            }
        }
        let needed = quads.len() * std::mem::size_of::<Quad>();
        if needed > VERTEX_BUDGET {
            return Err(GpuError("frame geometry exceeds 32 MiB budget".into()));
        }
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
        self.queue
            .write_buffer(&self.vertices, 0, bytemuck::cast_slice(&quads));
        let vertices = self.vertices.clone();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        // A clean filter elsewhere in the retained target needs no replay and
        // must not force every unrelated quad into its own render pass.
        let has_damaged_blur = has_blur
            && draws.iter().any(|draw| {
                draw.blur.is_some_and(|(bounds, _)| {
                    damage.iter().any(|region| {
                        region
                            .intersection(draw.clip)
                            .and_then(|rect| rect.intersection(bounds))
                            .and_then(|output| scissor(output, self.scale, self.width, self.height))
                            .is_some()
                    })
                })
            });
        if !has_damaged_blur {
            stats.render_passes += 1;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.set_bind_group(0, &self.bind, &[]);
            for region in &damage {
                if let Some((x, y, w, h)) = scissor(*region, self.scale, self.width, self.height) {
                    stats.damaged_pixels += u64::from(w) * u64::from(h);
                    pass.set_scissor_rect(x, y, w, h);
                    pass.set_pipeline(&self.clear_pipeline);
                    pass.set_bind_group(0, &self.bind, &[]);
                    pass.draw(0..6, 0..1);
                    stats.draw_calls += 1;
                    pass.set_pipeline(&self.pipeline);
                    for draw in &draws {
                        if let Some(rect) = region.intersection(draw.clip)
                            && let Some((x, y, w, h)) =
                                scissor(rect, self.scale, self.width, self.height)
                        {
                            pass.set_scissor_rect(x, y, w, h);
                            pass.set_bind_group(
                                0,
                                draw.image.and_then(|id| self.images.get(&id)).map_or_else(
                                    || {
                                        draw.layer
                                            .and_then(|id| self.layers.get(&id))
                                            .map_or(&self.bind, |l| &l.bind)
                                    },
                                    |entry| &entry.0,
                                ),
                                &[],
                            );
                            pass.draw(0..6, draw.start..draw.end);
                            stats.draw_calls += 1;
                            stats.instances += (draw.end - draw.start) as usize;
                        }
                    }
                }
            }
        } else {
            for region in &damage {
                let Some((x, y, w, h)) = scissor(*region, self.scale, self.width, self.height)
                else {
                    continue;
                };
                stats.damaged_pixels += u64::from(w) * u64::from(h);
                self.draw_range(
                    &mut encoder,
                    &vertices,
                    (x, y, w, h),
                    0..1,
                    true,
                    (None, None),
                );
                stats.draw_calls += 1;
                stats.render_passes += 1;
                for draw in &draws {
                    if let Some(rect) = region.intersection(draw.clip) {
                        if let Some((bounds, effects)) = draw.blur
                            && let Some(output) = rect.intersection(bounds)
                            && self.blur(&mut encoder, bounds, output, effects)
                        {
                            stats.draw_calls += 2;
                            stats.render_passes += 2;
                            stats.blur_passes += 2;
                        }
                        if draw.end > draw.start
                            && let Some(scissor) =
                                scissor(rect, self.scale, self.width, self.height)
                        {
                            self.draw_range(
                                &mut encoder,
                                &vertices,
                                scissor,
                                draw.start..draw.end,
                                false,
                                (draw.image, draw.layer),
                            );
                            stats.draw_calls += 1;
                            stats.render_passes += 1;
                            stats.instances += (draw.end - draw.start) as usize;
                        }
                    }
                }
            }
        }
        self.queue.submit([encoder.finish()]);
        self.fresh = false;
        self.frame_quads = quads;
        Ok(stats)
    }
    fn draw_range(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        vertices: &wgpu::Buffer,
        scissor: (u32, u32, u32, u32),
        range: std::ops::Range<u32>,
        clear: bool,
        source: (Option<u64>, Option<NodeId>),
    ) {
        let (image, layer) = source;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("damaged quads"),
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
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_pipeline(if clear {
            &self.clear_pipeline
        } else {
            &self.pipeline
        });
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
        pass.set_scissor_rect(scissor.0, scissor.1, scissor.2, scissor.3);
        pass.draw(0..6, range);
    }
    fn blur(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        bounds: Rect,
        output: Rect,
        effects: lumine::scene::Effects,
    ) -> bool {
        if self.blur_resources.is_none() {
            let source = texture(&self.device, self.width, self.height, "backdrop source");
            let intermediate = texture(&self.device, self.width, self.height, "horizontal blur");
            let shader = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("backdrop blur"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("blur.wgsl").into()),
                });
            let pipeline = self
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("backdrop blur"),
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
                            format: FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                });
            self.blur_resources = Some((pipeline, source, intermediate));
        }
        let (pipeline, source, intermediate) = self.blur_resources.as_ref().unwrap();
        let sigma = (effects.blur_radius * self.scale).clamp(0.1, 64.);
        let radius = (sigma * 3.).ceil() as u32;
        let Some((x, y, width, height)) = scissor(output, self.scale, self.width, self.height)
        else {
            return false;
        };
        // Physical coordinates stay integral throughout both passes and the copy.
        let left = x.saturating_sub(radius);
        let top = y.saturating_sub(radius);
        let right = (x + width + radius).min(self.width);
        let bottom = (y + height + radius).min(self.height);
        let origin = wgpu::Origin3d {
            x: left,
            y: top,
            z: 0,
        };
        let mut from = self.target.as_image_copy();
        from.origin = origin;
        let mut to = source.as_image_copy();
        to.origin = origin;
        encoder.copy_texture_to_texture(
            from,
            to,
            wgpu::Extent3d {
                width: right - left,
                height: bottom - top,
                depth_or_array_layers: 1,
            },
        );
        for vertical in [false, true] {
            let sample = if vertical { &intermediate } else { &source };
            let target = if vertical { &self.target } else { intermediate };
            let params: [f32; 12] = [
                if vertical { 0. } else { 1. },
                if vertical { 1. } else { 0. },
                sigma,
                if vertical { 1. } else { 0. },
                bounds.x * self.scale,
                bounds.y * self.scale,
                bounds.width * self.scale,
                bounds.height * self.scale,
                effects.opacity,
                effects.edge_fade * self.scale,
                0.,
                0.,
            ];
            let uniform = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("blur parameters"),
                    contents: bytemuck::cast_slice(&params),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("blur"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &sample.create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(
                            &source.create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: uniform.as_entire_binding(),
                    },
                ],
            });
            let view = target.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("separable blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            let (pass_y, pass_height) = if vertical {
                (y, height)
            } else {
                (top, bottom - top)
            };
            pass.set_scissor_rect(x, pass_y, width, pass_height);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        true
    }
    fn upload_image(&mut self, image: &lumine::image::ImageData) -> Result<(), GpuError> {
        let bytes = image.pixels().len();
        if bytes + self.images.values().map(|entry| entry.1).sum::<usize>() > 64 * 1024 * 1024 {
            return Err(GpuError(
                "visible image textures exceed the 64 MiB budget".into(),
            ));
        }
        if image.width() > self.device.limits().max_texture_dimension_2d
            || image.height() > self.device.limits().max_texture_dimension_2d
        {
            return Err(GpuError("image exceeds GPU texture dimensions".into()));
        }
        let texture = texture(&self.device, image.width(), image.height(), "image");
        let mut pixels = image.pixels().to_vec();
        for pixel in pixels.chunks_exact_mut(4) {
            let alpha = u16::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
        self.queue.write_texture(
            texture.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width() * 4),
                rows_per_image: None,
            },
            texture.size(),
        );
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
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
                            ..Default::default()
                        },
                    )),
                },
            ],
        });
        self.images.insert(image.id(), (bind, bytes));
        Ok(())
    }
    fn reset_atlas(&mut self, size: u32) {
        self.atlas = texture(&self.device, size, size, "glyph atlas");
        self.atlas_size = size;
        self.bind = self.texture_bind(&self.atlas.create_view(&Default::default()), "glyph atlas");
        self.entries.clear();
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
        if self.atlas_size == 1 {
            self.reset_atlas(INITIAL_ATLAS);
        }
        if self.cursor.0 + p.width + 1 > self.atlas_size {
            self.cursor.0 = 0;
            self.cursor.1 += self.cursor.2 + 1;
            self.cursor.2 = 0;
        }
        if self.cursor.1 + p.height > self.atlas_size || p.width > self.atlas_size {
            return Err(GpuError(
                "glyph atlas capacity exceeded in one frame".into(),
            ));
        }
        let entry = AtlasEntry {
            x: self.cursor.0,
            y: self.cursor.1,
            width: p.width,
            height: p.height,
            left: p.left,
            top: p.top,
        };
        let mut rgba = Vec::with_capacity((p.width * p.height * 4) as usize);
        match image.content {
            SwashContent::Mask => {
                for alpha in image.data {
                    rgba.extend_from_slice(&[255, 255, 255, alpha]);
                }
            }
            SwashContent::Color => rgba = image.data,
            SwashContent::SubpixelMask => {
                for pixel in image.data.chunks_exact(4) {
                    rgba.extend_from_slice(&[255, 255, 255, pixel[0].max(pixel[1]).max(pixel[2])]);
                }
            }
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.atlas,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: entry.x,
                    y: entry.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(p.width * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: p.width,
                height: p.height,
                depth_or_array_layers: 1,
            },
        );
        self.cursor.0 += p.width + 1;
        self.cursor.2 = self.cursor.2.max(p.height);
        self.entries.insert(key, Some(entry));
        stats.glyph_uploads += 1;
        Ok(Some(entry))
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
    pub fn present_with_notify(&mut self, before_present:impl FnOnce()) -> Result<PresentationStatus, GpuError> {
        if self.surface.is_none() {
            return Ok(PresentationStatus::Offscreen);
        }
        let frame = match surface::acquire(self)? {
            surface::Acquired::Frame(frame) => frame,
            surface::Acquired::Skipped(status) => return Ok(status),
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
        self.queue.submit([encoder.finish()]);
        before_present();
        frame.present();
        Ok(PresentationStatus::Presented)
    }
    pub fn readback(&self) -> Result<Vec<u8>, GpuError> {
        let stride = (self.width * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(stride) * u64::from(self.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: None,
                },
            },
            self.target.size(),
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
        drop(mapped);
        buffer.unmap();
        Ok(pixels)
    }
}
fn texture(device: &wgpu::Device, width: u32, height: u32, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
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
    Quad { rect:[d.bounds.x,d.bounds.y,d.bounds.width,d.bounds.height], color:rgba(d.color,1.),
        uv:[0.;4], fade:[0.;4], options:[0.;4], shape:[0.;4],border:[0.;4] }
}
fn rgba(c: Color, opacity: f32) -> [f32; 4] {
    [
        c.0 as f32 / 255.,
        c.1 as f32 / 255.,
        c.2 as f32 / 255.,
        c.3 as f32 / 255. * opacity,
    ]
}
fn quad_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    blend: bool,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{label:Some("quads"),layout:Some(layout),vertex:wgpu::VertexState{module:shader,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[wgpu::VertexBufferLayout{array_stride:112,step_mode:wgpu::VertexStepMode::Instance,attributes:&wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4,2=>Float32x4,3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4]}]},primitive:Default::default(),depth_stencil:None,multisample:Default::default(),fragment:Some(wgpu::FragmentState{module:shader,entry_point:Some("fs"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState{format:FORMAT,blend:blend.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),write_mask:wgpu::ColorWrites::ALL})]}),multiview_mask:None,cache:None})
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
        .filter_map(|(output, radius)| {
            let width = (viewport.width * scale).round() as u32;
            let height = (viewport.height * scale).round() as u32;
            let output = output.intersection(viewport)?;
            let (x, y, w, h) = scissor(output, scale, width, height)?;
            let halo = ((*radius * scale).clamp(0.1, 64.) * 3.).ceil() as u32;
            let left = x.saturating_sub(halo);
            let top = y.saturating_sub(halo);
            let right = (x + w + halo).min(width);
            let bottom = (y + h + halo).min(height);
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
    let mut regions: Vec<Rect> = Vec::new();
    for r in damage {
        let Some(mut r) = rounded_damage(*r, viewport, scale) else {
            continue;
        };
        let mut i = 0;
        while i < regions.len() {
            if regions[i].intersects(r) {
                r = r.union(regions.swap_remove(i));
                i = 0;
            } else {
                i += 1;
            }
        }
        regions.push(r);
    }
    regions
}

pub mod assets;

pub mod text;

fn create_quad_pipelines(device: &wgpu::Device) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("quads"),
        source: wgpu::ShaderSource::Wgsl(include_str!("draw.wgsl").into()),
    });
    let group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("quads"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
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
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("quads"),
        bind_group_layouts: &[Some(&group_layout)],
        immediate_size: 0,
    });
    let pipeline = quad_pipeline(device, &shader, &layout, true);
    let clear_pipeline = quad_pipeline(device, &shader, &layout, false);
    (pipeline, clear_pipeline)
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
        Ok(())
    }
    fn configure(&mut self) {
        self.surface
            .as_ref()
            .unwrap()
            .configure(&self.device, self.config.as_ref().unwrap());
    }
}

#[cfg(test)]
mod blur_damage_tests {
    use super::*;

    #[test]
    fn distant_damage_is_local_and_sampling_halo_activates_filter() {
        let viewport = Rect::new(0., 0., 500., 300.);
        let filter = (Rect::new(100., 100., 40., 30.), 2.);
        let distant = Rect::new(10., 10., 5., 5.);
        assert_eq!(
            blur_damage(&[distant], &[filter], viewport, 1.),
            vec![distant]
        );
        assert_eq!(
            blur_damage(&[Rect::new(95., 110., 1., 1.)], &[filter], viewport, 1.),
            vec![Rect::new(94., 94., 52., 42.)]
        );
        assert!(blur_damage(&[], &[filter], viewport, 1.).is_empty());
    }

    #[test]
    fn chained_filters_expand_to_fixed_point_independent_of_order() {
        let viewport = Rect::new(0., 0., 500., 300.);
        let filters = [
            (Rect::new(145., 100., 20., 20.), 2.),
            (Rect::new(120., 100., 20., 20.), 2.),
        ];
        let damage = [Rect::new(115., 105., 1., 1.)];
        let expected = vec![Rect::new(114., 94., 57., 32.)];
        assert_eq!(blur_damage(&damage, &filters, viewport, 1.), expected);
        assert_eq!(
            blur_damage(&damage, &[filters[1], filters[0]], viewport, 1.),
            expected
        );
    }

    #[test]
    fn reconstruction_covers_integer_sampling_support_across_fractional_scales() {
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
                    let r = ((radius * scale).clamp(0.1, 64.) * 3.).ceil() as u32;
                    let regions = blur_damage(&[output], &[(output, radius)], viewport, scale);
                    assert_eq!(regions.len(), 1);
                    let (dx, dy, dw, dh) = scissor(regions[0], scale, 701, 503).unwrap();
                    assert!(dx <= x.saturating_sub(r) && dy <= y.saturating_sub(r));
                    assert!(dx + dw >= (x + w + r).min(701));
                    assert!(dy + dh >= (y + h + r).min(503));
                }
            }
        }
    }

    #[test]
    fn fractional_output_rounds_before_integer_kernel_halo_and_clips_viewport() {
        let viewport = Rect::new(0., 0., 400., 300.);
        // At 1.25x the output spans physical [12,26), radius ceil(.3125*3)=1.
        let regions = blur_damage(
            &[Rect::new(10.2, 10.2, 1., 1.)],
            &[(Rect::new(10.1, 10.1, 10.1, 10.1), 0.25)],
            viewport,
            1.25,
        );
        let (x, y, width, height) = scissor(regions[0], 1.25, 500, 375).unwrap();
        assert!(x <= 11 && y <= 11 && x + width >= 27 && y + height >= 27);
        assert!(width < 20 && height < 20);
        let clamped = blur_damage(
            &[Rect::new(0., 0., 1., 1.)],
            &[(Rect::new(0., 0., 1., 1.), 10000.)],
            viewport,
            1.,
        );
        assert_eq!(clamped, vec![Rect::new(0., 0., 193., 193.)]);
    }
}

pub mod canvas;

pub mod decoration;

pub mod svg;
