#![cfg(target_os = "windows")]

use gitbutler_tauri::menu::menu_item_set_enabled;
use tauri::menu::{Menu, SubmenuBuilder};

#[test]
fn menu_item_enabled_state() -> anyhow::Result<()> {
    let app = tauri::test::mock_app();
    let project_menu = SubmenuBuilder::new(&app, "Project")
        .text("project/open-in-vscode", "Open in Editor")
        .build()?;
    let menu = Menu::with_items(&app, &[&project_menu])?;
    app.set_menu(menu)?;

    for enabled in [false, true] {
        menu_item_set_enabled(
            app.handle().clone(),
            "project/open-in-vscode".into(),
            enabled,
        )?;
        let item = project_menu.get("project/open-in-vscode").unwrap();
        assert_eq!(
            item.as_menuitem().unwrap().is_enabled()?,
            enabled,
            "the Open in Editor item must reflect the requested state"
        );
    }
    Ok(())
}

#[test]
fn menu_item_missing_menu() {
    let app = tauri::test::mock_app();
    assert!(
        menu_item_set_enabled(app.handle().clone(), "missing".into(), false).is_err(),
        "updating an item without an application menu must fail"
    );
}

#[test]
fn menu_item_missing_id() -> anyhow::Result<()> {
    let app = tauri::test::mock_app();
    app.set_menu(Menu::new(&app)?)?;
    assert!(
        menu_item_set_enabled(app.handle().clone(), "missing".into(), false).is_err(),
        "updating an unknown menu item must fail"
    );
    Ok(())
}
