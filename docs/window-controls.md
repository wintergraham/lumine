# Native window controls

`WindowContext::window` is a cloneable, thread-safe `WindowHandle`. Its methods queue requests onto the application thread:

```rust,no_run
# fn configure(cx: &mut lumine_desktop::WindowContext) {
let window = cx.window.clone();
window.set_title("Notes — saved");
window.set_inner_size(800., 600.); // logical client dimensions
window.set_maximized(true);
window.set_minimized(false);
window.set_visible(true);
window.request_focus();
# }
```

Title, size, visibility, and requested minimize/maximize state survive suspension and native recreation. Commands also work while a newly opened window is awaiting construction. A focus request made without a native window is delivered once after creation. Closed handles ignore subsequent requests. Updating the title invalidates the accessible window title as well.

Size requests reject zero, negative, nonfinite, or dimensions exceeding `u32::MAX`. The operating system can constrain any request. Observe `WindowContext::viewport` for actual logical client dimensions; receiving a command is not confirmation that the window manager accepted it. Native resize and scale events update the size used for recreation.

Winit currently does not implement dynamic visibility or focus requests on Wayland, and Wayland cannot programmatically unminimize. Focus requests on other platforms can also be rejected by focus-stealing prevention. Minimize/maximize recreation state records application requests; it does not promise to mirror every window-manager action.

Run the interactive example with `cargo run -p lumine-desktop --example window_controls`. On X11 with a window manager, append `-- --smoke-test` to request title, resize, minimize/restore, maximize/restore, hide/show, and close. The example checks actual viewport resize acknowledgments. External X11 property and map-state observations additionally verified minimization, maximization, restoration, hiding and showing; see `platform-validation/window-controls-x11.json`. This does not constitute native macOS or Wayland control validation.

Confirmed hidden, minimized, zero-sized or occluded surfaces skip frame preparation and GPU submission. Pending damage remains until restoration; application tasks and model updates continue. Unknown visibility is treated as drawable, so unsupported compositor queries do not blank the window. A redraw request cannot override explicit native occlusion. `hidden_updates` and `scripts/hidden_updates_smoke.py` check actual X11 unmapped states and latest restored pixels after 40 timed model updates. This does not imply that all reactive application computations stop while hidden.
