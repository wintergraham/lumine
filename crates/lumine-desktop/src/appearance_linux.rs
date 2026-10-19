//! Linux preference observation. GTK stays on the UI thread; workers only wait
//! for native descriptors or portal signals and wake winit through its proxy.
//! Chromium 154's OsSettingsProviderGtk prioritizes portal 1/2, then samples
//! window.background. Portal 0 is not a synonym for a light GTK theme.
use crate::SystemAppearance;
use std::sync::Arc;
#[path = "appearance_linux/gtk.rs"]
mod gtk;
#[path = "appearance_linux/portal.rs"]
mod portal;
#[path = "appearance_linux/readiness.rs"]
mod readiness;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PortalPreference {
    Pending,
    Unavailable,
    NoPreference,
    Light,
    Dark,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Event {
    DispatchGtk,
    GtkChanged,
    Portal(PortalPreference),
}
type Wake = Arc<dyn Fn(Event) + Send + Sync>;

pub(crate) struct Appearance {
    gtk: Option<gtk::Gtk>,
    _portal: Option<portal::Portal>,
    portal: PortalPreference,
    gtk_light: Option<bool>,
}
impl Appearance {
    /// Called on the same UI thread as GTK/CEF initialization, before building UI.
    pub(crate) fn new(wake: Wake) -> Self {
        let gtk = gtk::Gtk::new(wake.clone())
            .map_err(|e| eprintln!("lumine: GTK appearance unavailable: {e}"))
            .ok();
        let gtk_light = gtk.as_ref().and_then(|gtk| gtk.system_light());
        let portal = portal::Portal::new(wake)
            .map_err(|e| eprintln!("lumine: portal appearance unavailable: {e}"))
            .ok();
        Self {
            gtk,
            gtk_light,
            portal: if portal.is_some() {
                PortalPreference::Pending
            } else {
                PortalPreference::Unavailable
            },
            _portal: portal,
        }
    }
    pub(crate) fn state(&self) -> SystemAppearance {
        resolve(self.portal, self.gtk_light)
    }
    pub(crate) fn handle(&mut self, event: Event) -> SystemAppearance {
        match event {
            Event::Portal(preference) => self.portal = preference,
            Event::DispatchGtk => {
                if let Some(gtk) = &mut self.gtk {
                    gtk.dispatch();
                }
            }
            Event::GtkChanged => {
                self.gtk_light = self.gtk.as_ref().and_then(|gtk| gtk.system_light())
            }
        }
        self.state()
    }
}
fn resolve(portal: PortalPreference, gtk_light: Option<bool>) -> SystemAppearance {
    match portal {
        PortalPreference::Dark => SystemAppearance::Dark,
        PortalPreference::Light => SystemAppearance::Light,
        _ => match gtk_light {
            Some(true) => SystemAppearance::Light,
            Some(false) => SystemAppearance::Dark,
            None => match portal {
                PortalPreference::Pending => SystemAppearance::Pending,
                // CSS light includes the absence of an active preference. GTK,
                // when available, has already supplied the fallback above.
                PortalPreference::NoPreference => SystemAppearance::Light,
                _ => SystemAppearance::Unsupported,
            },
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_portal_overrides_gtk_and_zero_preserves_actual_gtk() {
        assert_eq!(
            resolve(PortalPreference::Dark, Some(true)),
            SystemAppearance::Dark
        );
        assert_eq!(
            resolve(PortalPreference::Light, Some(false)),
            SystemAppearance::Light
        );
        assert_eq!(
            resolve(PortalPreference::NoPreference, Some(false)),
            SystemAppearance::Dark
        );
        assert_eq!(
            resolve(PortalPreference::Unavailable, Some(true)),
            SystemAppearance::Light
        );
        assert_eq!(
            resolve(PortalPreference::Pending, None),
            SystemAppearance::Pending
        );
        assert_eq!(
            resolve(PortalPreference::Unavailable, None),
            SystemAppearance::Unsupported
        );
    }
}
