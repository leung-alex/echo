use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
};

use crate::activation::show_main;

pub(crate) fn create_tray(app: &tauri::AppHandle) -> Result<(), String> {
    let open = MenuItemBuilder::with_id("open", "Open Echo")
        .build(app)
        .map_err(|error| error.to_string())?;
    let quit = MenuItemBuilder::with_id("quit", "Quit Echo")
        .build(app)
        .map_err(|error| error.to_string())?;
    let menu = MenuBuilder::new(app)
        .items(&[&open, &quit])
        .build()
        .map_err(|error| error.to_string())?;
    TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Echo Recall")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                let _ = show_main(app, "history", None, "tray-open");
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)
        .map(|_| ())
        .map_err(|error| error.to_string())
}
