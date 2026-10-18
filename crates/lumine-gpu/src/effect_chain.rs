//! Ordered, retained image filters. Only a dirty suffix is dispatched. Live
//! immutable snapshots share prefixes but never rewrite each other's resources.
use rustc_hash::FxHashMap;
use std::{collections::HashSet, sync::Arc};
use wgpu::util::DeviceExt;
use lumine::image::{EffectStage, FilteredImage};

const BUDGET: usize = 64 * 1024 * 1024;
const PARAMS: usize = 4 + 97 * 2;
#[derive(Default)]
pub(crate) struct Cache {
    pipeline: Option<wgpu::ComputePipeline>,
    sampler: Option<wgpu::Sampler>,
    instances: FxHashMap<u64, Vec<Snapshot>>,
    weights: FxHashMap<u32, Vec<[f32; 2]>>,
    live: HashSet<u64>,
    next_resource: u64,
    pub dispatches: usize,
    pub allocations: usize,
    pub cache_hits: usize,
}
#[derive(Clone)]
struct Snapshot {
    revision: u64,
    input: u64,
    stages: Vec<(EffectStage, Vec<Arc<Resource>>)>,
}
struct Resource {
    id: u64,
    input: wgpu::Texture,
    texture: wgpu::Texture,
    parameters: wgpu::Buffer,
    values: [f32; PARAMS],
    bind: wgpu::BindGroup,
}
impl Resource {
    fn bytes(&self) -> usize {
        self.texture.width() as usize * self.texture.height() as usize * 4 + PARAMS * 4
    }
}
impl Cache {
    pub fn retain(&mut self, live: HashSet<u64>, instances: &HashSet<u64>) {
        self.live = live;
        self.instances.retain(|id, snapshots| {
            if !instances.contains(id) {
                return false;
            }
            let mut spare = false;
            snapshots.retain(|s| {
                self.live.contains(&s.revision)
                    || (!spare && {
                        spare = true;
                        true
                    })
            });
            true
        });
        if self.bytes() > BUDGET {
            self.instances
                .values_mut()
                .for_each(|snapshots| snapshots.retain(|s| self.live.contains(&s.revision)));
        }
    }
    pub fn bytes(&self) -> usize {
        let mut seen = HashSet::new();
        self.instances
            .values()
            .flatten()
            .flat_map(|s| &s.stages)
            .flat_map(|(_, passes)| passes)
            .filter(|pass| seen.insert(pass.id))
            .map(|pass| pass.bytes())
            .sum()
    }
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        revision: u64,
        chain: &FilteredImage,
        profiler: &std::cell::RefCell<Option<crate::profiling::Profiler>>,
    ) -> Result<wgpu::Texture, &'static str> {
        let width = source.width();
        let height = source.height();
        let pass_count: usize = chain
            .stages
            .iter()
            .map(|s| {
                if matches!(s, EffectStage::Blur { .. }) {
                    2
                } else {
                    1
                }
            })
            .sum();
        let mut allocated_bytes = self.bytes();
        if (width as usize * height as usize * 4 + PARAMS * 4) * pass_count > BUDGET {
            return Err("image effect intermediates exceed the 64 MiB budget");
        }
        if self.pipeline.is_none() {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("image effect chain"),
                source: wgpu::ShaderSource::Wgsl(include_str!("effect_chain.wgsl").into()),
            });
            self.pipeline = Some(device.create_compute_pipeline(
                &wgpu::ComputePipelineDescriptor {
                    label: Some("image effect chain"),
                    layout: None,
                    module: &module,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                },
            ));
            self.sampler = Some(device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }));
        }
        let pipeline = self.pipeline.as_ref().unwrap();
        let sampler = self.sampler.as_ref().unwrap();
        let snapshots = self.instances.entry(chain.instance).or_default();
        if let Some(snapshot) = snapshots.iter().find(|s| s.revision == revision) {
            self.cache_hits += chain.stages.len();
            return Ok(snapshot
                .stages
                .last()
                .unwrap()
                .1
                .last()
                .unwrap()
                .texture
                .clone());
        }
        // A retired snapshot can recycle its buffers and textures. Live ones
        // can contribute a shared prefix, but their dirty suffix stays immutable.
        let retired = snapshots
            .iter()
            .position(|s| !self.live.contains(&s.revision));
        let mut previous = if let Some(index) = retired {
            Some(snapshots.swap_remove(index))
        } else {
            snapshots.last().cloned()
        };
        // A removed snapshot's unshared resources are retired, not retained
        // cache usage. Count them again only when this revision reuses them.
        if retired.is_some() {
            allocated_bytes -= previous
                .as_ref()
                .unwrap()
                .stages
                .iter()
                .flat_map(|(_, resources)| resources)
                .filter(|resource| Arc::strong_count(resource) == 1)
                .map(|resource| resource.bytes())
                .sum::<usize>();
        }
        let mut dirty = previous
            .as_ref()
            .is_none_or(|s| s.input != chain.input.id());
        let mut input = source.clone();
        let mut stages = Vec::with_capacity(chain.stages.len());
        for (index, stage) in chain.stages.iter().copied().enumerate() {
            let old = previous.as_mut().and_then(|s| s.stages.get_mut(index));
            let same = old.as_ref().is_some_and(|(old_stage, resources)| {
                *old_stage == stage
                    && resources[0].texture.width() == width
                    && resources[0].texture.height() == height
            });
            dirty |= !same;
            if !dirty {
                let resources = std::mem::take(&mut old.unwrap().1);
                allocated_bytes += resources
                    .iter()
                    .filter(|resource| Arc::strong_count(resource) == 1)
                    .map(|resource| resource.bytes())
                    .sum::<usize>();
                let last = resources.last().unwrap();
                input = last.texture.clone();
                stages.push((stage, resources));
                self.cache_hits += 1;
                continue;
            }
            let mut recycled = old.map(|(_, resources)| std::mem::take(resources).into_iter());
            let mut params = [0f32; PARAMS];
            let directions = match stage {
                EffectStage::Blur { radius } => {
                    if !self.weights.contains_key(&radius.to_bits()) && self.weights.len() >= 64 {
                        self.weights.clear();
                    }
                    let samples = self
                        .weights
                        .entry(radius.to_bits())
                        .or_insert_with(|| crate::gaussian::samples(radius));
                    params[3] = samples.len() as f32;
                    for (i, [offset, weight]) in samples.iter().copied().enumerate() {
                        params[4 + i * 2] = offset;
                        params[5 + i * 2] = weight;
                    }
                    0..2
                }
                EffectStage::Dither { levels, cell_size } => {
                    params[1] = cell_size as f32;
                    params[2] = levels as f32;
                    2..3
                }
            };
            let mut resources = Vec::new();
            for direction in directions {
                params[0] = direction as f32;
                let retired = recycled
                    .as_mut()
                    .and_then(Iterator::next)
                    .and_then(|resource| Arc::try_unwrap(resource).ok());
                let mut resource = if let Some(resource) =
                    retired.filter(|r| r.texture.width() == width && r.texture.height() == height)
                {
                    allocated_bytes += resource.bytes();
                    let mut resource = resource;
                    if resource.values != params {
                        queue.write_buffer(&resource.parameters, 0, bytemuck::cast_slice(&params));
                        resource.values = params;
                    }
                    resource
                } else {
                    allocated_bytes += width as usize * height as usize * 4 + PARAMS * 4;
                    if allocated_bytes > BUDGET {
                        return Err("cached image effect intermediates exceed the 64 MiB budget");
                    }
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("image effect intermediate"),
                        size: source.size(),
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
                        label: Some("image effect parameters"),
                        contents: bytemuck::cast_slice(&params),
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    });
                    self.next_resource += 1;
                    self.allocations += 1;
                    let bind = bind(device, pipeline, sampler, &parameters, &texture, &input);
                    Resource {
                        id: self.next_resource,
                        input: input.clone(),
                        texture,
                        parameters,
                        values: params,
                        bind,
                    }
                };
                if resource.input != input {
                    resource.bind = bind(
                        device,
                        pipeline,
                        sampler,
                        &resource.parameters,
                        &resource.texture,
                        &input,
                    );
                    resource.input = input.clone();
                }
                {
                    let stamp = profiler.borrow_mut().as_mut().and_then(|p| p.pass("effect_chain"));
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("image effect stage"),
                        timestamp_writes: stamp.as_ref().map(|s| s.compute_writes()),
                    });
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, &resource.bind, &[]);
                    pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
                }
                self.dispatches += 1;
                input = resource.texture.clone();
                resources.push(Arc::new(resource));
            }
            stages.push((stage, resources));
        }
        snapshots.push(Snapshot {
            revision,
            input: chain.input.id(),
            stages,
        });
        Ok(input)
    }
}
fn bind(
    device: &wgpu::Device,
    pipeline: &wgpu::ComputePipeline,
    sampler: &wgpu::Sampler,
    parameters: &wgpu::Buffer,
    output: &wgpu::Texture,
    input: &wgpu::Texture,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("image effect stage"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: parameters.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(
                    &output.create_view(&Default::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(
                    &input.create_view(&Default::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}
