//! Immutable SVG source. Parsing and device-scale rasterization belong to the renderer.
use crate::{
    affine::Affine,
    scene::{Color, Rect},
};
use std::sync::Arc;
#[derive(Clone, Debug)]
pub struct SvgData {
    source_id: u64,
    bytes: Arc<[u8]>,
    tint: Option<Color>,
    transform: Affine,
    viewport_ink: Rect,
    overflow: [bool; 2],
}
impl SvgData {
    /// Bounded source storage; XML validity is checked by the renderer. SVG text
    /// and external resources follow the renderer's restricted SVG asset policy.
    pub fn new(bytes: impl Into<Arc<[u8]>>) -> Result<Self, &'static str> {
        let bytes = bytes.into();
        if bytes.is_empty() || bytes.len() > 4 * 1024 * 1024 {
            return Err("SVG source requires 1 byte..=4 MiB");
        }
        Ok(Self {
            source_id: crate::image::next_identity()?,
            bytes,
            tint: None,
            transform: Affine::IDENTITY,
            viewport_ink: Rect::new(0., 0., 1., 1.),
            overflow: [false; 2],
        })
    }
    /// Immutable source identity, preserved by tint and affine refinements.
    pub fn source_id(&self) -> u64 {
        self.source_id
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn tint(&self) -> Option<Color> {
        self.tint
    }
    /// Renderer-measured ink in fractions of the original SVG viewport.
    /// This changes paint allocation only, never layout or hit-test geometry.
    pub fn with_overflow(mut self, ink: Rect, axes: [bool; 2]) -> Result<Self, &'static str> {
        if ![ink.x, ink.y, ink.width, ink.height, ink.x + ink.width, ink.y + ink.height]
            .iter().all(|value| value.is_finite()) || ink.width < 0. || ink.height < 0.
        {
            return Err("invalid SVG ink bounds");
        }
        self.viewport_ink = ink;
        self.overflow = axes;
        Ok(self)
    }
    pub fn overflow(&self) -> [bool; 2] {
        self.overflow
    }
    /// Untransformed raster allocation, retaining the original viewport origin.
    pub fn raster_bounds(&self, bounds: Rect) -> Rect {
        let ink = self.viewport_ink;
        let left = if self.overflow[0] { ink.x.min(0.) } else { 0. };
        let top = if self.overflow[1] { ink.y.min(0.) } else { 0. };
        let right = if self.overflow[0] { (ink.x + ink.width).max(1.) } else { 1. };
        let bottom = if self.overflow[1] { (ink.y + ink.height).max(1.) } else { 1. };
        Rect::new(bounds.x + left * bounds.width, bounds.y + top * bounds.height,
            (right - left) * bounds.width, (bottom - top) * bounds.height)
    }
    pub fn tinted(&self, color: Color) -> Self {
        Self {
            tint: Some(color),
            ..self.clone()
        }
    }
    pub fn transformed(&self, transform: Affine) -> Self {
        Self {
            transform: if transform.is_finite() {
                transform
            } else {
                Affine::IDENTITY
            },
            ..self.clone()
        }
    }
    pub fn transform(&self) -> Affine {
        self.transform
    }
    pub fn same_pixels(&self, other: &Self) -> bool {
        self.bytes == other.bytes && self.tint == other.tint
            && self.viewport_ink == other.viewport_ink && self.overflow == other.overflow
    }
    pub fn paint_transform(&self, bounds: Rect) -> Affine {
        if self.transform == Affine::IDENTITY {
            self.transform
        } else {
            self.transform.around(
                bounds.x + bounds.width * 0.5,
                bounds.y + bounds.height * 0.5,
            )
        }
    }
    pub fn paint_bounds(&self, bounds: Rect) -> Rect {
        if self.transform.inverse().is_none() {
            Rect::default()
        } else {
            self.paint_transform(bounds).bounds(self.raster_bounds(bounds))
        }
    }
}

impl PartialEq for SvgData {
    fn eq(&self, other: &Self) -> bool {
        self.same_pixels(other) && self.transform == other.transform
    }
}
