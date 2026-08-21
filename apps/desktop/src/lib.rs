use echo_activation::decode_args;
use tauri::{Manager, WindowEvent};

use crate::activation::handle_activation;
use crate::commands::{
    activation_ack, activation_state, history_clear, quick_insert_activate_panel,
    quick_insert_active_panel, quick_insert_begin_session, quick_insert_clear_session,
    quick_insert_clear_unpinned_history, quick_insert_create_favorite,
    quick_insert_delete_favorite, quick_insert_delete_history_many, quick_insert_execute,
    quick_insert_list, quick_insert_move_history_many_to_favorites,
    quick_insert_move_history_to_favorite, quick_insert_pin_history, quick_insert_pin_history_many,
    quick_insert_reorder_favorites, quick_insert_unpin_history, quick_insert_update_favorite,
    settings_get, settings_update,
};
use crate::composition::{create_main_window, hide_composition, reposition_favorites, EchoState};
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
            quick_insert_clear_session,
            quick_insert_move_history_to_favorite,
            quick_insert_move_history_many_to_favorites,
            quick_insert_create_favorite,
            quick_insert_update_favorite,
            quick_insert_pin_history,
            quick_insert_unpin_history,
            quick_insert_pin_history_many,
            quick_insert_delete_history_many,
            quick_insert_clear_unpinned_history,
            quick_insert_reorder_favorites,
            quick_insert_delete_favorite,
            quick_insert_activate_panel,
            quick_insert_active_panel,
            settings_get,
            settings_update,
            history_clear,
            activation_state,
            activation_ack,
        ])
        .setup(|app| {
            let state = EchoState::build().map_err(std::io::Error::other)?;
            state.start_history_event_bridge(app.handle());
            app.manage(state);
            create_tray(app.handle()).map_err(std::io::Error::other)?;
            create_main_window(app.handle()).map_err(std::io::Error::other)?;
            let args = std::env::args().collect::<Vec<_>>();
            if let Some(Ok(envelope)) = decode_args(args.iter().map(String::as_str)) {
                handle_activation(app.handle(), envelope).map_err(std::io::Error::other)?;
            } else {
                crate::composition::show_composition(app.handle())
                    .map_err(std::io::Error::other)?;
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                hide_composition(window.app_handle());
            }
            WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                if window.label() == "main" {
                    reposition_favorites(window.app_handle());
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                reposition_favorites(window.app_handle());
            }
            WindowEvent::Destroyed => {
                if window.label() == "main" {
                    if let Some(favorites) = window.app_handle().get_webview_window("favorites") {
                        let _ = favorites.close();
                    }
                }
            }
            _ => {}
        });
    builder
        .run(tauri::generate_context!())
        .expect("error while running Echo Recall");
}
