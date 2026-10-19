//! Platform scrollbar placement, shared by native layout and offscreen hosts.
#[cfg(target_os = "macos")]
static CLASSIC: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Call once on the main thread before starting offscreen workers. macOS chooses
/// overlay or classic scrollbars from its user preference and attached devices.
pub fn reserves_scrollbar_gutters() -> bool {
    #[cfg(target_os = "macos")]
    {
        if let Some(classic) = CLASSIC.get() {
            return *classic;
        }
        let Some(_main_thread) = objc2::MainThreadMarker::new() else {
            // Headless core tests have no AppKit host; retain classic fixture layout.
            return true;
        };
        // SAFETY: NSScroller is an AppKit class and this query runs on the main thread.
        let style: isize = unsafe { objc2::msg_send![objc2::class!(NSScroller), preferredScrollerStyle] };
        return *CLASSIC.get_or_init(|| style == 0);
    }
    #[cfg(not(target_os = "macos"))]
    true
}
