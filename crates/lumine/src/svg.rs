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
            self.paint_transform(bounds).bounds(bounds)
        }
    }
}

impl PartialEq for SvgData {
    fn eq(&self, other: &Self) -> bool {
        self.same_pixels(other) && self.transform == other.transform
    }
}
