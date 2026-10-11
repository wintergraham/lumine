//! Windows edit menus use the OS popup, with native keyboard/accessibility behavior.
use std::ffi::c_void;
use winit::{
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};

#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}
#[link(name = "user32")]
unsafe extern "system" {
    fn CreatePopupMenu() -> *mut c_void;
    fn DestroyMenu(menu: *mut c_void) -> i32;
    fn AppendMenuW(menu: *mut c_void, flags: u32, id: usize, label: *const u16) -> i32;
    fn TrackPopupMenuEx(
        menu: *mut c_void,
        flags: u32,
        x: i32,
        y: i32,
        owner: *mut c_void,
        params: *const c_void,
    ) -> u32;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn ClientToScreen(window: *mut c_void, point: *mut Point) -> i32;
}
struct Menu(*mut c_void);
impl Drop for Menu {
    fn drop(&mut self) {
        unsafe {
            DestroyMenu(self.0);
        }
    }
}

fn items(
    selection: bool,
    editable: bool,
    nonempty: bool,
    paste: bool,
) -> [(usize, &'static str, bool); 4] {
    [
        (1, "Cu&t\tCtrl+X", selection && editable),
        (2, "&Copy\tCtrl+C", selection),
        (3, "&Paste\tCtrl+V", paste && editable),
        (4, "Select &all\tCtrl+A", nonempty),
    ]
}

pub(crate) fn show(
    window: &Window,
    selection: bool,
    editable: bool,
    nonempty: bool,
    paste: bool,
    caret: Option<(f32, f32)>,
) -> u32 {
    let Ok(handle) = window.window_handle() else {
        return 0;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return 0;
    };
    let owner = handle.hwnd.get() as *mut c_void;
    let menu = Menu(unsafe { CreatePopupMenu() });
    if menu.0.is_null() {
        return 0;
    }
    for (id, label, enabled) in items(selection, editable, nonempty, paste) {
        if id == 4 {
            unsafe {
                AppendMenuW(menu.0, 0x800, 0, std::ptr::null());
            }
        }
        let label: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
        if unsafe { AppendMenuW(menu.0, if enabled { 0 } else { 1 }, id, label.as_ptr()) } == 0 {
            return 0;
        }
    }
    let mut point = Point { x: 0, y: 0 };
    if let Some((x, y)) = caret {
        let scale = window.scale_factor() as f32;
        point.x = (x * scale).round() as i32;
        point.y = (y * scale).round() as i32;
        unsafe {
            ClientToScreen(owner, &mut point);
        }
    } else {
        unsafe {
            GetCursorPos(&mut point);
        }
    }
    // Return the command directly; no synthetic WM_COMMAND leaks into app menus.
    unsafe {
        TrackPopupMenuEx(
            menu.0,
            0x100 | 0x80 | 0x2,
            point.x,
            point.y,
            owner,
            std::ptr::null(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edit_menu_preserves_read_only_copy_and_disables_unavailable_actions() {
        let enabled = |selection, editable, nonempty, paste| {
            items(selection, editable, nonempty, paste).map(|(_, _, enabled)| enabled)
        };
        assert_eq!(enabled(true, true, true, true), [true, true, true, true]);
        assert_eq!(enabled(true, false, true, true), [false, true, false, true]);
        assert_eq!(
            enabled(false, true, false, false),
            [false, false, false, false]
        );
        assert_eq!(
            enabled(false, true, true, false),
            [false, false, false, true]
        );
    }
}
