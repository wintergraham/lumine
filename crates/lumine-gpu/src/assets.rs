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
    decode_svg_at(bytes, width, height, [width as f32, height as f32, 0., 0.], [false; 2])
}
/// Overflow on a collapsed raw SVG belongs to its SVG root, not its HTML holder.
/// Raw style sheets require source-side expansion before native import.
pub fn svg_root_overflow(bytes: &[u8]) -> Result<[bool; 2], GpuError> {
    if bytes.len() > 4 * 1024 * 1024 { return Err(GpuError("SVG source exceeds limit".into())); }
    let text = std::str::from_utf8(bytes).map_err(|e| GpuError(e.to_string()))?;
    let document = resvg::usvg::roxmltree::Document::parse(text).map_err(|e| GpuError(e.to_string()))?;
    let root = document.root_element();
    if document.descendants().any(|node| node.has_tag_name("style")) {
        return Err(GpuError("Expand raw SVG style sheets before native import".into()));
    }
    let mut shorthand = root.attribute("overflow").unwrap_or("hidden");
    let mut axes = [None; 2];
    for declaration in root.attribute("style").unwrap_or("").split(';') {
        let Some((key, value)) = declaration.split_once(':') else { continue; };
        match key.trim().to_ascii_lowercase().as_str() {
            "overflow" => { shorthand = value.trim(); axes = [None; 2]; },
            "overflow-x" => axes[0] = Some(value.trim()),
            "overflow-y" => axes[1] = Some(value.trim()),
            _ => {},
        }
    }
    let mut values = shorthand.split_whitespace();
    let x = values.next().unwrap_or("hidden");
    let y = values.next().unwrap_or(x);
    if values.next().is_some() { return Err(GpuError("Unsupported raw SVG overflow value".into())); }
    let axes = [axes[0].unwrap_or(x), axes[1].unwrap_or(y)];
    if axes.iter().any(|axis| !matches!(*axis, "visible" | "hidden" | "clip" | "scroll" | "auto")) {
        return Err(GpuError("Resolve raw SVG overflow values before native import".into()));
    }
    Ok([axes[0] == "visible" && matches!(axes[1], "visible" | "clip"),
        axes[1] == "visible" && matches!(axes[0], "visible" | "clip")])
}
/// Inline SVG paint bounds in its original viewport, including strokes and filters.
/// Root overflow is supplied by the host's computed CSS; ancestor clips stay in Scene.
pub fn svg_with_overflow(
    bytes: impl Into<Arc<[u8]>>,
    overflow: [bool; 2],
) -> Result<lumine::svg::SvgData, GpuError> {
    let source = lumine::svg::SvgData::new(bytes).map_err(|e| GpuError(e.into()))?;
    let mut options = resvg::usvg::Options::default();
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let tree = resvg::usvg::Tree::from_data(source.bytes(), &options)
        .map_err(|e| GpuError(e.to_string()))?;
    let ink = tree.root().abs_layer_bounding_box();
    let size = tree.size();
    source.with_overflow(lumine::scene::Rect::new(
        ink.x() / size.width(), ink.y() / size.height(),
        ink.width() / size.width(), ink.height() / size.height(),
    ), overflow).map_err(|e| GpuError(e.into()))
}
/// Decode an uploaded SVG at its intrinsic aspect ratio with the same bounds.
pub fn decode_svg_natural(bytes: &[u8]) -> Result<Arc<ImageData>, GpuError> {
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(GpuError("SVG input exceeds bounds".into()));
    }
    let mut options = resvg::usvg::Options::default();
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let tree =
        resvg::usvg::Tree::from_data(bytes, &options).map_err(|e| GpuError(e.to_string()))?;
    let width = (tree.size().width() * 2.).ceil().max(1.) as u32;
    let height = (tree.size().height() * 2.).ceil().max(1.) as u32;
    decode_svg(bytes, width, height)
}
#[cfg(test)]
#[test]
fn uploaded_svg_preserves_aspect_ratio_and_rejects_invalid_input() {
    let image=decode_svg_natural(br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="red"/></svg>"#).unwrap();
    assert_eq!(image.width(),image.height()*2);
    assert_eq!(&image.pixels()[..4],&[255,0,0,255]);
    assert!(decode_svg_natural(b"not an image").is_err());
}
/// Rasterize at the exact device-space size and fractional origin, leaving
/// transparent padding instead of scaling the rounded allocation back down.
pub(crate) fn decode_svg_at(
    bytes: &[u8],
    width: u32,
    height: u32,
    geometry: [f32; 4],
    overflow: [bool; 2],
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
    // Device-grid padding must not expose ink on a clipped viewport axis.
    if overflow != [true; 2] {
        for (index, pixel) in pixmap.data_mut().chunks_exact_mut(4).enumerate() {
            let x = (index % width as usize) as f32;
            let y = (index / width as usize) as f32;
            let coverage = |position: f32, origin: f32, extent: f32| {
                ((position + 1.).min(origin + extent) - position.max(origin)).clamp(0., 1.)
            };
            let alpha = if overflow[0] { 1. } else { coverage(x, geometry[2], geometry[0]) }
                * if overflow[1] { 1. } else { coverage(y, geometry[3], geometry[1]) };
            for channel in pixel {
                *channel = (f32::from(*channel) * alpha).round() as u8;
            }
        }
    }
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
