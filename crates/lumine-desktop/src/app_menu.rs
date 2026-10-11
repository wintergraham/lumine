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
/// A single logical-key shortcut shared by native macOS and Linux key routing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuAccelerator(pub lumine::actions::Keystroke);
impl MenuAccelerator {
    pub fn new(key: lumine::input::Key, modifiers: lumine::input::Modifiers) -> Self {
        Self(lumine::actions::Keystroke::new(key, modifiers))
    }
    pub fn primary(key: lumine::input::Key) -> Self {
        Self::new(
            key,
            lumine::input::Modifiers {
                control: !cfg!(target_os = "macos"),
                meta: cfg!(target_os = "macos"),
                ..Default::default()
            },
        )
    }
    fn name(&self) -> Result<String, AppMenuError> {
        use lumine::input::Key;
        let key = match &self.0.key {
            Key::Character(s)
                if s.chars().count() == 1 && s.chars().all(|c| c.is_ascii_alphanumeric()) =>
            {
                s.to_ascii_uppercase()
            }
            Key::Function(n) if (1..=35).contains(n) => format!("F{n}"),
            Key::Character(_) | Key::Function(_) => {
                return Err(AppMenuError(
                    "menu accelerators require one ASCII letter/digit or a supported named key"
                        .into(),
                ));
            }
            key => format!("{key:?}"),
        };
        let m = self.0.modifiers;
        let mut parts = Vec::new();
        if m.control {
            parts.push("Ctrl".to_owned())
        }
        if m.meta {
            parts.push("Super".to_owned())
        }
        if m.alt {
            parts.push("Alt".to_owned())
        }
        if m.shift {
            parts.push("Shift".to_owned())
        }
        parts.push(key);
        Ok(parts.join("+"))
    }
    pub fn label(&self) -> String {
        self.name().unwrap_or_default()
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
        checked: Option<bool>,
        accelerator: Option<MenuAccelerator>,
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
            checked: None,
            accelerator: None,
        }
    }
    pub fn checked(mut self, checked: bool) -> Self {
        if let Self::Item { checked: value, .. } = &mut self {
            *value = Some(checked)
        }
        self
    }
    pub fn accelerator(mut self, accelerator: MenuAccelerator) -> Self {
        if let Self::Item {
            accelerator: value, ..
        } = &mut self
        {
            *value = Some(accelerator)
        }
        self
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
    fn visit(
        menus: &[AppMenu],
        ids: &mut HashSet<String>,
        shortcuts: &mut HashSet<String>,
    ) -> Result<(), AppMenuError> {
        for menu in menus {
            if menu.label.is_empty() || menu.label.contains('\0') {
                return Err(AppMenuError(
                    "menu labels must be nonempty and contain no NUL".into(),
                ));
            }
            for entry in &menu.entries {
                match entry {
                    AppMenuEntry::Item {
                        label,
                        action,
                        accelerator,
                        ..
                    } => {
                        if let Some(shortcut) = accelerator
                            && !shortcuts.insert(shortcut.name()?)
                        {
                            return Err(AppMenuError("menu accelerators must be unique".into()));
                        }
                        if label.is_empty()
                            || label.contains('\0')
                            || action.0.is_empty()
                            || action.0.contains('\0')
                            || !ids.insert(action.0.clone())
                        {
                            return Err(AppMenuError("menu items need valid labels and unique nonempty action identifiers".into()));
                        }
                    }
                    AppMenuEntry::Submenu(menu) => {
                        visit(std::slice::from_ref(menu), ids, shortcuts)?
                    }
                    AppMenuEntry::Separator => {}
                }
            }
        }
        Ok(())
    }
    visit(menus, &mut HashSet::new(), &mut HashSet::new())
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
                checked,
                accelerator,
            } => {
                let label = format!(
                    "{}{}{}",
                    if checked == Some(true) {
                        "✓ "
                    } else if checked.is_some() {
                        "  "
                    } else {
                        ""
                    },
                    label,
                    accelerator
                        .map(|a| format!("    {}", a.label()))
                        .unwrap_or_default()
                );
                let dispatch = dispatch.clone();
                let item = compose::menu_item(label);
                let item = if let Some(checked) = checked {
                    item.menu_checked(checked)
                } else {
                    item
                };
                item.h(30.)
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

pub(crate) fn keymap(menus: &[AppMenu]) -> lumine::actions::Keymap {
    use lumine::actions::{Action, KeyBinding, Keymap};
    fn collect(menus: &[AppMenu], mut map: Keymap) -> Keymap {
        for menu in menus {
            for entry in &menu.entries {
                match entry {
                    AppMenuEntry::Item {
                        action,
                        enabled: true,
                        accelerator: Some(key),
                        ..
                    } => {
                        let mut stroke = key.0.clone();
                        if let lumine::input::Key::Character(s) = &mut stroke.key {
                            *s = s.to_ascii_lowercase();
                        }
                        map = map.bind(
                            KeyBinding::new([stroke.clone()], Action::new(action.clone()))
                                .expect("single menu shortcut"),
                        );
                        if let lumine::input::Key::Character(s) = &mut stroke.key {
                            let upper = s.to_ascii_uppercase();
                            if upper != *s {
                                *s = upper;
                                map = map.bind(
                                    KeyBinding::new([stroke], Action::new(action.clone()))
                                        .expect("single menu shortcut"),
                                );
                            }
                        }
                    }
                    AppMenuEntry::Submenu(menu) => map = collect(std::slice::from_ref(menu), map),
                    _ => {}
                }
            }
        }
        map
    }
    collect(menus, Keymap::new())
}
pub(crate) fn update_entry(
    menus: &mut [AppMenu],
    action: &MenuAction,
    enabled: Option<bool>,
    checked: Option<bool>,
) -> bool {
    for menu in menus {
        for entry in &mut menu.entries {
            match entry {
                AppMenuEntry::Item {
                    action: id,
                    enabled: current,
                    checked: check,
                    ..
                } if id == action => {
                    if let Some(value) = enabled {
                        *current = value
                    }
                    if let Some(value) = checked {
                        *check = Some(value)
                    }
                    return true;
                }
                AppMenuEntry::Submenu(menu) => {
                    if update_entry(std::slice::from_mut(menu), action, enabled, checked) {
                        return true;
                    }
                }
                _ => {}
            }
        }
    }
    false
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
                        checked,
                        accelerator,
                    } => {
                        let accelerator = accelerator
                            .as_ref()
                            .map(|a| {
                                a.name()?
                                    .parse::<muda::accelerator::KeyAccelerator>()
                                    .map_err(|e| AppMenuError(e.to_string()))
                            })
                            .transpose()?;
                        if let Some(checked) = checked {
                            let item = muda::CheckMenuItem::with_id(
                                action.0.clone(),
                                label,
                                *enabled,
                                *checked,
                                None,
                            );
                            item.set_key_accelerator(accelerator)
                                .map_err(|e| AppMenuError(e.to_string()))?;
                            menu.append(&item)
                        } else {
                            let item =
                                muda::MenuItem::with_id(action.0.clone(), label, *enabled, None);
                            item.set_key_accelerator(accelerator)
                                .map_err(|e| AppMenuError(e.to_string()))?;
                            menu.append(&item)
                        }
                    }
                    AppMenuEntry::Submenu(child) => menu.append(&submenu(child)?),
                    AppMenuEntry::Separator => menu.append(&muda::PredefinedMenuItem::separator()),
                };
                result.map_err(|e| AppMenuError(e.to_string()))?;
            }
            Ok(menu)
        }
        validate(menus)?;
        let menu = muda::Menu::new();
        // AppKit reserves the first submenu for the application and replaces
        // its title with the process name. Keep user menus in subsequent slots,
        // including after dynamic model replacement.
        let application = muda::Submenu::new("Application", true);
        application
            .append_items(&[
                &muda::PredefinedMenuItem::services(None),
                &muda::PredefinedMenuItem::separator(),
                &muda::PredefinedMenuItem::hide(None),
                &muda::PredefinedMenuItem::hide_others(None),
                &muda::PredefinedMenuItem::show_all(None),
                &muda::PredefinedMenuItem::separator(),
                &muda::PredefinedMenuItem::quit(None),
            ])
            .map_err(|e| AppMenuError(e.to_string()))?;
        menu.append(&application)
            .map_err(|e| AppMenuError(e.to_string()))?;
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

#[cfg(test)]
mod dynamic_tests {
    use super::*;
    use lumine::{
        actions::Action,
        input::{EventPhase, InputEvent, Key, Modifiers},
        widgets::Ui,
    };
    #[test]
    fn checked_menu_items_publish_accessible_controlled_state() {
        let mut ui = Ui::new(400., 300.);
        let open = ui.signal(true);
        let view = ui.mount(
            compose::menu("File", open, compose::button().child("File"))
                .child(compose::menu_item("Enabled").menu_checked(true)),
        );
        ui.prepare_frame();
        let semantics = ui.semantics.borrow();
        let (_, item) = semantics
            .iter()
            .find(|(_, s)| s.label == "Enabled" && s.role == lumine::semantics::Role::MenuItem)
            .unwrap();
        assert_eq!(item.checked, Some(true));
        drop(semantics);
        let update = crate::accessibility::AccessibilityTree::new().update(
            &ui.scene.borrow(),
            &ui.semantics.borrow(),
            None,
            "Menu",
            1.,
        );
        assert!(
            update
                .nodes
                .iter()
                .any(|(_, node)| node.role() == accesskit::Role::MenuItemCheckBox
                    && node.toggled() == Some(accesskit::Toggled::True))
        );
        view.unmount();
    }
    #[test]
    fn replacement_unregisters_shortcuts_and_focused_action_can_override() {
        let mut ui = Ui::new(400., 300.);
        let root = ui.scene.borrow().root();
        let output = Rc::new(std::cell::RefCell::new(Vec::new()));
        let fallback = output.clone();
        let _listener = ui.input.listen(root, move |event| {
            if event.phase != EventPhase::Capture
                && !event.default_prevented()
                && let InputEvent::Action(action) = &event.event
                && let Some(action) = action.downcast_ref::<MenuAction>()
            {
                fallback.borrow_mut().push(action.0.clone());
                event.prevent_default();
            }
        });
        let model = vec![AppMenu::new(
            "File",
            [AppMenuEntry::item("Save", "save")
                .accelerator(MenuAccelerator::primary(Key::Character("s".into())))],
        )];
        let binding = ui.input.bind_keys(root, keymap(&model));
        let override_count = Rc::new(std::cell::Cell::new(0));
        let count = override_count.clone();
        let view = ui.mount(compose::button().child("Focused").on_action::<MenuAction>(
            move |_, event| {
                count.set(count.get() + 1);
                event.prevent_default();
            },
        ));
        ui.prepare_frame();
        ui.input.focus(&ui.scene, Some(view.node()));
        let press = || InputEvent::KeyDown {
            key: Key::Character("s".into()),
            modifiers: Modifiers {
                control: !cfg!(target_os = "macos"),
                meta: cfg!(target_os = "macos"),
                ..Default::default()
            },
            repeat: false,
        };
        ui.dispatch(press());
        assert_eq!(override_count.get(), 1);
        assert!(output.borrow().is_empty());
        view.unmount();
        ui.input.focus(&ui.scene, None);
        ui.dispatch(press());
        assert_eq!(*output.borrow(), ["save"]);
        drop(binding);
        let replacement = vec![AppMenu::new(
            "Tools",
            [AppMenuEntry::item("Disabled", "save")
                .enabled(false)
                .accelerator(MenuAccelerator::primary(Key::Character("s".into())))],
        )];
        let _binding = ui.input.bind_keys(root, keymap(&replacement));
        ui.dispatch(press());
        assert_eq!(*output.borrow(), ["save"]);
        // Direct application actions still route through the same typed channel.
        ui.input
            .dispatch_action(&ui.scene, Action::new(MenuAction("new".into())));
        assert_eq!(*output.borrow(), ["save", "new"]);
    }
    #[test]
    fn updates_preserve_shortcut_and_reject_ambiguous_accelerators() {
        let shortcut = MenuAccelerator::primary(lumine::input::Key::Character("s".into()));
        let mut model = vec![AppMenu::new(
            "File",
            [AppMenuEntry::item("Save", "save").accelerator(shortcut.clone())],
        )];
        assert!(update_entry(
            &mut model,
            &"save".into(),
            Some(false),
            Some(true)
        ));
        assert!(!enabled_action(&model, &"save".into()));
        assert!(
            matches!(&model[0].entries[0],AppMenuEntry::Item{checked:Some(true),accelerator:Some(a),..} if a==&shortcut)
        );
        assert!(!update_entry(
            &mut model,
            &"missing".into(),
            Some(true),
            None
        ));
        model[0]
            .entries
            .push(AppMenuEntry::item("Other", "other").accelerator(shortcut));
        assert!(validate(&model).is_err());
    }
}
#[cfg(test)]
mod live_model_test {
    use super::*;
    #[test]
    fn replacing_rendered_bar_settles_without_changing_editor_ownership() {
        use lumine::widgets::Ui;
        let mut ui = Ui::new(500., 300.);
        let model = ui.signal(vec![AppMenu::new(
            "File",
            [AppMenuEntry::item("Save", "save")],
        )]);
        let read = model.clone();
        let view = ui.mount(compose::switch(
            move || read.get(),
            |menus, _| app_menu_bar(menus, |_| {}).unwrap(),
        ));
        ui.prepare_frame();
        model.set(vec![AppMenu::new(
            "File",
            [AppMenuEntry::item("Save", "save")
                .enabled(false)
                .checked(true)],
        )]);
        ui.prepare_frame();
        model.set(vec![AppMenu::new(
            "Tools",
            [AppMenuEntry::item("New", "new")],
        )]);
        ui.prepare_frame();
        view.unmount();
    }
}
