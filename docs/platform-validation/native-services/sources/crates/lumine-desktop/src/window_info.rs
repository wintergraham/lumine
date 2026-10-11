//! Native geometry snapshots and explicit platform capability reporting.
use winit::{
    event_loop::ActiveEventLoop,
    monitor::MonitorHandle,
    raw_window_handle::{HasDisplayHandle, RawDisplayHandle},
    window::{Window, WindowAttributes},
};
use lumine::{reactive::Signal, widgets::Ui};
/// Physical desktop coordinates and physical client-area size. Position is None
/// when the compositor does not expose global window placement (notably Wayland).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowBounds {
    pub position: Option<(i32, i32)>,
    pub size: (u32, u32),
}
impl WindowBounds {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            position: None,
            size: (width, height),
        }
    }
    pub fn at(mut self, x: i32, y: i32) -> Self {
        self.position = Some((x, y));
        self
    }
    pub(crate) fn read(window: &Window) -> Self {
        let size = window.inner_size();
        Self {
            position: window.outer_position().ok().map(|p| (p.x, p.y)),
            size: (size.width, size.height),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplaySelector {
    Primary,
    Index(usize),
    Name(String),
}
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayInfo {
    pub index: usize,
    pub name: Option<String>,
    pub position: (i32, i32),
    pub size: (u32, u32),
    pub scale_factor: f64,
    pub primary: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowCapabilities {
    pub global_position: bool,
    pub placement: bool,
    pub interactive_drag: bool,
    pub native_fullscreen: bool,
}
#[derive(Clone)]
pub struct WindowInfo {
    pub ready: Signal<bool>,
    pub bounds: Signal<WindowBounds>,
    pub fullscreen: Signal<bool>,
    pub displays: Signal<Vec<DisplayInfo>>,
    pub current_display: Signal<Option<usize>>,
    pub capabilities: Signal<WindowCapabilities>,
    /// The most recent rejected explicit placement/drag request, if any.
    pub last_request_error: Signal<Option<String>>,
}
impl WindowInfo {
    pub(crate) fn new(ui: &Ui, width: f64, height: f64) -> Self {
        Self {
            ready: ui.signal(false),
            bounds: ui.signal(WindowBounds::new(
                width.max(1.) as u32,
                height.max(1.) as u32,
            )),
            fullscreen: ui.signal(false),
            displays: ui.signal(Vec::new()),
            current_display: ui.signal(None),
            capabilities: ui.signal(WindowCapabilities::default()),
            last_request_error: ui.signal(None),
        }
    }
    pub(crate) fn refresh(&self, window: &Window) {
        let wayland = window
            .display_handle()
            .is_ok_and(|h| matches!(h.as_raw(), RawDisplayHandle::Wayland(_)));
        let primary = window.primary_monitor();
        let current = window.current_monitor();
        let monitors = window.available_monitors().collect::<Vec<_>>();
        self.current_display.set(
            current
                .as_ref()
                .and_then(|m| monitors.iter().position(|candidate| candidate == m)),
        );
        self.displays.set(
            monitors
                .iter()
                .enumerate()
                .map(|(index, m)| {
                    let p = m.position();
                    let s = m.size();
                    DisplayInfo {
                        index,
                        name: m.name(),
                        position: (p.x, p.y),
                        size: (s.width, s.height),
                        scale_factor: m.scale_factor(),
                        primary: primary.as_ref() == Some(m),
                    }
                })
                .collect(),
        );
        self.bounds.set(WindowBounds::read(window));
        self.fullscreen.set(window.fullscreen().is_some());
        self.capabilities.set(WindowCapabilities {
            global_position: !wayland,
            placement: !wayland,
            interactive_drag: true,
            native_fullscreen: true,
        });
        self.ready.set(true);
    }
}
pub(crate) fn select_monitor(
    event_loop: &ActiveEventLoop,
    selector: &DisplaySelector,
) -> Option<MonitorHandle> {
    match selector {
        DisplaySelector::Primary => event_loop
            .primary_monitor()
            .or_else(|| event_loop.available_monitors().next()),
        DisplaySelector::Index(index) => event_loop.available_monitors().nth(*index),
        DisplaySelector::Name(name) => event_loop
            .available_monitors()
            .find(|m| m.name().as_ref() == Some(name)),
    }
}
pub(crate) fn attributes(
    options: &crate::WindowOptions,
    event_loop: &ActiveEventLoop,
) -> Result<WindowAttributes, String> {
    let mut attrs = Window::default_attributes()
        .with_title(&options.title)
        .with_inner_size(winit::dpi::LogicalSize::new(options.width, options.height))
        .with_resizable(options.resizable)
        .with_transparent(options.transparent)
        .with_decorations(options.decorations)
        .with_visible(false);
    if let Some((w, h)) = options.min_size {
        if !w.is_finite() || !h.is_finite() || w <= 0. || h <= 0. {
            return Err("minimum window dimensions must be finite and positive".into());
        }
        attrs = attrs.with_min_inner_size(winit::dpi::LogicalSize::new(w, h));
    }
    if let Some(selector) = &options.display {
        let monitor = select_monitor(event_loop, selector)
            .ok_or_else(|| "requested display is unavailable".to_owned())?;
        attrs = attrs.with_position(monitor.position());
    }
    if let Some(bounds) = options.bounds {
        if bounds.size.0 == 0 || bounds.size.1 == 0 {
            return Err("window bounds must have positive size".into());
        }
        attrs = attrs.with_inner_size(winit::dpi::PhysicalSize::new(bounds.size.0, bounds.size.1));
        if let Some((x, y)) = bounds.position {
            attrs = attrs.with_position(winit::dpi::PhysicalPosition::new(x, y));
        }
    }
    Ok(attrs)
}
