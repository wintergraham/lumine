//! Optional presentation telemetry. Counter-only damage is excluded so an idle
//! window does not report the HUD's own update rate as application FPS.
use std::{cell::Cell, rc::Rc};
use lumine::scene::Rect;

#[derive(Clone, Default)]
pub struct FrameCounter {
    enabled: Rc<Cell<bool>>,
    frames: Rc<Cell<u64>>,
    overlay_bounds: Rc<Cell<Option<Rect>>>,
}
impl FrameCounter {
    pub fn set_enabled(&self, enabled: bool) {
        if self.enabled.replace(enabled) != enabled {
            self.frames.set(0);
            self.overlay_bounds.set(None);
        }
    }
    /// Successfully submitted native presentations since enabling the counter.
    /// Excludes presentations whose damage is entirely within `overlay_bounds`.
    pub fn frames(&self) -> u64 {
        self.frames.get()
    }
    pub fn set_overlay_bounds(&self, bounds: Option<Rect>) {
        self.overlay_bounds.set(bounds);
    }
    pub(crate) fn content_damage(&self, damage: &[Rect]) -> bool {
        self.enabled.get()
            && damage.iter().any(|r| {
                self.overlay_bounds.get().is_none_or(|overlay| {
                    r.x < overlay.x
                        || r.y < overlay.y
                        || r.x + r.width > overlay.x + overlay.width
                        || r.y + r.height > overlay.y + overlay.height
                })
            })
    }
    pub(crate) fn presented(&self, content: bool) {
        if self.enabled.get() && content {
            self.frames.set(self.frames.get().saturating_add(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_presentations_and_excludes_counter_only_damage() {
        let counter = FrameCounter::default();
        let content = [Rect::new(0., 0., 1280., 900.)];
        assert!(!counter.content_damage(&content));
        counter.set_enabled(true);
        counter.set_overlay_bounds(Some(Rect::new(1000., 8., 264., 32.)));
        counter.presented(counter.content_damage(&[Rect::new(1012., 16., 150., 14.)]));
        assert_eq!(counter.frames(), 0);
        counter.presented(counter.content_damage(&content));
        counter.presented(false);
        assert_eq!(counter.frames(), 1);
        counter.set_enabled(false);
        counter.presented(true);
        assert_eq!(counter.frames(), 0);
    }
}
