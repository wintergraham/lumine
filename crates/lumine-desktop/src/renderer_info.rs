//! Window-scoped live renderer observations. Mount precedes GPU creation, so
//! absence is explicit until an actual renderer/target is available.
use lumine::reactive::{Runtime, Signal};
use lumine_gpu::renderer_info::RendererInfo;

#[derive(Clone)]
pub struct RendererInfoService {
    observed: Signal<Option<RendererInfo>>,
}
impl RendererInfoService {
    pub(crate) fn new(runtime: &Runtime) -> Self {
        Self {
            observed: runtime.signal(None),
        }
    }
    /// A tracked read: a component effect follows creation, scale/size changes,
    /// suspension and closure. None must preserve unqualified source rendering.
    pub fn observed(&self) -> Option<RendererInfo> {
        self.observed.get()
    }
    pub fn observed_untracked(&self) -> Option<RendererInfo> {
        self.observed.with_untracked(Clone::clone)
    }
    pub(crate) fn publish(&self, info: RendererInfo) {
        self.replace(Some(info));
    }
    pub(crate) fn reset(&self) {
        self.replace(None);
    }
    fn replace(&self, next: Option<RendererInfo>) {
        if self.observed.with_untracked(|before| *before != next) {
            self.observed.set(next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumine_gpu::renderer_info::{AdapterInfo, TextureInfo};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    fn observation(size: [u32; 3], scale_factor: f32, name: &str) -> RendererInfo {
        RendererInfo {
            os: "linux",
            arch: "x86_64",
            adapter: AdapterInfo {
                name: name.into(),
                vendor: 0,
                device: 0,
                device_type: "Cpu".into(),
                backend: "Vulkan".into(),
                driver: "test".into(),
                driver_info: "test".into(),
            },
            retained_target: TextureInfo {
                format: "Bgra8Unorm".into(),
                size,
                sample_count: 1,
                mip_level_count: 1,
                dimension: "D2".into(),
                usage_bits: 0,
            },
            scale_factor,
            logical_viewport: [
                f64::from(size[0]) / f64::from(scale_factor),
                f64::from(size[1]) / f64::from(scale_factor),
            ],
            retained_color_encoding: "srgb-encoded-bytes",
            retained_alpha: "premultiplied",
            background_rgba: [0, 0, 0, 0],
            isolated_layer: false,
            surface: None,
        }
    }

    #[test]
    fn mounted_observer_follows_real_target_replacement_and_reset_without_duplicate_updates() {
        let runtime = Runtime::new();
        let service = RendererInfoService::new(&runtime);
        let view_service = service.clone();
        let history = Rc::new(RefCell::new(Vec::new()));
        let observations = history.clone();
        let _effect =
            runtime.effect(move || observations.borrow_mut().push(view_service.observed()));
        let first = observation([2880, 1800, 1], 2., "first device");
        service.publish(first.clone());
        runtime.flush();
        service.publish(first.clone());
        runtime.flush();
        let resized = observation([1600, 1000, 1], 1.25, "first device");
        service.publish(resized.clone());
        runtime.flush();
        service.reset();
        runtime.flush();
        service.reset();
        runtime.flush();
        let recreated = observation([800, 600, 1], 1., "replacement device");
        service.publish(recreated.clone());
        runtime.flush();
        assert_eq!(
            *history.borrow(),
            vec![None, Some(first), Some(resized), None, Some(recreated)]
        );
    }

    #[test]
    fn window_services_are_isolated_and_untracked_inspection_does_not_subscribe() {
        let runtime = Runtime::new();
        let first = RendererInfoService::new(&runtime);
        let second = RendererInfoService::new(&runtime);
        let inspected = first.clone();
        let reads = Rc::new(Cell::new(0));
        let count = reads.clone();
        let _effect = runtime.effect(move || {
            let _ = inspected.observed_untracked();
            count.set(count.get() + 1);
        });
        first.publish(observation([100, 200, 1], 1., "one"));
        runtime.flush();
        assert_eq!(reads.get(), 1);
        assert!(second.observed().is_none());
        first.reset();
        assert!(first.observed().is_none());
    }
}
