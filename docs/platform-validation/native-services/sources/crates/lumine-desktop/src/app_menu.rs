//! One menu model for macOS's application menu and an accessible Linux menu bar.
use std::{collections::HashSet, fmt, rc::Rc};
use lumine::compose::{self, Context, View};
use lumine::style::Styled;

/// Stable application-defined command identifier shared by native and rendered menus.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MenuAction(pub String);
impl From<&str> for MenuAction {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}
impl From<String> for MenuAction {
    fn from(value: String) -> Self {
        Self(value)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppMenu {
    pub label: String,
    pub entries: Vec<AppMenuEntry>,
}
impl AppMenu {
    pub fn new(label: impl Into<String>, entries: impl IntoIterator<Item = AppMenuEntry>) -> Self {
        Self {
            label: label.into(),
            entries: entries.into_iter().collect(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppMenuEntry {
    Item {
        label: String,
        action: MenuAction,
        enabled: bool,
    },
    Submenu(AppMenu),
    Separator,
}
impl AppMenuEntry {
    pub fn item(label: impl Into<String>, action: impl Into<MenuAction>) -> Self {
        Self::Item {
            label: label.into(),
            action: action.into(),
            enabled: true,
        }
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        if let Self::Item { enabled: value, .. } = &mut self {
            *value = enabled;
        }
        self
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppMenuError(pub String);
impl fmt::Display for AppMenuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for AppMenuError {}
/// macOS exposes the application menu in the system menu bar. Linux windows use
/// [`app_menu_bar`], as winit has no GTK menubar host or universal Wayland app menu.
pub fn has_native_app_menu() -> bool {
    cfg!(target_os = "macos")
}

pub(crate) fn validate(menus: &[AppMenu]) -> Result<(), AppMenuError> {
    fn visit(menus: &[AppMenu], ids: &mut HashSet<String>) -> Result<(), AppMenuError> {
        for menu in menus {
            if menu.label.is_empty() || menu.label.contains('\0') {
                return Err(AppMenuError(
                    "menu labels must be nonempty and contain no NUL".into(),
                ));
            }
            for entry in &menu.entries {
                match entry {
                    AppMenuEntry::Item { label, action, .. } => {
                        if label.is_empty()
                            || label.contains('\0')
                            || action.0.is_empty()
                            || action.0.contains('\0')
                            || !ids.insert(action.0.clone())
                        {
                            return Err(AppMenuError("menu items need valid labels and unique nonempty action identifiers".into()));
                        }
                    }
                    AppMenuEntry::Submenu(menu) => visit(std::slice::from_ref(menu), ids)?,
                    AppMenuEntry::Separator => {}
                }
            }
        }
        Ok(())
    }
    visit(menus, &mut HashSet::new())
}
pub(crate) fn enabled_action(menus: &[AppMenu], action: &MenuAction) -> bool {
    menus
        .iter()
        .flat_map(|m| &m.entries)
        .any(|entry| match entry {
            AppMenuEntry::Item {
                action: id,
                enabled,
                ..
            } => *enabled && id == action,
            AppMenuEntry::Submenu(menu) => enabled_action(std::slice::from_ref(menu), action),
            AppMenuEntry::Separator => false,
        })
}
/// Accessible retained menus with keyboard traversal, Escape dismissal, focus
/// restoration, and nested submenus. Mount inside a Linux window's content layout.
/// The same model can also be rendered explicitly on any platform.
pub fn app_menu_bar(
    menus: Vec<AppMenu>,
    dispatch: impl Fn(MenuAction) + 'static,
) -> Result<View, AppMenuError> {
    validate(&menus)?;
    let dispatch: Rc<dyn Fn(MenuAction)> = Rc::new(dispatch);
    Ok(compose::component(move |cx| {
        let menus = menus
            .into_iter()
            .map(|menu| {
                let open = cx.state(false);
                let toggle = open.clone();
                let anchor = compose::button()
                    .h(30.)
                    .p(6.)
                    .child(menu.label.clone())
                    .on_click(move || {
                        toggle.set(!toggle.get());
                    });
                compose::menu(menu.label, open, anchor).children(entries(
                    cx,
                    menu.entries,
                    dispatch.clone(),
                ))
            })
            .collect::<Vec<_>>();
        compose::row().gap(4.).children(menus)
    }))
}
fn entries(
    cx: &mut Context,
    entries: Vec<AppMenuEntry>,
    dispatch: Rc<dyn Fn(MenuAction)>,
) -> Vec<View> {
    entries
        .into_iter()
        .map(|entry| match entry {
            AppMenuEntry::Item {
                label,
                action,
                enabled,
            } => {
                let dispatch = dispatch.clone();
                compose::menu_item(label)
                    .h(30.)
                    .p(6.)
                    .disabled(!enabled)
                    .on_click(move || dispatch(action.clone()))
            }
            AppMenuEntry::Submenu(menu) => compose::submenu(menu.label, cx.state(false))
                .children(self::entries(cx, menu.entries, dispatch.clone())),
            AppMenuEntry::Separator => compose::row()
                .h(1.)
                .bg(lumine::scene::Color(100, 100, 100, 255)),
        })
        .collect()
}

#[cfg(target_os = "macos")]
pub(crate) struct NativeMenu {
    menu: muda::Menu,
}
#[cfg(target_os = "macos")]
impl NativeMenu {
    pub(crate) fn new(menus: &[AppMenu]) -> Result<Self, AppMenuError> {
        fn submenu(model: &AppMenu) -> Result<muda::Submenu, AppMenuError> {
            let menu = muda::Submenu::new(&model.label, true);
            for entry in &model.entries {
                let result = match entry {
                    AppMenuEntry::Item {
                        label,
                        action,
                        enabled,
                    } => menu.append(&muda::MenuItem::with_id(
                        action.0.clone(),
                        label,
                        *enabled,
                        None,
                    )),
                    AppMenuEntry::Submenu(child) => menu.append(&submenu(child)?),
                    AppMenuEntry::Separator => menu.append(&muda::PredefinedMenuItem::separator()),
                };
                result.map_err(|e| AppMenuError(e.to_string()))?;
            }
            Ok(menu)
        }
        validate(menus)?;
        let menu = muda::Menu::new();
        for model in menus {
            menu.append(&submenu(model)?)
                .map_err(|e| AppMenuError(e.to_string()))?;
        }
        Ok(Self { menu })
    }
    pub(crate) fn install(&self) {
        self.menu.init_for_nsapp();
    }
}
#[cfg(target_os = "macos")]
impl Drop for NativeMenu {
    fn drop(&mut self) {
        self.menu.remove_for_nsapp();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rendered_menu_keyboard_dispatches_shared_identifier_and_skips_disabled() {
        use lumine::{
            input::{InputEvent, Key, Modifiers},
            widgets::Ui,
        };
        let mut ui = Ui::new(600., 400.);
        let actions = Rc::new(std::cell::RefCell::new(Vec::new()));
        let output = actions.clone();
        let menus = vec![AppMenu::new(
            "File",
            [
                AppMenuEntry::item("Open", "open"),
                AppMenuEntry::item("Unavailable", "disabled").enabled(false),
                AppMenuEntry::item("Save", "save"),
            ],
        )];
        let _view =
            ui.mount(app_menu_bar(menus, move |action| output.borrow_mut().push(action)).unwrap());
        ui.prepare_frame();
        for key in [Key::Tab, Key::Enter, Key::ArrowDown, Key::Enter] {
            ui.dispatch(InputEvent::KeyDown {
                key: key.clone(),
                modifiers: Modifiers::default(),
                repeat: false,
            });
            ui.dispatch(InputEvent::KeyUp {
                key,
                modifiers: Modifiers::default(),
            });
            ui.prepare_frame();
        }
        assert_eq!(*actions.borrow(), vec![MenuAction::from("save")]);
    }
    #[test]
    fn duplicate_nested_actions_are_rejected_before_mount_or_native_creation() {
        let model = vec![AppMenu::new(
            "File",
            [
                AppMenuEntry::item("Open", "open"),
                AppMenuEntry::Submenu(AppMenu::new("More", [AppMenuEntry::item("Again", "open")])),
            ],
        )];
        assert!(validate(&model).is_err());
    }
    #[test]
    fn disabled_and_unknown_commands_cannot_dispatch() {
        let menus = vec![AppMenu::new(
            "File",
            [
                AppMenuEntry::item("Open", "open"),
                AppMenuEntry::item("Save", "save").enabled(false),
            ],
        )];
        assert!(enabled_action(&menus, &"open".into()));
        assert!(!enabled_action(&menus, &"save".into()));
        assert!(!enabled_action(&menus, &"unknown".into()));
    }
}
