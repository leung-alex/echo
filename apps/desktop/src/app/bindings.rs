//! Slint callbacks enqueue commands; no domain calls happen on the render thread.
use super::*;
use slint::{CloseRequestResponse, DataTransfer};
fn dragged(data: &DataTransfer) -> Option<RowKey> {
    data.user_data()?
        .downcast::<RowKey>()
        .ok()
        .map(|key| *key)
        .filter(|key| key.source == QuickInsertSource::Favorite)
}
pub(super) fn connect(app: &App) {
    for role in [Role::Main, Role::Favorites] {
        let window = &app.windows[role.index()];
        macro_rules! callback {
            ($method:ident, $($arg:ident),* => $command:expr) => {{
                let hub=app.hub.clone();window.$method(move |$($arg),*| hub.post(Event::Command($command)));
            }};
        }
        callback!(on_query_edited, query => Command::Query(role,query.to_string()));
        callback!(on_row_selected, key => Command::Select(role,key.to_string()));
        callback!(on_row_action, action,key => Command::Action(role,action.to_string(),key.to_string()));
        callback!(on_batch_action, action => Command::Batch(role,action.to_string()));
        callback!(on_load_more, => Command::More(role));
        callback!(on_previous_page, => Command::Previous(role));
        callback!(on_panel_selected, panel => Command::Panel(role,panel.to_string()));
        callback!(on_route_selected, route => Command::Route(route.to_string()));
        callback!(on_dismiss, => Command::Dismiss(role));
        callback!(on_drag_requested, => Command::Drag(role));
        callback!(on_thumbnail_requested, key => Command::Thumbnail(role,key.to_string()));
        callback!(on_save_settings, => Command::SaveSettings);
        callback!(on_save_favorite, => Command::SaveFavorite(role));
        callback!(on_cancel_editor, => Command::CancelEditor(role));
        callback!(on_confirm_clear, => Command::Clear(role));
        callback!(on_create_favorite, => Command::Create(role));
        callback!(on_quit, => Command::Quit);
        let hub = app.hub.clone();
        window.on_key_event(move |key, ctrl, shift, target| {
            let intent = key_intent(role, key.as_str(), ctrl, shift, target.as_str())
                .unwrap_or(Intent::None);
            if intent == Intent::None {
                return false;
            }
            hub.post(Event::Command(Command::Keyboard(role, intent)));
            true
        });
        window.on_drag_data(|key| {
            let mut data = DataTransfer::default();
            if let Ok(key) = key.as_str().parse::<RowKey>() {
                if key.source == QuickInsertSource::Favorite {
                    data.set_user_data(Rc::new(key));
                }
            }
            data
        });
        window.on_can_drop_data(|data| dragged(&data).is_some());
        let hub = app.hub.clone();
        window.on_drop_data(move |data, target| {
            let Some(source) = dragged(&data) else {
                return false;
            };
            let Ok(target) = target.as_str().parse::<RowKey>() else {
                return false;
            };
            if target.source != QuickInsertSource::Favorite {
                return false;
            }
            hub.post(Event::Command(Command::Reorder(role, source, target)));
            true
        });
        let hub = app.hub.clone();
        window.window().on_close_requested(move || {
            hub.post(Event::Command(Command::Dismiss(role)));
            CloseRequestResponse::KeepWindowShown
        });
    }
}
