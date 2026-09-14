//! Typed, single-window callbacks. Clipboard identities are resolved by presentation.
use super::*;
use slint::{CloseRequestResponse, DataTransfer};
fn dragged(data: &DataTransfer) -> Option<crate::events::DragOrigin> {
    data.user_data()?
        .downcast::<crate::events::DragOrigin>()
        .ok()
        .map(|k| *k)
        .filter(|k| k.key.source == QuickInsertSource::Favorite)
}
pub(super) fn connect(app: &App) {
    let window = &app.window;
    macro_rules! callback {
        ($method:ident, $($arg:ident),* => $command:expr) => {{
            let hub=app.hub.clone();window.$method(move |$($arg),*| hub.post(Event::Command($command)));
        }};
    }
    window
        .global::<crate::FavoriteIconImages>()
        .on_index_for(|key| crate::favorite_icons::index(key.as_str()));
    window.set_favorite_icon_choices(crate::favorite_icons::choices(""));
    let weak = window.as_weak();
    window.on_filter_favorite_icons(move |query| {
        if let Some(window) = weak.upgrade() {
            window.set_favorite_icon_choices(crate::favorite_icons::choices(query.as_str()));
        }
    });
    callback!(on_query_edited,query=>Command::Query(query.to_string()));
    callback!(on_row_selected,key=>Command::Select(key.to_string()));
    callback!(on_row_action,action,key=>Command::Action(action.to_string(),key.to_string()));
    callback!(on_batch_action,action=>Command::Batch(action.to_string()));
    callback!(on_load_more,=>Command::More);
    callback!(on_previous_page,=>Command::Previous);
    callback!(on_panel_selected,panel=>Command::Panel(panel.to_string()));
    callback!(on_route_selected,route=>Command::Route(route.to_string()));
    callback!(on_dismiss,=>Command::Dismiss);
    callback!(on_drag_requested,=>Command::Drag);
    let weak = window.as_weak();
    let hub = app.hub.clone();
    window.on_thumbnail_requested(move |key| {
        if weak.upgrade().is_some() {
            hub.post(Event::Command(Command::Thumbnail(key.to_string())));
        }
    });
    callback!(on_save_settings,=>Command::SaveSettings);
    callback!(on_settings_edited,=>Command::SettingsEdited);
    callback!(on_settings_action,action=>Command::SettingsAction(action.to_string()));
    callback!(on_space_action,action,key=>Command::SpaceAction(action.to_string(),key.to_string()));
    callback!(on_navigate_space,delta=>Command::Keyboard(Intent::SwitchSpace(delta)));
    callback!(on_picker_selected,key=>Command::PickerSelect(key.to_string()));
    callback!(on_picker_query_edited,query=>Command::PickerQuery(query.to_string()));
    callback!(on_picker_next,=>Command::PickerMore);
    callback!(on_confirm,answer=>Command::Confirm(answer.to_string()));
    let weak = window.as_weak();
    let hub = app.hub.clone();
    window.on_viewport_changed(move || {
        if weak.upgrade().is_some() {
            hub.post(Event::Command(Command::ViewportChanged));
        }
    });
    callback!(on_stage_scrolled,delta=>Command::StageScroll(delta));
    let weak = window.as_weak();
    window.on_editor_edited(move || {
        if let Some(window) = weak.upgrade() {
            super::editor_validation::edited(&window);
        }
    });
    callback!(on_save_favorite,=>Command::SaveFavorite);
    callback!(on_cancel_editor,=>Command::CancelEditor);
    callback!(on_confirm_clear,=>Command::Clear);
    callback!(on_create_favorite,=>Command::Create);
    callback!(on_quit,=>Command::Quit);
    let hub = app.hub.clone();
    window.on_key_event(move |key, ctrl, shift, target| {
        let intent = key_intent(key.as_str(), ctrl, shift, target.as_str()).unwrap_or(Intent::None);
        if intent == Intent::None {
            return false;
        }
        hub.post(Event::Command(Command::Keyboard(intent)));
        true
    });
    window.on_ime_composing(is_composing);
    window.on_drag_data(|key, binding| {
        let mut data = DataTransfer::default();
        let origin = APP.with(|slot| {
            let slot = slot.borrow();
            let app = slot.as_ref()?.try_borrow().ok()?;
            let key = app.surface.resolve_key(key.as_str())?;
            (key.source == QuickInsertSource::Favorite).then_some(crate::events::DragOrigin {
                frame: app.software_frame_stamp(),
                binding,
                key,
            })
        });
        if let Some(origin) = origin {
            data.set_user_data(Rc::new(origin));
        }
        data
    });
    window.on_can_drop_data(|data| dragged(&data).is_some());
    let hub = app.hub.clone();
    window.on_drop_data(move |data, target| {
        let Some(source) = dragged(&data) else {
            return false;
        };
        let Some(target) =
            resolve_key(target.as_str()).filter(|k| k.source == QuickInsertSource::Favorite)
        else {
            return false;
        };
        hub.post(Event::Command(Command::Reorder(source, target)));
        true
    });
    let hub = app.hub.clone();
    window.window().on_close_requested(move || {
        hub.post(Event::Command(Command::Dismiss));
        CloseRequestResponse::KeepWindowShown
    });
}
