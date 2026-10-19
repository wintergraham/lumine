//! GTK 3 is queried and dispatched only on the UI thread, including when CEF
//! initialized it first. No GTK object or GLib context is sent to a worker.
use super::{
    Event, Wake,
    readiness::{Readiness, Watch},
};
use libloading::Library;
use std::{
    ffi::{c_char, c_int, c_uint, c_ulong, c_void},
    marker::PhantomData,
    ptr,
    rc::Rc,
};
type P = *mut c_void;
type Callback = unsafe extern "C" fn();
macro_rules! api {
    ($($name:ident: $kind:ty;)*) => {
        // GTK registers process-global GTypes and callbacks. Its module must
        // remain loaded after this observer stops; unloading it is unsafe.
        #[allow(non_snake_case)]
        struct Api { $($name: $kind,)* _library: std::mem::ManuallyDrop<Library> }
        impl Api {
            unsafe fn load() -> Result<Self, Box<dyn std::error::Error>> {
                let library = unsafe { Library::new("libgtk-3.so.0")? };
                Ok(Self { $($name: unsafe { *library.get::<$kind>(concat!(stringify!($name), "\0").as_bytes())? },)* _library: std::mem::ManuallyDrop::new(library) })
            }
        }
    }
}
api! {
    gtk_init_check: unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char) -> c_int;
    gtk_settings_get_default: unsafe extern "C" fn() -> P;
    gtk_widget_path_new: unsafe extern "C" fn() -> P;
    gtk_widget_path_append_type: unsafe extern "C" fn(P, usize) -> c_int;
    gtk_widget_path_iter_set_object_name: unsafe extern "C" fn(P, c_int, *const c_char);
    gtk_widget_path_iter_add_class: unsafe extern "C" fn(P, c_int, *const c_char);
    gtk_widget_path_unref: unsafe extern "C" fn(P);
    gtk_style_context_new: unsafe extern "C" fn() -> P;
    gtk_style_context_set_path: unsafe extern "C" fn(P, P);
    gtk_style_context_add_provider: unsafe extern "C" fn(P, P, c_uint);
    gtk_css_provider_new: unsafe extern "C" fn() -> P;
    gtk_css_provider_load_from_data: unsafe extern "C" fn(P, *const c_char, isize, *mut P) -> c_int;
    gtk_render_background: unsafe extern "C" fn(P, P, f64, f64, f64, f64);
    g_signal_connect_data: unsafe extern "C" fn(P, *const c_char, Option<Callback>, P, Option<unsafe extern "C" fn(P, P)>, c_uint) -> c_ulong;
    g_signal_handler_disconnect: unsafe extern "C" fn(P, c_ulong);
    g_object_unref: unsafe extern "C" fn(P);
    g_main_context_default: unsafe extern "C" fn() -> P;
    g_main_context_acquire: unsafe extern "C" fn(P) -> c_int;
    g_main_context_release: unsafe extern "C" fn(P);
    g_main_context_prepare: unsafe extern "C" fn(P, *mut c_int) -> c_int;
    g_main_context_query: unsafe extern "C" fn(P, c_int, *mut c_int, *mut libc::pollfd, c_int) -> c_int;
    g_main_context_iteration: unsafe extern "C" fn(P, c_int) -> c_int;
    cairo_image_surface_create: unsafe extern "C" fn(c_int, c_int, c_int) -> P;
    cairo_surface_status: unsafe extern "C" fn(P) -> c_int;
    cairo_create: unsafe extern "C" fn(P) -> P;
    cairo_surface_flush: unsafe extern "C" fn(P);
    cairo_image_surface_get_data: unsafe extern "C" fn(P) -> *mut u8;
    cairo_image_surface_get_stride: unsafe extern "C" fn(P) -> c_int;
    cairo_destroy: unsafe extern "C" fn(P);
    cairo_surface_destroy: unsafe extern "C" fn(P);
}
pub(super) struct Gtk {
    api: Api,
    context: P,
    settings: P,
    style: P,
    provider: P,
    signals: Vec<(P, c_ulong)>,
    _callback: Box<Wake>,
    readiness: Readiness,
    _ui_thread: PhantomData<Rc<()>>,
}
unsafe extern "C" fn notify(_: P, _: P, data: P) {
    let wake = unsafe { &*data.cast::<Wake>() };
    wake(Event::GtkChanged);
}
unsafe extern "C" fn changed(_: P, data: P) {
    let wake = unsafe { &*data.cast::<Wake>() };
    wake(Event::GtkChanged);
}
impl Gtk {
    pub(super) fn new(wake: Wake) -> Result<Self, Box<dyn std::error::Error>> {
        let api = unsafe { Api::load()? };
        if unsafe { (api.gtk_init_check)(ptr::null_mut(), ptr::null_mut()) } == 0 {
            return Err("GTK cannot observe the native display".into());
        }
        let settings = unsafe { (api.gtk_settings_get_default)() };
        if settings.is_null() {
            return Err("GTK settings unavailable".into());
        }
        let readiness = Readiness::new(wake.clone())?;
        let callback = Box::new(wake);
        let data = (&*callback as *const Wake).cast_mut().cast();
        let (style, provider) = unsafe {
            // Match Chromium 154 GetStyleContextFromCss(""): window.background.chromium.
            let path = (api.gtk_widget_path_new)();
            (api.gtk_widget_path_append_type)(path, 4); // G_TYPE_NONE
            (api.gtk_widget_path_iter_set_object_name)(path, -1, c"window".as_ptr());
            (api.gtk_widget_path_iter_add_class)(path, -1, c"background".as_ptr());
            (api.gtk_widget_path_iter_add_class)(path, -1, c"chromium".as_ptr());
            let style = (api.gtk_style_context_new)();
            (api.gtk_style_context_set_path)(style, path);
            (api.gtk_widget_path_unref)(path);
            let provider = (api.gtk_css_provider_new)();
            (api.gtk_css_provider_load_from_data)(
                provider,
                c"* { border-radius: 0px; border-style: none; box-shadow: none; }".as_ptr(),
                -1,
                ptr::null_mut(),
            );
            (api.gtk_style_context_add_provider)(style, provider, c_uint::MAX);
            (style, provider)
        };
        let mut signals = Vec::new();
        for name in [
            c"notify::gtk-theme-name",
            c"notify::gtk-application-prefer-dark-theme",
        ] {
            let id = unsafe {
                (api.g_signal_connect_data)(
                    settings,
                    name.as_ptr(),
                    Some(std::mem::transmute::<unsafe extern "C" fn(P, P, P), Callback>(notify)),
                    data,
                    None,
                    0,
                )
            };
            signals.push((settings, id));
        }
        let id = unsafe {
            (api.g_signal_connect_data)(
                style,
                c"changed".as_ptr(),
                Some(std::mem::transmute::<unsafe extern "C" fn(P, P), Callback>(
                    changed,
                )),
                data,
                None,
                0,
            )
        };
        signals.push((style, id));
        let mut this = Self {
            context: unsafe { (api.g_main_context_default)() },
            api,
            settings,
            style,
            provider,
            signals,
            _callback: callback,
            readiness,
            _ui_thread: PhantomData,
        };
        this.arm();
        Ok(this)
    }
    pub(super) fn system_light(&self) -> Option<bool> {
        self.background().map(|color| is_light(color))
    }
    fn background(&self) -> Option<[u8; 3]> {
        unsafe {
            // Chromium samples 24x24, preserving images/gradients rather than
            // inferring appearance from a setting name or a solid CSS property.
            let surface = (self.api.cairo_image_surface_create)(0, 24, 24); // ARGB32
            if surface.is_null() {
                return None;
            }
            if (self.api.cairo_surface_status)(surface) != 0 {
                (self.api.cairo_surface_destroy)(surface);
                return None;
            }
            let cr = (self.api.cairo_create)(surface);
            (self.api.gtk_render_background)(self.style, cr, 0., 0., 24., 24.);
            (self.api.cairo_surface_flush)(surface);
            let data = (self.api.cairo_image_surface_get_data)(surface);
            let stride = (self.api.cairo_image_surface_get_stride)(surface) as usize;
            let mut sums = [0u64; 4];
            for y in 0..24 {
                for x in 0..24 {
                    let argb = ptr::read_unaligned(data.add(y * stride + x * 4).cast::<u32>());
                    sums[0] += ((argb >> 24) & 255) as u64;
                    sums[1] += ((argb >> 16) & 255) as u64;
                    sums[2] += ((argb >> 8) & 255) as u64;
                    sums[3] += (argb & 255) as u64;
                }
            }
            (self.api.cairo_destroy)(cr);
            (self.api.cairo_surface_destroy)(surface);
            Some(if sums[0] == 0 {
                [0, 0, 0]
            } else {
                [
                    (sums[1] * 255 / sums[0]) as u8,
                    (sums[2] * 255 / sums[0]) as u8,
                    (sums[3] * 255 / sums[0]) as u8,
                ]
            })
        }
    }
    pub(super) fn dispatch(&mut self) {
        // One native iteration per readiness notification. More pending native
        // work is rearmed through GLib's own descriptors/timeouts, never a timer
        // that polls preferences or rebuilds a UI tree.
        unsafe {
            (self.api.g_main_context_iteration)(self.context, 0);
        }
        self.arm();
    }
    fn arm(&mut self) {
        unsafe {
            if (self.api.g_main_context_acquire)(self.context) == 0 {
                return;
            }
            let mut priority = 0;
            (self.api.g_main_context_prepare)(self.context, &mut priority);
            let mut timeout = -1;
            let needed = (self.api.g_main_context_query)(
                self.context,
                priority,
                &mut timeout,
                ptr::null_mut(),
                0,
            )
            .max(0);
            let mut fds = vec![
                libc::pollfd {
                    fd: -1,
                    events: 0,
                    revents: 0
                };
                needed as usize
            ];
            let count = (self.api.g_main_context_query)(
                self.context,
                priority,
                &mut timeout,
                fds.as_mut_ptr(),
                needed,
            );
            (self.api.g_main_context_release)(self.context);
            if count >= 0 && count <= needed {
                fds.truncate(count as usize);
                self.readiness.watch(Watch { fds, timeout });
            }
        }
    }
}
impl Drop for Gtk {
    fn drop(&mut self) {
        unsafe {
            for (object, id) in &self.signals {
                (self.api.g_signal_handler_disconnect)(*object, *id);
            }
            (self.api.g_object_unref)(self.style);
            (self.api.g_object_unref)(self.provider);
        }
        // GtkSettings/default context are borrowed and owned by GTK itself.
        let _ = self.settings;
    }
}
fn is_light(rgb: [u8; 3]) -> bool {
    let linear = |v: u8| {
        let c = v as f32 / 255.;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    // Chromium color_utils::IsDark midpoint for its default GoogleGrey900.
    0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2]) >= 0.211692036
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chromium_luminance_uses_linear_rgb_not_average_or_theme_name() {
        assert!(is_light([246, 245, 244]));
        assert!(!is_light([46, 52, 54]));
        assert!(is_light([0, 255, 0]));
        assert!(is_light([255, 0, 0]));
        assert!(!is_light([254, 0, 0]));
    }
}
