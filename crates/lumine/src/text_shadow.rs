//! Inherited glyph-ink shadows. These are paint, never layout or editor geometry.
use crate::scene::{Color, Rect, Transform};
use std::sync::Arc;

/// One CSS text shadow. The blur field is CSS radius; sigma is exactly radius/2.
/// Private finite normalized bits make inherited lists stable cache keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextShadow {
    x: u32,
    y: u32,
    blur: u32,
    color: [u8; 4],
}
impl TextShadow {
    pub fn new(x: f32, y: f32, blur_radius: f32, color: Color) -> Result<Self, &'static str> {
        if !x.is_finite()
            || !y.is_finite()
            || x.abs() > 4096.
            || y.abs() > 4096.
            || !blur_radius.is_finite()
            || !(0. ..=64.).contains(&blur_radius)
        {
            return Err(
                "text shadow requires finite offsets within +/-4096px and CSS blur in 0..=64px",
            );
        }
        let bits = |v: f32| if v == 0. { 0 } else { v.to_bits() };
        Ok(Self {
            x: bits(x),
            y: bits(y),
            blur: bits(blur_radius),
            color: [color.0, color.1, color.2, color.3],
        })
    }
    pub fn offset(self) -> Transform {
        Transform {
            x: f32::from_bits(self.x),
            y: f32::from_bits(self.y),
        }
    }
    pub fn blur_radius(self) -> f32 {
        f32::from_bits(self.blur)
    }
    pub fn sigma(self) -> f32 {
        self.blur_radius() * 0.5
    }
    pub fn color(self) -> Color {
        Color(self.color[0], self.color[1], self.color[2], self.color[3])
    }
    pub fn paint_bounds(self, ink: Rect) -> Rect {
        {
            let ink = ink.expand((3. * self.sigma()).ceil());
            let offset = self.offset();
            Rect::new(ink.x + offset.x, ink.y + offset.y, ink.width, ink.height)
        }
    }
}
/// CSS order: first shadow is topmost. The renderer paints this list in reverse.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextShadows(Option<Arc<[TextShadow]>>);
impl TextShadows {
    pub const NONE: Self = Self(None);
    pub fn new(shadows: impl IntoIterator<Item = TextShadow>) -> Result<Self, &'static str> {
        let mut values = Vec::new();
        for shadow in shadows {
            if values.len() == 8 {
                return Err("text shadow list exceeds 8 entries");
            }
            values.push(shadow);
        }
        Ok(if values.is_empty() {
            Self::NONE
        } else {
            Self(Some(values.into()))
        })
    }
    pub fn shadows(&self) -> &[TextShadow] {
        self.0.as_deref().unwrap_or(&[])
    }
    pub fn is_empty(&self) -> bool {
        self.shadows().is_empty()
    }
    pub fn storage_bytes(&self) -> usize {
        std::mem::size_of_val(self.shadows())
    }
    pub fn paint_bounds(&self, ink: Rect) -> Rect {
        self.shadows()
            .iter()
            .filter(|s| s.color().3 > 0)
            .fold(ink, |out, shadow| out.union(shadow.paint_bounds(ink)))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_bounded_paint_has_no_layout_metric() {
        let a = TextShadow::new(-0., 1., 8., Color(1, 2, 3, 230)).unwrap();
        assert_eq!(a, TextShadow::new(0., 1., 8., Color(1, 2, 3, 230)).unwrap());
        assert_eq!(a.sigma(), 4.);
        let ink = Rect::new(10., 20., 5., 7.);
        assert_eq!(a.paint_bounds(ink), Rect::new(-2., 9., 29., 31.));
        assert!(TextShadow::new(0., 0., -1., Color(0, 0, 0, 255)).is_err());
        assert!(TextShadow::new(f32::NAN, 0., 1., Color(0, 0, 0, 255)).is_err());
        assert!(TextShadows::new([a; 9]).is_err());
        assert_eq!(TextShadows::NONE.paint_bounds(ink), ink);
    }
}
