use rustc_hash::FxHashMap;
use std::collections::HashSet;
use wgpu::util::DeviceExt;
#[cfg(test)]
const BUDGET: usize = 64 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct Cache {
    pipelines: FxHashMap<&'static str, wgpu::ComputePipeline>,
    instances: FxHashMap<u64, Vec<Instance>>,
    live: HashSet<u64>,
    pub dispatches: usize,
    pub allocations: usize,
}
struct Instance {
    revision: u64,
    shader: &'static str,
    texture: wgpu::Texture,
    parameters: wgpu::Buffer,
    parameter_len: usize,
    bind: wgpu::BindGroup,
    display: wgpu::BindGroup,
}
impl Instance {
    fn bytes(&self) -> usize {
        self.texture.width() as usize * self.texture.height() as usize * 4 + self.parameter_len * 4
    }
}
fn candidate(entries: &[Instance], revision: u64, live: &HashSet<u64>) -> Option<usize> {
    entries
        .iter()
        .position(|e| e.revision == revision)
        .or_else(|| entries.iter().position(|e| !live.contains(&e.revision)))
}
fn fits(used: usize, replaced: usize, requested: usize, budget: usize) -> bool {
    used.saturating_sub(replaced)
        .checked_add(requested)
        .is_some_and(|bytes| bytes <= budget)
}
impl Cache {
    // Keep live immutable snapshots and one recyclable slot per instance.
    pub fn retain(&mut self, live: HashSet<u64>, instances: &HashSet<u64>) {
        self.live = live;
        self.instances.retain(|id, entries| {
            if !instances.contains(id) {
                return false;
            }
            let mut spare = false;
            entries.retain(|entry| {
                self.live.contains(&entry.revision)
                    || (!spare && {
                        spare = true;
                        true
                    })
            });
            true
        });
    }
    pub fn bytes(&self) -> usize {
        self.instances.values().flatten().map(Instance::bytes).sum()
    }
    fn reserve_capacity(&mut self, data: &lumine::image::ImageData, budget: usize) -> Result<(), &'static str> {
        let image = data.procedural().expect("procedural source");
        let requested =
            data.width() as usize * data.height() as usize * 4 + image.parameters.len() * 4;
        let replaced = |this: &Self| {
            image
                .instance
                .and_then(|id| this.instances.get(&id))
                .and_then(|entries| {
                    candidate(entries, data.id(), &this.live).map(|index| entries[index].bytes())
                })
                .unwrap_or(0)
        };
        let existing = replaced(self);
        // Animated uniforms normally keep the allocation unchanged. The
        // already-bounded cache cannot grow, so avoid scanning every instance.
        if requested <= existing || fits(self.bytes(), existing, requested, budget) {
            return Ok(());
        }
        // Retired snapshots are only recycling spares; discard them before
        // rejecting an otherwise valid set of immutable live shader outputs.
        self.instances.retain(|_, entries| {
            entries.retain(|entry| self.live.contains(&entry.revision));
            !entries.is_empty()
        });
        if fits(self.bytes(), replaced(self), requested, budget) {
            Ok(())
        } else {
            Err("procedural image resources exceed the renderer image budget")
        }
    }
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        display_layout: &wgpu::BindGroupLayout,
        data: &lumine::image::ImageData,
        budget: usize,
    ) -> Result<(wgpu::Texture, wgpu::BindGroup), &'static str> {
        self.reserve_capacity(data, budget)?;
        let revision = data.id();
        let width = data.width();
        let height = data.height();
        let image = data.procedural().expect("procedural source");
        let pipeline = self.pipelines.entry(image.shader).or_insert_with(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("procedural image"),
                source: wgpu::ShaderSource::Wgsl(image.shader.into()),
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("procedural image"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
        });
        let create = || {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("procedural image"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let parameters = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("procedural uniforms"),
                contents: bytemuck::cast_slice(&image.parameters),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });
            let view = texture.create_view(&Default::default());
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("procedural image"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: parameters.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                ],
            });
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            let display = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("procedural display"),
                layout: display_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });
            Instance {
                revision,
                shader: image.shader,
                texture,
                parameters,
                parameter_len: image.parameters.len(),
                bind,
                display,
            }
        };
        let mut temporary;
        let entry = if let Some(id) = image.instance {
            let entries = self.instances.entry(id).or_default();
            let index = candidate(entries, revision, &self.live);
            if let Some(index) = index {
                let e = &mut entries[index];
                if e.shader != image.shader
                    || e.texture.width() != width
                    || e.texture.height() != height
                    || e.parameter_len != image.parameters.len()
                {
                    *e = create();
                    self.allocations += 1;
                } else {
                    queue.write_buffer(&e.parameters, 0, bytemuck::cast_slice(&image.parameters));
                    e.revision = revision;
                }
                e
            } else {
                entries.push(create());
                self.allocations += 1;
                entries.last_mut().unwrap()
            }
        } else {
            temporary = create();
            self.allocations += 1;
            &mut temporary
        };
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &entry.bind, &[]);
        pass.dispatch_workgroups(image.dispatch[0], image.dispatch[1], 1);
        self.dispatches += 1;
        Ok((entry.texture.clone(), entry.display.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aggregate_capacity_counts_uniform_buffers_and_replaced_resources() {
        assert!(fits(32 * 1024 * 1024, 0, 32 * 1024 * 1024, BUDGET));
        assert!(!fits(32 * 1024 * 1024 + 4, 0, 32 * 1024 * 1024, BUDGET));
        assert!(fits(48 * 1024 * 1024, 32 * 1024 * 1024, 40 * 1024 * 1024, BUDGET));
        assert!(!fits(48 * 1024 * 1024, 8 * 1024 * 1024, 40 * 1024 * 1024, BUDGET));
        assert!(!fits(usize::MAX, 0, usize::MAX, BUDGET));
    }
}
