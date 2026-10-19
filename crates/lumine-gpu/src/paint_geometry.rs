//! Frame-local affine/clip bindings. Ordinary glyph instances retain their stride.
use super::*;
use std::hash::Hasher;
use lumine::{affine::Affine, scene::PaintItem};

#[repr(C)]
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct Parameters {
    matrix: [[f32; 4]; 2],
    inverse: [[f32; 4]; 2],
    fade_inverse: [[f32; 4]; 2],
    bounds: [f32; 4],
    // Clip offset/count, device scale, enabled.
    metadata: [u32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct Clip {
    inverse: [[f32; 4]; 2],
    bounds: [f32; 4],
    axes: [u32; 4],
    corners: [f32; 4],
}
fn rows(m: Affine) -> [[f32; 4]; 2] {
    [[m.a, m.c, m.tx, 0.], [m.b, m.d, m.ty, 0.]]
}
fn rect(r: Rect) -> [f32; 4] {
    [r.x, r.y, r.width, r.height]
}
pub(super) fn layout(device: &wgpu::Device, indexed: bool) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("paint geometry"),
        entries: &{
            let mut entries = vec![
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<Parameters>() as u64
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ];
            if indexed {
                for binding in [2, 3] {
                    entries.push(wgpu::BindGroupLayoutEntry {
                        binding,
                        visibility: if binding == 2 {
                            wgpu::ShaderStages::VERTEX_FRAGMENT
                        } else {
                            wgpu::ShaderStages::VERTEX
                        },
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    });
                }
            }
            entries
        },
    })
}
pub(super) struct FrameGeometry {
    uniforms: wgpu::Buffer,
    clips: wgpu::Buffer,
    pub bind: wgpu::BindGroup,
    stride: usize,
    uniform_capacity: usize,
    clip_capacity: usize,
    parameters: Vec<u8>,
    regions: Vec<Clip>,
    identity_uploaded: bool,
    pub indexed_bind: Option<wgpu::BindGroup>,
    instances: Option<wgpu::Buffer>,
    instance_capacity: usize,
    indices: Vec<u32>,
    last: Option<(Parameters, std::ops::Range<usize>, u32, u64)>,
}
impl FrameGeometry {
    pub fn bytes(&self) -> usize {
        self.uniform_capacity + self.clip_capacity + self.instance_capacity
    }
    pub fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let stride = (std::mem::size_of::<Parameters>() as u32)
            .next_multiple_of(device.limits().min_uniform_buffer_offset_alignment)
            as usize;
        let uniforms = buffer(
            device,
            stride,
            "paint uniforms",
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::STORAGE,
        );
        let clips = buffer(
            device,
            std::mem::size_of::<Clip>(),
            "paint clips",
            wgpu::BufferUsages::STORAGE,
        );
        let bind = group(device, layout, &uniforms, &clips);
        Self {
            uniforms,
            clips,
            bind,
            stride,
            uniform_capacity: stride,
            clip_capacity: std::mem::size_of::<Clip>(),
            parameters: Vec::new(),
            regions: Vec::new(),
            identity_uploaded: false,
            indexed_bind: None,
            instances: None,
            instance_capacity: 0,
            indices: Vec::new(),
            last: None,
        }
    }
    pub fn begin(&mut self) {
        self.parameters.clear();
        self.regions.clear();
        self.last = None;
        self.push(Parameters {
            matrix: rows(Affine::IDENTITY),
            inverse: rows(Affine::IDENTITY),
            fade_inverse: rows(Affine::IDENTITY),
            bounds: [0.; 4],
            metadata: [0, 0, 1_f32.to_bits(), 0],
        });
    }
    fn push(&mut self, parameters: Parameters) -> u32 {
        let offset = self.parameters.len();
        self.parameters
            .extend_from_slice(bytemuck::bytes_of(&parameters));
        self.parameters.resize(offset + self.stride, 0);
        offset as u32
    }
    pub(super) fn clip_count(&self, offset: u32) -> u32 {
        let bytes = &self.parameters[offset as usize..][..std::mem::size_of::<Parameters>()];
        bytemuck::pod_read_unaligned::<Parameters>(bytes).metadata[1]
    }
    pub fn item(&mut self, item: &PaintItem<'_>, scale: f32) -> Result<(u32, u64), GpuError> {
        if item.transform == Affine::IDENTITY
            && item.clip_regions.is_empty()
            && item.fade_transform == Affine::IDENTITY
        {
            return Ok((0, 0));
        }
        let start = self.regions.len();
        for region in item.clip_regions.iter() {
            self.regions.push(Clip {
                inverse: rows(region.inverse),
                bounds: rect(region.bounds),
                axes: [u32::from(region.axes[0]), u32::from(region.axes[1]), 0, 0],
                corners: region.corners,
            });
        }
        if matches!(item.kind, NodeKind::Text { .. } | NodeKind::RichText { .. }) {
            self.regions.push(Clip {
                inverse: rows(item.transform.inverse().unwrap_or_default()),
                bounds: rect(item.bounds),
                axes: [1, 1, 0, 0],
                corners: [0.; 4],
            });
        }
        if self.parameters.len() + self.stride > VERTEX_BUDGET
            || self.regions.len() * std::mem::size_of::<Clip>() > VERTEX_BUDGET
        {
            return Err(GpuError(
                "frame paint geometry exceeds 32 MiB budget".into(),
            ));
        }
        let parameters = Parameters {
            matrix: rows(if item.isolated {
                Affine::IDENTITY
            } else {
                item.transform
            }),
            inverse: rows(if item.isolated || item.effects.blur_radius > 0. {
                item.transform.inverse().unwrap_or_default()
            } else {
                Affine::IDENTITY
            }),
            fade_inverse: rows(item.fade_transform),
            bounds: if item.isolated || item.effects.blur_radius > 0. {
                rect(item.bounds)
            } else {
                [0.; 4]
            },
            metadata: [
                start as u32,
                (self.regions.len() - start) as u32,
                scale.to_bits(),
                if item.isolated { 2 } else { 1 },
            ],
        };
        // Frame placement of a clip slice is not visual state.
        let mut stable = parameters;
        stable.metadata[0] = 0;
        // Sibling primitives under one animated container can still batch.
        if let Some((previous, range, offset, hash)) = &self.last
            && *previous == stable
            && self.regions[range.clone()] == self.regions[start..]
        {
            let result = (*offset, *hash);
            self.regions.truncate(start);
            return Ok(result);
        }
        let mut hash = rustc_hash::FxHasher::default();
        hash.write(bytemuck::bytes_of(&stable));
        hash.write(bytemuck::cast_slice(&self.regions[start..]));
        let hash = hash.finish();
        let offset = self.push(parameters);
        self.last = Some((stable, start..self.regions.len(), offset, hash));
        Ok((offset, hash))
    }
    /// Keep the original padded parameters and blur bindings. Indices address
    /// 128-byte storage records, including the padding between uniform records.
    pub fn upload_instances(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        draws: &[Draw],
        count: usize,
        belt: &mut wgpu::util::StagingBelt,
        encoder: &mut wgpu::CommandEncoder,
    ) -> usize {
        instance_indices(&mut self.indices, draws, count);
        let bytes = bytemuck::cast_slice(&self.indices);
        let mut allocations = 0;
        if bytes.len() > self.instance_capacity {
            self.instance_capacity = bytes.len().next_power_of_two();
            self.instances = Some(buffer(
                device,
                self.instance_capacity,
                "paint geometry indices",
                wgpu::BufferUsages::STORAGE,
            ));
            self.indexed_bind = None;
            allocations += 1;
        }
        if self.indexed_bind.is_none() {
            self.indexed_bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("indexed frame paint geometry"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.uniforms,
                            offset: 0,
                            size: wgpu::BufferSize::new(std::mem::size_of::<Parameters>() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.clips.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.instances.as_ref().unwrap().as_entire_binding(),
                    },
                ],
            }));
        }
        belt.write_buffer(
            encoder,
            self.instances.as_ref().unwrap(),
            0,
            wgpu::BufferSize::new(bytes.len() as u64).expect("background index"),
        )
        .copy_from_slice(bytes);
        allocations
    }
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        belt: &mut wgpu::util::StagingBelt,
        encoder: &mut wgpu::CommandEncoder,
    ) -> usize {
        if self.parameters.len() == self.stride && self.identity_uploaded {
            return 0;
        }
        let needed = self.parameters.len();
        let clip_bytes = bytemuck::cast_slice(&self.regions);
        let mut resized = false;
        let mut allocations = 0;
        if needed > self.uniform_capacity {
            self.uniform_capacity = needed.next_power_of_two();
            self.uniforms = buffer(
                device,
                self.uniform_capacity,
                "paint uniforms",
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::STORAGE,
            );
            resized = true;
            allocations += 1;
        }
        if clip_bytes.len() > self.clip_capacity {
            self.clip_capacity = clip_bytes.len().next_power_of_two();
            self.clips = buffer(
                device,
                self.clip_capacity,
                "paint clips",
                wgpu::BufferUsages::STORAGE,
            );
            resized = true;
            allocations += 1;
        }
        if resized {
            self.indexed_bind = None;
            self.bind = group(device, layout, &self.uniforms, &self.clips);
        }
        belt.write_buffer(
            encoder,
            &self.uniforms,
            0,
            wgpu::BufferSize::new(needed as u64).unwrap(),
        )
        .copy_from_slice(&self.parameters);
        if !clip_bytes.is_empty() {
            belt.write_buffer(
                encoder,
                &self.clips,
                0,
                wgpu::BufferSize::new(clip_bytes.len() as u64).unwrap(),
            )
            .copy_from_slice(clip_bytes);
        }
        self.identity_uploaded = true;
        allocations
    }
}
fn buffer(
    device: &wgpu::Device,
    size: usize,
    label: &str,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size as u64,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
fn group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    clips: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("frame paint geometry"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: uniforms,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<Parameters>() as u64),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: clips.as_entire_binding(),
            },
        ],
    })
}

pub(super) fn instance_indices(indices: &mut Vec<u32>, draws: &[Draw], count: usize) {
    indices.clear();
    indices.resize(count, 0);
    for draw in draws {
        assert_eq!(
            draw.geometry_offset as usize % std::mem::size_of::<Parameters>(),
            0
        );
        indices[draw.start as usize..draw.end as usize]
            .fill(draw.geometry_offset / std::mem::size_of::<Parameters>() as u32);
    }
}

/// Generate only the opt-in shader. The reference WGSL stays byte-for-byte intact.
pub(super) fn shader(indexed: bool) -> String {
    let geometry = include_str!("paint_geometry.wgsl").replace("GEOMETRY_GROUP", "2");
    let draw = include_str!("draw.wgsl");
    if !indexed {
        return geometry + draw;
    }
    // Flat geometry is constant within each primitive, like its old uniform.
    // Storage lookup makes Naga conservatively reject derivatives in clip loops.
    let geometry = geometry.replace(
        "@group(2) @binding(0) var<uniform> geometry:PaintGeometry;",
        "var<private> geometry:PaintGeometry;\n\
         @group(2) @binding(2) var<storage,read> geometries:array<PaintGeometry>;\n\
         @group(2) @binding(3) var<storage,read> geometry_indices:array<u32>;",
    );
    let draw = draw
        .replace("@location(9) @interpolate(flat) knockout:vec4<f32>",
            "@location(9) @interpolate(flat) knockout:vec4<f32>, @location(10) @interpolate(flat) geometry_index:u32")
        .replace("@vertex fn vs(v:Vertex,@builtin(vertex_index) i:u32)->Out {",
            "@vertex fn vs(v:Vertex,@builtin(vertex_index) i:u32,@builtin(instance_index) instance:u32)->Out {\n geometry=geometries[geometry_indices[instance]];")
        .replace("o.knockout=v.uv;return o;", "o.knockout=v.uv;o.geometry_index=geometry_indices[instance];return o;")
        .replace("fn shade(v:Out)->vec4<f32> {", "fn shade(v:Out)->vec4<f32> {\n geometry=geometries[v.geometry_index];");
    "diagnostic(off, derivative_uniformity);\n".to_owned() + &geometry + &draw
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexed_shader_changes_only_geometry_transport() {
        let reference = shader(false);
        assert_eq!(
            reference,
            include_str!("paint_geometry.wgsl").replace("GEOMETRY_GROUP", "2")
                + include_str!("draw.wgsl")
        );
        let indexed = shader(true);
        assert!(!indexed.contains("var<uniform> geometry:PaintGeometry"));
        for required in [
            "@builtin(instance_index) instance:u32",
            "@location(10) @interpolate(flat) geometry_index:u32",
            "geometry=geometries[geometry_indices[instance]];",
            "o.geometry_index=geometry_indices[instance];",
            "geometry=geometries[v.geometry_index];",
        ] {
            assert_eq!(indexed.matches(required).count(), 1, "{required}");
        }
        assert_eq!(std::mem::size_of::<Parameters>(), 128);
    }
}
