//! Native, source-pinned analytic convex raster spans, not expected image data.
//! The bounded C++ subset and its license/provenance live in vendor/skia154-source-paint.
use std::{cell::RefCell, collections::HashMap, sync::Arc};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Span {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub coverage: i32,
    /// 0: SkARGB32::blitAntiH / Color32; 1: its two-pixel specialization.
    pub blend: i32,
}
unsafe extern "C" {
    fn lumine_source_skia154_rrect(
        left: f32,
        top: f32,
        width: f32,
        height: f32,
        radius: f32,
        viewport_width: i32,
        viewport_height: i32,
        output: *mut Span,
        capacity: usize,
    ) -> usize;
}
#[derive(Default)]
struct Cache {
    values: HashMap<[u32; 7], Arc<Vec<Span>>>,
    bytes: usize,
}
thread_local! { static CACHE: RefCell<Cache> = RefCell::new(Cache::default()); }

pub(super) fn spans(rect: [f32; 4], radius: f32, viewport: [u32; 2]) -> Option<Arc<Vec<Span>>> {
    if !rect.into_iter().chain([radius]).all(f32::is_finite)
        || viewport.into_iter().any(|v| v == 0 || v > 8191)
    {
        return None;
    }
    let key = [
        rect[0].to_bits(),
        rect[1].to_bits(),
        rect[2].to_bits(),
        rect[3].to_bits(),
        radius.to_bits(),
        viewport[0],
        viewport[1],
    ];
    CACHE.with(|cache| {
        if let Some(spans) = cache.borrow().values.get(&key) {
            return Some(spans.clone());
        }
        // Two calls keep the ABI allocation bounded and avoid handing C++ any
        // Rust allocator ownership. Unsupported geometry returns zero.
        let invoke = |out, capacity| unsafe {
            lumine_source_skia154_rrect(
                rect[0],
                rect[1],
                rect[2],
                rect[3],
                radius,
                viewport[0] as i32,
                viewport[1] as i32,
                out,
                capacity,
            )
        };
        let count = invoke(std::ptr::null_mut(), 0);
        if count == 0 || count > 65536 {
            return None;
        }
        let mut spans = vec![Span::default(); count];
        if invoke(spans.as_mut_ptr(), count) != count
            || spans.iter().any(|span| {
                span.x < 0
                    || span.y < 0
                    || span.width < 1
                    || span.x + span.width > viewport[0] as i32
                    || span.y >= viewport[1] as i32
                    || !(1..=255).contains(&span.coverage)
                    || !(0..=1).contains(&span.blend)
            })
        {
            return None;
        }
        // The selected convex RLE path emits each final pixel once. Preserve
        // dispatch per span, and reject any out-of-slice overlapping result.
        spans.sort_by_key(|span| (span.y, span.x));
        if spans
            .windows(2)
            .any(|pair| pair[0].y == pair[1].y && pair[0].x + pair[0].width > pair[1].x)
        {
            return None;
        }
        let spans = Arc::new(spans);
        let bytes = count * std::mem::size_of::<Span>();
        let mut cache = cache.borrow_mut();
        if cache.values.len() >= 128 || cache.bytes + bytes > 8 * 1024 * 1024 {
            cache.values.clear();
            cache.bytes = 0;
        }
        cache.bytes += bytes;
        cache.values.insert(key, spans.clone());
        Some(spans)
    })
}
