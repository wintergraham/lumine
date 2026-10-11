//! Live shared menus with checked state and native/logical-key shortcuts.
use lumine::{compose::prelude::*, input::Key};
use lumine_desktop::{AppMenu, AppMenuEntry, Application, MenuAccelerator, WindowOptions};
fn model(checked: bool) -> Vec<AppMenu> {
    vec![AppMenu::new(
        "File",
        [
            AppMenuEntry::item("Save", "save")
                .enabled(checked)
                .accelerator(MenuAccelerator::primary(Key::Character("s".into()))),
            AppMenuEntry::item("Enabled", "toggle")
                .checked(checked)
                .accelerator(MenuAccelerator::primary(Key::Character("b".into()))),
            AppMenuEntry::item("Replace menu", "replace")
                .accelerator(MenuAccelerator::primary(Key::Character("r".into()))),
        ],
    )]
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .menus(model(false))?
        .window(WindowOptions {
            title: "lumine dynamic menus".into(),
            width: 500.,
            height: 300.,
            ..Default::default()
        })
        .run(|cx| {
            let windows = cx.windows.clone();
            let mut checked = false;
            cx.on_menu_action(move |action| {
                println!("ACTION {}", action.0);
                match action.0.as_str() {
                    "toggle" => {
                        checked = !checked;
                        windows
                            .set_menu_state("toggle", None, Some(checked))
                            .unwrap();
                        windows.set_menu_state("save", Some(checked), None).unwrap();
                        println!("CHECKED {checked}");
                    }
                    "replace" => {
                        windows
                            .set_menus(vec![AppMenu::new(
                                "Tools",
                                [AppMenuEntry::item("New command", "new").accelerator(
                                    MenuAccelerator::primary(Key::Character("d".into())),
                                )],
                            )])
                            .unwrap();
                        println!("REPLACED");
                    }
                    _ => {}
                }
            });
            let editor = cx.ui.signal(String::new());
            cx.render(
                column()
                    .p(12.)
                    .gap(12.)
                    .child(cx.app_menu_bar())
                    .child(text_input("Editor", editor).w(400.).h(40.))
                    .child("Ctrl/Cmd+B enables Save; Ctrl/Cmd+R replaces menus."),
            );
            cx.windows.open(
                WindowOptions {
                    title: "lumine dynamic menus secondary".into(),
                    width: 420.,
                    height: 220.,
                    ..Default::default()
                },
                |cx| {
                    cx.on_menu_action(|action| println!("SECOND {}", action.0));
                    cx.render(
                        column()
                            .p(12.)
                            .child(cx.app_menu_bar())
                            .child("Menu changes apply here too."),
                    );
                },
            );
            println!("READY");
        })
}
