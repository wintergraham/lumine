//! Bounded image decoding and SVG rasterization, independent of a GPU device.
use crate::GpuError;
use image::ImageDecoder;
use std::{io::Cursor, sync::Arc};
use lumine::image::ImageData;
const MAX_BYTES: usize = 64 * 1024 * 1024;
/// Decode PNG/JPEG into immutable, straight-alpha RGBA8. No filesystem/network access.
pub fn decode_image(bytes: &[u8]) -> Result<Arc<ImageData>, GpuError> {
    if bytes.len() > MAX_BYTES {
        return Err(GpuError("encoded image exceeds 64 MiB".into()));
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| GpuError(e.to_string()))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(MAX_BYTES as u64);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().map_err(|e| GpuError(e.to_string()))?;
    let (width, height) = decoder.dimensions();
    // An RGB/greyscale decoder can fit its own limit while conversion to RGBA
    // would allocate a larger, rejected output. Check before decoding pixels.
    if (u64::from(width) * u64::from(height))
        .checked_mul(4)
        .is_none_or(|bytes| bytes > MAX_BYTES as u64)
    {
        return Err(GpuError("decoded image exceeds 64 MiB".into()));
    }
    limits
        .reserve(decoder.total_bytes())
        .map_err(|e| GpuError(e.to_string()))?;
    decoder
        .set_limits(limits)
        .map_err(|e| GpuError(e.to_string()))?;
    let decoded = image::DynamicImage::from_decoder(decoder)
        .map_err(|e| GpuError(e.to_string()))?
        .into_rgba8();
    ImageData::new(decoded.width(), decoded.height(), decoded.into_raw())
        .map(Arc::new)
        .map_err(|e| GpuError(e.into()))
}
/// Rasterize an SVG at the requested physical size. External resources are disabled.
/// SVG text needs conversion to outlines; the lightweight SVG dependency disables fonts.
pub fn decode_svg(bytes: &[u8], width: u32, height: u32) -> Result<Arc<ImageData>, GpuError> {
    decode_svg_at(bytes, width, height, [width as f32, height as f32, 0., 0.])
}
/// Rasterize at the exact device-space size and fractional origin, leaving
/// transparent padding instead of scaling the rounded allocation back down.
pub(crate) fn decode_svg_at(
    bytes: &[u8],
    width: u32,
    height: u32,
    geometry: [f32; 4],
) -> Result<Arc<ImageData>, GpuError> {
    if bytes.len() > 4 * 1024 * 1024
        || width == 0
        || height == 0
        || (u64::from(width) * u64::from(height))
            .checked_mul(4)
            .is_none_or(|bytes| bytes > MAX_BYTES as u64)
    {
        return Err(GpuError("SVG input or output exceeds bounds".into()));
    }
    let mut options = resvg::usvg::Options::default();
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let tree =
        resvg::usvg::Tree::from_data(bytes, &options).map_err(|e| GpuError(e.to_string()))?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| GpuError("SVG image allocation failed".into()))?;
    let transform = resvg::tiny_skia::Transform::from_row(
        geometry[0] / tree.size().width(),
        0.,
        0.,
        geometry[1] / tree.size().height(),
        geometry[2],
        geometry[3],
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut pixels = pixmap.take();
    for p in pixels.as_chunks_mut::<4>().0 {
        let alpha = p[3] as u32;
        if alpha > 0 {
            for c in &mut p[..3] {
                *c = (u32::from(*c) * 255 + alpha / 2)
                    .checked_div(alpha)
                    .unwrap_or(0)
                    .min(255) as u8;
            }
        }
    }
    ImageData::new(width, height, pixels)
        .map(Arc::new)
        .map_err(|e| GpuError(e.into()))
}

/// Decode GIF frames once, compositing offsets and Keep/Background/Previous
/// disposal into full straight-alpha frames. Background disposal is transparent,
/// matching GUI image compositing rather than filling the window with GIF palette
/// background. Absent looping extension plays once; finite repetitions add to the
/// initial play. Zero-delay GIF frames use 100 ms; other delays floor at 10 ms.
pub fn decode_gif(bytes: &[u8]) -> Result<Arc<lumine::animation::Animation>, GpuError> {
    use lumine::animation::{Animation, Frame, LoopCount};
    if bytes.len() > MAX_BYTES {
        return Err(GpuError("encoded GIF exceeds 64 MiB".into()));
    }
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    options.check_frame_consistency(true);
    options.set_memory_limit(gif::MemoryLimit::Bytes(
        std::num::NonZeroU64::new(MAX_BYTES as u64).unwrap(),
    ));
    let mut decoder = options
        .read_info(Cursor::new(bytes))
        .map_err(|e| GpuError(e.to_string()))?;
    let (width, height) = (u32::from(decoder.width()), u32::from(decoder.height()));
    let length = u64::from(width) * u64::from(height) * 4;
    if length == 0 || length > MAX_BYTES as u64 {
        return Err(GpuError("GIF canvas exceeds 64 MiB".into()));
    }
    let mut canvas = vec![0u8; length as usize];
    let mut frames = Vec::new();
    while let Some(frame) = decoder
        .read_next_frame()
        .map_err(|e| GpuError(e.to_string()))?
    {
        if frames.len() >= 4096 || (frames.len() + 1) as u64 * length > MAX_BYTES as u64 {
            return Err(GpuError("decoded GIF exceeds frame/64 MiB budget".into()));
        }
        let previous = (frame.dispose == gif::DisposalMethod::Previous).then(|| canvas.clone());
        let (fw, fh, left, top) = (
            usize::from(frame.width),
            usize::from(frame.height),
            usize::from(frame.left),
            usize::from(frame.top),
        );
        for y in 0..fh {
            for x in 0..fw {
                let source = 4 * (y * fw + x);
                let destination = 4 * ((top + y) * width as usize + left + x);
                if frame.buffer[source + 3] > 0 {
                    canvas[destination..destination + 4]
                        .copy_from_slice(&frame.buffer[source..source + 4]);
                }
            }
        }
        let image = Arc::new(
            ImageData::new(width, height, canvas.clone()).map_err(|e| GpuError(e.into()))?,
        );
        let delay = if frame.delay == 0 {
            100
        } else {
            u64::from(frame.delay) * 10
        };
        frames.push(Frame {
            image,
            duration: std::time::Duration::from_millis(delay),
        });
        match frame.dispose {
            gif::DisposalMethod::Background => {
                for y in 0..fh {
                    let start = 4 * ((top + y) * width as usize + left);
                    canvas[start..start + 4 * fw].fill(0);
                }
            }
            gif::DisposalMethod::Previous => canvas = previous.unwrap(),
            _ => {}
        }
    }
    let loops = match decoder.repeat() {
        gif::Repeat::Infinite => LoopCount::Infinite,
        gif::Repeat::Finite(repeats) => {
            LoopCount::Finite(std::num::NonZeroU32::new(u32::from(repeats) + 1).unwrap())
        }
    };
    Animation::new(frames, loops)
        .map(Arc::new)
        .map_err(|e| GpuError(e.into()))
}
