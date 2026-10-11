//! Shared macOS glass chrome for examples: inset traffic lights over a
//! transparent titlebar and a rounded, behind-window blur.
use objc2::{
    MainThreadMarker, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject},
    sel,
};
use objc2_app_kit::{
    NSApplication, NSAutoresizingMaskOptions, NSWindow, NSWindowStyleMask, NSWindowTitleVisibility,
};
use objc2_foundation::NSString;

/// `hiddenInset` chrome: transparent titlebar, hidden title, content under
/// the traffic lights, a compact unified toolbar to centre them in a 40pt
/// strip, and a blurred desktop behind the translucent window.
pub fn inset_titlebar(title: &str) -> bool {
    let mtm = MainThreadMarker::new().expect("window setup runs on the UI thread");
    let app = NSApplication::sharedApplication(mtm);
    let Some(window) = app
        .windows()
        .into_iter()
        .find(|window| window.title().to_string() == title)
    else {
        eprintln!("glass: native window {title:?} not found");
        return false;
    };
    style(&window);
    true
}

fn style(window: &NSWindow) {
    window.setStyleMask(window.styleMask() | NSWindowStyleMask::FullSizeContentView);
    window.setTitlebarAppearsTransparent(true);
    window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    unsafe {
        // NSTitlebarSeparatorStyleNone
        let _: () = msg_send![window, setTitlebarSeparatorStyle: 1_isize];
        if let Some(class) = AnyClass::get(c"NSToolbar") {
            let id = NSString::from_str("lumine.glass.titlebar");
            let toolbar: Retained<AnyObject> =
                msg_send![msg_send![class, alloc], initWithIdentifier: &*id];
            let _: () = msg_send![&*toolbar, setShowsBaselineSeparator: false];
            let _: () = msg_send![window, setToolbar: &*toolbar];
            // NSWindowToolbarStyleUnifiedCompact
            let _: () = msg_send![window, setToolbarStyle: 4_isize];
        }
        frost(window);
    }
}

/// Put a behind-window `NSVisualEffectView` under the (transparent) lumine
/// view. Unlike the private CGS window blur, it can be clipped to the
/// window's rounded shape, so neither blur nor shadow is left square.
unsafe fn frost(window: &NSWindow) {
    let Some(content) = window.contentView() else {
        return;
    };
    let Some(frame) = (unsafe { content.superview() }) else {
        return;
    };
    let Some(class) = AnyClass::get(c"NSVisualEffectView") else {
        return;
    };
    // The corner radius varies by macOS version and toolbar style.
    let radius: f64 = if window.class().responds_to(sel!(_cornerRadius)) {
        unsafe { msg_send![window, _cornerRadius] }
    } else {
        10.
    };
    unsafe {
        let effect: Retained<objc2_app_kit::NSView> =
            msg_send![msg_send![class, alloc], initWithFrame: frame.bounds()];
        effect.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        // HUD material, behind-window blending, always active.
        let _: () = msg_send![&*effect, setMaterial: 13_isize];
        let _: () = msg_send![&*effect, setBlendingMode: 0_isize];
        let _: () = msg_send![&*effect, setState: 1_isize];
        effect.setWantsLayer(true);
        let layer: *mut AnyObject = msg_send![&*effect, layer];
        if let Some(layer) = layer.as_ref() {
            let continuous = NSString::from_str("continuous");
            let _: () = msg_send![layer, setCornerRadius: radius];
            let _: () = msg_send![layer, setCornerCurve: &*continuous];
            let _: () = msg_send![layer, setMasksToBounds: true];
        }
        // NSWindowBelow
        let _: () = msg_send![
            &*frame,
            addSubview: &*effect,
            positioned: -1_isize,
            relativeTo: &*content
        ];
    }
    window.invalidateShadow();
}
