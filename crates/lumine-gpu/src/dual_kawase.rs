//! Bounded Dual Kawase pyramid; cache the completed upsample for cheap composites.
use super::*;

const MAX_LEVELS: usize = 7;
pub(super) struct Kawase {
    layout: wgpu::BindGroupLayout,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    pub scratch: Option<Scratch>,
}
pub(super) struct Scratch {
    width: u32,
    height: u32,
    down: Vec<wgpu::Texture>,
    up: Vec<wgpu::Texture>,
}
impl Kawase {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Dual Kawase"),
            source: wgpu::ShaderSource::Wgsl(include_str!("dual_kawase.wgsl").into()),
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
            label: Some("Dual Kawase"),
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
        let build = |entry| {
            BlurPipelines::pipeline(
                device,
                &shader,
                Some(&layout),
                entry,
                None,
                std::slice::from_ref(&target),
            )
        };
        let down = build("fs_down");
        let up = build("fs_up");
        Self {
            layout,
            down,
            up,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Dual Kawase linear"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            scratch: None,
        }
    }
    pub(super) fn bytes(&self) -> usize {
        self.scratch.as_ref().map_or(0, |s| {
            s.down
                .iter()
                .chain(&s.up)
                .map(|t| t.width() as usize * t.height() as usize * 4)
                .sum()
        })
    }
    pub(super) fn trim_to(&mut self, width: u32, height: u32) {
        if self.scratch.as_ref().is_some_and(|s| {
            s.width > width.next_multiple_of(64) || s.height > height.next_multiple_of(64)
        }) {
            self.scratch = None;
        }
    }
    fn reserve(&mut self, device: &wgpu::Device, width: u32, height: u32, levels: usize) -> usize {
        let mut allocations = 0;
        if self
            .scratch
            .as_ref()
            .is_none_or(|s| s.width < width || s.height < height)
        {
            let limit = device.limits().max_texture_dimension_2d;
            self.scratch = Some(Scratch {
                // Do not combine maximum dimensions from opposite aspect ratios.
                width: width.next_multiple_of(64).min(limit),
                height: height.next_multiple_of(64).min(limit),
                down: Vec::new(),
                up: Vec::new(),
            });
        }
        let s = self.scratch.as_mut().unwrap();
        while s.down.len() < levels {
            let divisor = 1 << (s.down.len() + 1);
            let (w, h) = (s.width.div_ceil(divisor), s.height.div_ceil(divisor));
            s.down.push(texture(device, w, h, "Kawase down"));
            allocations += 1;
        }
        while s.up.len() < levels - 1 {
            let divisor = 1 << (s.up.len() + 1);
            s.up.push(texture(
                device,
                s.width.div_ceil(divisor),
                s.height.div_ceil(divisor),
                "Kawase up",
            ));
            allocations += 1;
        }
        allocations
    }
}

/// Crossfade by variance rather than jumping to the next integer depth.
/// This maps the existing sigma approximately; the kernel is not an exact Gaussian.
fn plan(sigma: f32) -> (usize, f32) {
    let variance = |level: usize| (35. / 72.) * (4_f32.powi(level as i32) - 1.);
    let mut levels = 1;
    while levels < MAX_LEVELS && sigma * sigma > variance(levels) {
        levels += 1;
    }
    let low = variance(levels - 1);
    (
        levels,
        ((sigma * sigma - low) / (variance(levels) - low)).clamp(0., 1.),
    )
}

impl GpuRenderer {
    /// Change filters without changing scene nodes. Flushes deferred work and repaints caches.
    pub fn set_blur_algorithm(&mut self, algorithm: BlurAlgorithm) {
        if self.blur_algorithm == algorithm {
            return;
        }
        self.flush_pending();
        self.blur_algorithm = algorithm;
        self.layers.clear();
        self.scroll_history = None;
        self.scroll_state = None;
        self.blur_cache.clear();
        self.gaussian_scratch = None;
        if let Some(kawase) = &mut self.kawase {
            kawase.scratch = None;
        }
        self.fresh = true;
    }
    pub fn blur_algorithm(&self) -> BlurAlgorithm {
        self.blur_algorithm
    }
    pub(super) fn blur_vertical(&self, _blur: &VerticalBlur) -> &wgpu::RenderPipeline {
        &self.blur_composite.as_ref().expect("filtered").vertical
    }
    pub(super) fn kawase_blur(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        area: (u32, u32, u32, u32),
        scissors: Vec<(u32, u32, u32, u32)>,
        sigma: f32,
        mut textures: Vec<wgpu::Texture>,
    ) -> (VerticalBlur, Vec<wgpu::Texture>) {
        let (x, y, w, h) = area;
        let (levels, blend) = plan(sigma);
        let prepasses = levels * 2;
        let stamps: Vec<_> = (0..prepasses)
            .map(|i| {
                self.profile_pass(if i < levels {
                    "blur_kawase_down"
                } else {
                    "blur_kawase_up"
                })
            })
            .collect();
        let kawase = self.kawase.get_or_insert_with(|| Kawase::new(&self.device));
        let mut allocations = kawase.reserve(&self.device, w, h, levels);
        let scratch = kawase.scratch.as_ref().unwrap();
        allocations += blur_cache::reserve(&self.device, &mut textures, 0, w, h);
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.target,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            textures[0].as_image_copy(),
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let original = textures[0].create_view(&Default::default());
        let bind = |source: &wgpu::Texture, raw: &wgpu::TextureView, size: [f32; 4], alpha: f32| {
            let params: [f32; 8] = [size[0], size[1], size[2], size[3], 1., alpha, 0., 0.];
            let uniform = upload::init_buffer(
                &self.device,
                self.mapped.is_some(),
                "Kawase parameters",
                bytemuck::cast_slice(&params),
                wgpu::BufferUsages::UNIFORM,
            );
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Kawase"),
                layout: &kawase.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &source.create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(raw),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&kawase.sampler),
                    },
                ],
            })
        };
        let mut stamp = 0;
        let mut pass = |pipeline: &wgpu::RenderPipeline,
                        group: &wgpu::BindGroup,
                        target: &wgpu::Texture,
                        width: u32,
                        height: u32| {
            let view = target.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Dual Kawase"),
                timestamp_writes: stamps[stamp].as_ref().map(|s| s.writes()),
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
            pass.set_viewport(0., 0., width as f32, height as f32, 0., 1.);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
            stamp += 1;
        };
        let mut source = &textures[0];
        let mut sw = w;
        let mut sh = h;
        for i in 0..levels {
            let (dw, dh) = (sw.div_ceil(2), sh.div_ceil(2));
            let group = bind(
                source,
                &original,
                [sw as f32, sh as f32, dw as f32, dh as f32],
                1.,
            );
            pass(&kawase.down, &group, &scratch.down[i], dw, dh);
            source = &scratch.down[i];
            sw = dw;
            sh = dh;
        }
        for i in (0..levels - 1).rev() {
            let divisor = 1 << (i + 1);
            let (dw, dh) = (w.div_ceil(divisor), h.div_ceil(divisor));
            let raw = scratch.down[i].create_view(&Default::default());
            let group = bind(
                source,
                &raw,
                [sw as f32, sh as f32, dw as f32, dh as f32],
                if i == levels - 2 { blend } else { 1. },
            );
            pass(&kawase.up, &group, &scratch.up[i], dw, dh);
            source = &scratch.up[i];
            sw = dw;
            sh = dh;
        }
        let source = source.clone();
        allocations += blur_cache::reserve(&self.device, &mut textures, 1, w, h);
        let group = bind(
            &source,
            &original,
            [sw as f32, sh as f32, w as f32, h as f32],
            if levels == 1 { blend } else { 1. },
        );
        pass(&kawase.up, &group, &textures[1], w, h);
        (
            VerticalBlur {
                geometry: None,
                bind: group,
                scissors,
                prepasses,
                predraws: prepasses,
                allocations,
            },
            textures,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opposite_aspect_crops_do_not_combine_into_a_square_pyramid() {
        let gpu = GpuRenderer::new(64, 64).unwrap();
        let mut kawase = Kawase::new(&gpu.device);
        kawase.reserve(&gpu.device, 1024, 64, 4);
        assert!(kawase.bytes() < 256 * 1024);
        kawase.reserve(&gpu.device, 64, 1024, 4);
        let scratch = kawase.scratch.as_ref().unwrap();
        assert_eq!((scratch.width, scratch.height), (64, 1024));
        assert!(kawase.bytes() < 256 * 1024);
        kawase.trim_to(64, 64);
        assert!(kawase.scratch.is_none());
        assert_eq!(kawase.bytes(), 0);
    }
}
