//! lumine patch: buffer fills and copies without a blit encoder.
//!
//! On Apple silicon the first blit encoder a process uses leaves the driver
//! holding roughly 110 MB for about eight seconds, and part of it for good
//! (measured with `footprint`: 297 MB graphics memory at launch and 65 MB at
//! rest for a small window, against 185 MB and 41 MB without blits). wgpu-core
//! issues two tiny buffer blits while creating every device. Here those
//! operations run as vertex-only render passes instead: rasterization is off
//! and each vertex writes one 32-bit word of the destination.

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBuffer, MTLCommandBuffer, MTLCommandEncoder, MTLCompileOptions, MTLDevice, MTLLibrary,
    MTLPrimitiveType, MTLRenderCommandEncoder, MTLRenderPassDescriptor, MTLRenderPipelineDescriptor,
    MTLRenderPipelineState,
};

const SOURCE: &str = "
#include <metal_stdlib>
using namespace metal;
vertex void lumine_fill(device uint* dst [[buffer(0)]], uint i [[vertex_id]]) { dst[i] = 0u; }
vertex void lumine_copy(device uint* dst [[buffer(0)]], const device uint* src [[buffer(1)]],
                      uint i [[vertex_id]]) { dst[i] = src[i]; }
";

pub(super) struct BufferWrites {
    fill: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    copy: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
}

impl BufferWrites {
    /// `None` if the pipelines cannot be built; callers then use blits.
    pub(super) fn new(device: &ProtocolObject<dyn MTLDevice>) -> Option<Self> {
        let library = device
            .newLibraryWithSource_options_error(
                &NSString::from_str(SOURCE),
                Some(&MTLCompileOptions::new()),
            )
            .ok()?;
        let pipeline = |name: &str| {
            let function = library.newFunctionWithName(&NSString::from_str(name))?;
            let descriptor = MTLRenderPipelineDescriptor::new();
            descriptor.setVertexFunction(Some(&function));
            descriptor.setRasterizationEnabled(false);
            device
                .newRenderPipelineStateWithDescriptor_error(&descriptor)
                .ok()
        };
        Some(Self {
            fill: pipeline("lumine_fill")?,
            copy: pipeline("lumine_copy")?,
        })
    }

    /// Zero `range` of `dst`, or copy `size` bytes from `src` into it, as a
    /// render pass on `cmd_buf`. Offsets and sizes must be multiples of four.
    pub(super) fn encode(
        &self,
        cmd_buf: &ProtocolObject<dyn MTLCommandBuffer>,
        dst: (&ProtocolObject<dyn MTLBuffer>, u64),
        src: Option<(&ProtocolObject<dyn MTLBuffer>, u64)>,
        size: u64,
    ) {
        let descriptor = MTLRenderPassDescriptor::new();
        descriptor.setRenderTargetWidth(1);
        descriptor.setRenderTargetHeight(1);
        descriptor.setDefaultRasterSampleCount(1);
        let encoder = cmd_buf
            .renderCommandEncoderWithDescriptor(&descriptor)
            .unwrap();
        encoder.setLabel(Some(&NSString::from_str("lumine buffer write")));
        unsafe {
            encoder.setVertexBuffer_offset_atIndex(Some(dst.0), dst.1 as usize, 0);
            if let Some((src, offset)) = src {
                encoder.setRenderPipelineState(&self.copy);
                encoder.setVertexBuffer_offset_atIndex(Some(src), offset as usize, 1);
            } else {
                encoder.setRenderPipelineState(&self.fill);
            }
            encoder.drawPrimitives_vertexStart_vertexCount(
                MTLPrimitiveType::Point,
                0,
                (size / 4) as usize,
            );
        }
        encoder.endEncoding();
    }
}
