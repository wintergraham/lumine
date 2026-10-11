use super::*;
use std::hash::{Hash, Hasher};
use lumine::scene::Effects;
pub(super) const BUDGET: usize = 64 * 1024 * 1024;
pub(super) struct CachedBlur {
    pub fingerprint: u64,
    pub filter: VerticalBlur,
    pub textures: Vec<wgpu::Texture>,
    pub used: u64,
    pub composite: [u32; 10],
}
impl CachedBlur {
    pub fn bytes(&self) -> usize {
        self.textures
            .iter()
            .map(|t| t.width() as usize * t.height() as usize * 4)
            .sum()
    }
}
/// Hash ordered draws contributing to each bounded input. Earlier filters include
/// their own input fingerprint, so overlapping dependencies propagate in paint order.
pub(super) fn fingerprints(
    draws: &[Draw],
    quads: &[Quad],
    layers: &FxHashMap<NodeId, LayerCache>,
    scale: f32,
    background: Color,
    atlas_epoch: u64,
    algorithm: BlurAlgorithm,
) -> FxHashMap<NodeId, u64> {
    let mut result = FxHashMap::default();
    for (index, draw) in draws.iter().enumerate() {
        let Some((id, bounds, effects)) = draw.blur else {
            continue;
        };
        let Some(area) = bounds.intersection(draw.clip) else {
            continue;
        };
        let area = snap_out(area, scale);
        let mut hash = rustc_hash::FxHasher::default();
        (
            scale.to_bits(),
            background.0,
            background.1,
            background.2,
            background.3,
            atlas_epoch,
            algorithm as u8,
        )
            .hash(&mut hash);
        for v in [area.x, area.y, area.width, area.height, effects.blur_radius] {
            v.to_bits().hash(&mut hash);
        }
        for source in &draws[..index] {
            if !source.clip.intersects(area) {
                continue;
            }
            if let Some((prior, rect, effects)) = source.blur
                && rect.intersects(area)
            {
                source.geometry_hash.hash(&mut hash);
                result.get(&prior).hash(&mut hash);
                composite_key(rect, effects, source.blur_mask).hash(&mut hash);
            }
            let mut touched = false;
            for quad in &quads[source.start as usize..source.end as usize] {
                let bounds = source.transform.bounds(quad_paint_bounds(quad));
                if bounds.intersects(area) {
                    hash.write(bytemuck::bytes_of(quad));
                    touched = true;
                }
            }
            if touched {
                source.geometry_hash.hash(&mut hash);
                source.image.hash(&mut hash);
                if let Some(id) = source.layer {
                    layers[&id].revision.hash(&mut hash);
                }
                for v in [
                    source.clip.x,
                    source.clip.y,
                    source.clip.width,
                    source.clip.height,
                ] {
                    v.to_bits().hash(&mut hash);
                }
            }
        }
        result.insert(id, hash.finish());
    }
    result
}
pub(super) fn composite_key(
    bounds: Rect,
    effects: Effects,
    mask: Option<lumine::scene::FadeMask>,
) -> [u32; 10] {
    let mask = mask.map_or(NO_MASK, |(rect, [top, bottom])| {
        [rect.y, rect.y + rect.height, top, bottom]
    });
    [
        bounds.x,
        bounds.y,
        bounds.width,
        bounds.height,
        effects.opacity,
        effects.edge_fade,
        mask[0],
        mask[1],
        mask[2],
        mask[3],
    ]
    .map(f32::to_bits)
}
/// Recycle a retired filter's bounded textures when dimensions still fit.
pub(super) fn reserve(
    device: &wgpu::Device,
    textures: &mut Vec<wgpu::Texture>,
    index: usize,
    w: u32,
    h: u32,
) -> usize {
    if textures
        .get(index)
        .is_none_or(|t| t.width() < w || t.height() < h)
    {
        let limit = device.limits().max_texture_dimension_2d;
        let t = texture(
            device,
            w.next_multiple_of(64).min(limit),
            h.next_multiple_of(64).min(limit),
            "cached blur",
        );
        if index == textures.len() {
            textures.push(t);
        } else {
            textures[index] = t;
        }
        return 1;
    }
    0
}
pub(super) fn reserve_scratch(
    device: &wgpu::Device,
    scratch: &mut Option<wgpu::Texture>,
    w: u32,
    h: u32,
) -> usize {
    if scratch
        .as_ref()
        .is_some_and(|t| t.width() >= w && t.height() >= h)
    {
        return 0;
    }
    let limit = device.limits().max_texture_dimension_2d;
    // A wide crop followed by a tall one must not retain a maximum-size square.
    *scratch = Some(texture(
        device,
        w.next_multiple_of(64).min(limit),
        h.next_multiple_of(64).min(limit),
        "Gaussian horizontal scratch",
    ));
    1
}
impl GpuRenderer {
    pub(super) fn blur_composite_bind(
        &mut self,
        original: &wgpu::Texture,
        filtered: &wgpu::Texture,
        bounds: Rect,
        origin: (u32, u32),
        effects: Effects,
        mask: Option<lumine::scene::FadeMask>,
    ) -> wgpu::BindGroup {
        let pipelines = self.blur_composite.get_or_insert_with(|| {
            let shader = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("cached blur composite"),
                    source: wgpu::ShaderSource::Wgsl(
                        (include_str!("paint_geometry.wgsl").replace("GEOMETRY_GROUP", "1")
                            + include_str!("blur_composite.wgsl"))
                        .into(),
                    ),
                });
            let layout = self
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("cached blur composite"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
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
                    ],
                });
            let vertical = BlurPipelines::pipeline(
                &self.device,
                &shader,
                Some(&layout),
                "fs",
                Some(&self.context.inner.geometry_layout),
                &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            );
            BlurPipelines {
                geometry_layout: Some(self.context.inner.geometry_layout.clone()),
                shader,
                layout,
                horizontal: vertical.clone(),
                crop: None,
                vertical,
                both: None,
                weights: Default::default(),
                sampler: self.device.create_sampler(&Default::default()),
            }
        });
        let mask = mask.map_or(NO_MASK, |(rect, [top, bottom])| {
            [rect.y, rect.y + rect.height, top, bottom]
        });
        let params = [
            bounds.x * self.scale,
            bounds.y * self.scale,
            bounds.width * self.scale,
            bounds.height * self.scale,
            effects.opacity,
            effects.edge_fade * self.scale,
            origin.0 as f32,
            origin.1 as f32,
            mask[0] * self.scale,
            mask[1] * self.scale,
            mask[2] * self.scale,
            mask[3] * self.scale,
        ];
        let uniform = upload::init_buffer(
            &self.device,
            self.mapped.is_some(),
            "blur composite parameters",
            bytemuck::cast_slice(&params),
            wgpu::BufferUsages::UNIFORM,
        );
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cached blur composite"),
            layout: &pipelines.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &filtered.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &original.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        })
    }
    pub(super) fn cache_blur(
        &mut self,
        id: NodeId,
        fingerprint: u64,
        filter: VerticalBlur,
        textures: Vec<wgpu::Texture>,
        composite: [u32; 10],
    ) {
        let entry = CachedBlur {
            fingerprint,
            filter,
            textures,
            used: self.frame,
            composite,
        };
        if entry.bytes() > BUDGET {
            return;
        }
        while self.blur_cache.len() >= 128
            || self.blur_cache.values().map(|c| c.bytes()).sum::<usize>() + entry.bytes() > BUDGET
        {
            let Some(old) = self
                .blur_cache
                .iter()
                .min_by_key(|(_, c)| c.used)
                .map(|(id, _)| *id)
            else {
                break;
            };
            self.blur_cache.remove(&old);
        }
        self.blur_cache.insert(id, entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opposite_aspect_gaussian_crops_reallocate_only_the_requested_scratch() {
        let gpu = GpuRenderer::new(64, 64).unwrap();
        let mut scratch = None;
        assert_eq!(reserve_scratch(&gpu.device, &mut scratch, 1024, 64), 1);
        assert_eq!(reserve_scratch(&gpu.device, &mut scratch, 64, 1024), 1);
        let texture = scratch.as_ref().unwrap();
        assert_eq!((texture.width(), texture.height()), (64, 1024));
        assert_eq!(reserve_scratch(&gpu.device, &mut scratch, 64, 1000), 0);
    }
}
