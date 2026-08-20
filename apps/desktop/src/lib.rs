use echo_activation::decode_args;
use tauri::{Manager, WindowEvent};

use crate::activation::handle_activation;
use crate::commands::{
    activation_ack, activation_state, history_clear, quick_insert_begin_session,
    quick_insert_delete, quick_insert_execute, quick_insert_list, quick_insert_set_favorite,
    saved_item_delete, saved_item_update, saved_items_delete_many, settings_get, settings_update,
};
use crate::composition::{create_main_window, EchoState};
use crate::events::create_tray;

mod activation;
mod commands;
mod composition;
mod events;
mod preview_protocol;
mod transport;

pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(Ok(envelope)) = decode_args(argv.iter().map(String::as_str)) {
                let _ = handle_activation(app, envelope);
            }
        }))
        .register_uri_scheme_protocol("echo-preview", |context, request| {
            preview_protocol::serve(
                &context.app_handle().state::<EchoState>().library,
                request.uri().path(),
            )
        })
        .invoke_handler(tauri::generate_handler![
            quick_insert_list,
            quick_insert_begin_session,
            quick_insert_execute,
            quick_insert_set_favorite,
            quick_insert_delete,
            saved_item_update,
            saved_item_delete,
            saved_items_delete_many,
            settings_get,
            settings_update,
            history_clear,
            activation_state,
            activation_ack,
        ])
        .setup(|app| {
            let state = EchoState::build().map_err(std::io::Error::other)?;
            app.manage(state);
            create_tray(app.handle()).map_err(std::io::Error::other)?;
            create_main_window(app.handle()).map_err(std::io::Error::other)?;
            let args = std::env::args().collect::<Vec<_>>();
            if let Some(Ok(envelope)) = decode_args(args.iter().map(String::as_str)) {
                handle_activation(app.handle(), envelope).map_err(std::io::Error::other)?;
            } else if let Some(window) = app.get_webview_window("main") {
                window.show().map_err(std::io::Error::other)?;
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        });
    builder
        .run(tauri::generate_context!())
        .expect("error while running Echo Recall");
}
