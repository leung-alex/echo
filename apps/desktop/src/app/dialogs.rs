//! Native content management dialogs. Data changes require explicit confirmation.
use super::*;
#[derive(Clone)]
pub(super) enum Confirmation {
    Hide,
    Quit,
    Route(String),
    CancelEditor,
    CancelSpace,
    DeleteItem(RowKey),
    DeleteHistory(Vec<i64>),
    DeleteSpace(SpaceId, i64),
    ClearHistory,
    Restart,
}
#[derive(Clone, Default)]
pub(super) enum Picker {
    #[default]
    Closed,
    Navigator,
    SpaceMenu,
    ItemMenu(RowKey),
    Target(RowKey),
    Catalog(SpaceId),
}
impl App {
    pub(super) fn ask_confirmation(
        &mut self,
        title: &str,
        body: &str,
        button: &str,
        danger: bool,
        action: Confirmation,
    ) {
        self.window.set_confirmation_title(title.into());
        self.window.set_confirmation_body(body.into());
        self.window.set_confirmation_button(button.into());
        self.window.set_confirmation_danger(danger);
        self.confirmation = Some(action);
        self.window.set_confirmation_open(true);
    }
    pub(super) fn confirm(&mut self, answer: &str) {
        if self.mutation.is_some() || self.session.busy() {
            return;
        }
        let action = self.confirmation.take();
        self.window.set_confirmation_open(false);
        if answer != "accept" {
            return;
        }
        match action {
            Some(Confirmation::Hide) => {
                self.discard_drafts();
                self.dismiss();
            }
            Some(Confirmation::Quit) => {
                self.discard_drafts();
                self.quit();
            }
            Some(Confirmation::Route(route)) => {
                self.discard_drafts();
                self.set_route(&route);
            }
            Some(Confirmation::CancelEditor) => self.close_editor(),
            Some(Confirmation::CancelSpace) => {
                self.window.set_space_dialog_open(false);
                self.space_original = None;
                self.space_edit_id = None;
            }
            Some(Confirmation::DeleteItem(key)) => {
                self.close_picker();
                self.clear_flow_cache();
                self.mutate(Mutation::Delete(key));
            }
            Some(Confirmation::DeleteHistory(ids)) => {
                self.clear_flow_cache();
                self.mutate(Mutation::BulkDelete(ids));
            }
            Some(Confirmation::ClearHistory) => {
                self.clear_flow_cache();
                self.mutate(Mutation::Clear);
            }
            Some(Confirmation::DeleteSpace(id, revision)) => {
                self.close_picker();
                if self.surface.space == id {
                    self.navigate_to(SpaceId::FAVORITES);
                    self.finish_motion();
                }
                self.space_mutation_at(id, revision, SpaceAction::Delete);
            }
            Some(Confirmation::Restart) => {
                self.restart = Some(false);
                self.quit();
            }
            None => {}
        }
    }
    pub(super) fn unsaved(&self) -> bool {
        self.window.get_settings_dirty() || self.editor_dirty() || self.space_dirty()
    }
    pub(super) fn request_hide(&mut self) {
        if self.mutation.is_some() || self.session.busy() {
            return;
        }
        if self.unsaved() {
            self.ask_confirmation(
                "Discard unsaved changes?",
                "Your saved content stays intact. Discard this draft and hide Echo?",
                "Discard & hide",
                false,
                Confirmation::Hide,
            );
        } else {
            self.discard_drafts();
            self.dismiss();
        }
    }
    pub(super) fn request_quit(&mut self) {
        if self.mutation.is_some() || self.session.busy() {
            self.report("Wait for the current operation to finish", false);
            return;
        }
        if self.unsaved() {
            self.ask_confirmation(
                "Discard changes and quit?",
                "This closes Echo and stops clipboard capture. Saved content is kept.",
                "Quit Echo",
                false,
                Confirmation::Quit,
            );
        } else {
            self.quit();
        }
    }
    pub(super) fn request_route(&mut self, route: &str) {
        if !matches!(route, "history" | "settings" | "about") || self.mutation.is_some() {
            return;
        }
        if self.window.get_route().as_str() == route {
            return;
        }
        if self.unsaved() {
            self.ask_confirmation(
                "Discard unsaved changes?",
                "Leave this page without saving the current draft?",
                "Discard changes",
                false,
                Confirmation::Route(route.into()),
            );
        } else {
            self.set_route(route);
        }
    }
    pub(super) fn set_route(&mut self, route: &str) {
        if self.inline_active() && route != "history" {
            self.stop_inline();
            self.activation_focus = None;
            self.popup_anchor = None;
            self.session.dismiss();
            self.worker
                .epoch
                .store(self.session.epoch, Ordering::Release);
            self.send(Work::Cancel);
            self.window.set_quick_insert(false);
            self.window.set_paste_target_available(false);
            if let Some(hwnd) = self.hwnd {
                let _ = shell::focus_window(hwnd);
            }
        }
        self.remember_position();
        self.finish_motion();
        self.preview_timer.stop();
        self.preview_started = None;
        if self.window.get_route().as_str() == "settings" && route != "settings" {
            self.clear_flow_cache();
        }
        self.close_picker();
        self.window.set_route(route.into());
        if route == "settings" {
            self.cancel_prewarm();
            self.render_settings();
            self.refresh_diagnostics();
            self.window.invoke_focus_controls();
        } else if route == "history" {
            self.apply_theme();
            self.window.invoke_focus_search(false);
            if self.surface.dirty {
                self.load(false);
            }
            self.schedule_prewarm();
        }
        let center = self.prepare_window_geometry();
        if let Some(hwnd) = self.hwnd.filter(|_| !self.quick_geometry_active) {
            let _ = shell::fit_window(hwnd, center, 16.0);
        }
        // Settings does not recapture or discard the external Quick Insert session.
    }
    fn discard_drafts(&mut self) {
        self.close_editor();
        self.window.set_space_dialog_open(false);
        self.space_original = None;
        self.space_edit_id = None;
        self.window.set_clear_confirm_open(false);
        self.close_picker();
        self.render_settings();
        self.apply_theme();
    }
    pub(super) fn escape(&mut self) {
        if self.hook.as_ref().is_some_and(WindowHook::is_composing) || self.mutation.is_some() {
            return;
        }
        if self.window.get_confirmation_open() {
            self.confirm("cancel");
            return;
        }
        if self.window.get_picker_open() {
            self.close_picker();
            return;
        }
        if self.window.get_space_dialog_open() {
            self.space_action("cancel", "");
            return;
        }
        if self.window.get_editor_open() {
            self.cancel_editor();
            return;
        }
        if self.window.get_clear_confirm_open() {
            self.window.set_clear_confirm_open(false);
            return;
        }
        if self.window.get_route().as_str() != "history" {
            self.request_route("history");
            return;
        }
        if self.surface.batch {
            self.surface.set_batch(false);
            self.render_selection();
            return;
        }
        if !self.surface.query.is_empty() {
            self.window.set_query("".into());
            self.command(Command::Query(String::new()));
            return;
        }
        self.request_hide();
    }
    pub(super) fn space_mutation(&mut self, id: SpaceId, action: SpaceAction) {
        let Some(space) = self.spaces.iter().find(|s| s.id == id) else {
            self.report("Space no longer exists", true);
            return;
        };
        self.space_mutation_at(id, space.revision, action);
    }
    fn space_mutation_at(&mut self, id: SpaceId, revision: i64, action: SpaceAction) {
        let request_id = format!(
            "desktop:{}:{}:{}",
            std::process::id(),
            self.session.epoch,
            self.serial.wrapping_add(1)
        );
        self.mutate(Mutation::Space(SpaceCommand {
            space_id: Some(id),
            expected_revision: Some(revision),
            request_id,
            action,
        }));
    }
    pub(super) fn space_action(&mut self, action: &str, key: &str) {
        if self.mutation.is_some() || self.session.busy() {
            return;
        }
        let id = SpaceId::parse(key).unwrap_or(self.surface.space);
        match action {
            "new" => {
                self.finish_motion();
                self.close_picker();
                self.space_edit_id = None;
                self.space_original = None;
                self.window.set_space_dialog_new(true);
                self.window.set_space_draft_title("".into());
                self.window.set_space_draft_description("".into());
                self.window.set_space_draft_icon("Folder".into());
                self.window.set_space_draft_accent("amber".into());
                self.report("", false);
                self.window.set_space_dialog_open(true);
            }
            "edit" => {
                let Some(space) = self.spaces.iter().find(|s| s.id == id).cloned() else {
                    return;
                };
                if id.is_system() {
                    self.report("System spaces are fixed", false);
                    return;
                }
                self.close_picker();
                self.space_edit_id = Some((id, space.revision));
                self.window.set_space_dialog_new(false);
                self.window.set_space_draft_title(space.title.into());
                self.window
                    .set_space_draft_description(space.description.into());
                self.window
                    .set_space_draft_icon(space.icon_key.unwrap_or_default().into());
                self.window.set_space_draft_accent(space.accent_key.into());
                self.space_original = Some(self.space_draft_values());
                self.report("", false);
                self.window.set_space_dialog_open(true);
            }
            "save" => self.save_space(),
            "cancel" => {
                if self.space_dirty() {
                    self.ask_confirmation(
                        "Discard space draft?",
                        "No saved content will be changed.",
                        "Discard",
                        false,
                        Confirmation::CancelSpace,
                    );
                } else {
                    self.window.set_space_dialog_open(false);
                    self.space_original = None;
                    self.space_edit_id = None;
                }
            }
            "delete" => {
                let Some(space) = self.spaces.iter().find(|s| s.id == id).cloned() else {
                    return;
                };
                if id.is_system() {
                    self.report("System spaces cannot be deleted", true);
                    return;
                }
                self.ask_confirmation(&format!("Delete ‘{}’ space?",space.title),
                    "Items used only in this space will move to Favorites. Shared items remain in their other spaces. No saved content is deleted.",
                    "Delete space",true,Confirmation::DeleteSpace(id,space.revision));
            }
            "up" | "down" => self.space_mutation(
                id,
                SpaceAction::MoveSpace(if action == "up" { -1 } else { 1 }),
            ),
            "menu" => {
                self.picker = Picker::SpaceMenu;
                let mut actions = if self.surface.space == SpaceId::HISTORY {
                    vec![("clear", "Clear unpinned history", true)]
                } else {
                    vec![
                        ("new-item", "New content", false),
                        ("existing", "Add existing saved content", false),
                    ]
                };
                if !self.surface.space.is_system() {
                    actions.extend([
                        ("edit-space", "Edit this space", false),
                        ("delete-space", "Delete this space", true),
                    ]);
                }
                self.open_actions("Space options", actions);
            }
            "navigator" => {
                self.picker = Picker::Navigator;
                let actions = self
                    .spaces
                    .iter()
                    .map(|s| crate::ActionVm {
                        key: s.id.to_string().into(),
                        label: s.title.clone().into(),
                        detail: format!("{} items", s.item_count).into(),
                        danger: false,
                        enabled: true,
                    })
                    .collect();
                self.show_actions("Choose space", actions, false);
            }
            "add-existing" => {
                if id != SpaceId::HISTORY {
                    self.picker = Picker::Catalog(id);
                    self.show_actions("Add existing saved content", Vec::new(), true);
                    self.picker_query(String::new());
                }
            }
            _ => {}
        }
    }
    fn space_draft_values(&self) -> (String, String, String, String) {
        (
            self.window.get_space_draft_title().to_string(),
            self.window.get_space_draft_description().to_string(),
            self.window.get_space_draft_icon().to_string(),
            self.window.get_space_draft_accent().to_string(),
        )
    }
    fn space_dirty(&self) -> bool {
        if !self.window.get_space_dialog_open() {
            return false;
        }
        let values = self.space_draft_values();
        self.space_original.as_ref().map_or(
            !values.0.trim().is_empty() || !values.1.trim().is_empty(),
            |old| old != &values,
        )
    }
    fn save_space(&mut self) {
        let (title, description, icon, accent_key) = self.space_draft_values();
        let draft = SpaceDraft {
            title,
            description,
            icon_key: formatting::optional(&icon),
            accent_key,
        };
        let draft = match draft.normalize() {
            Ok(value) => value,
            Err(e) => {
                self.report(e.to_string(), true);
                return;
            }
        };
        if let Some((id, revision)) = self.space_edit_id {
            self.space_mutation_at(id, revision, SpaceAction::Update(draft));
        } else {
            let request_id = format!(
                "desktop:{}:{}:{}",
                std::process::id(),
                self.session.epoch,
                self.serial.wrapping_add(1)
            );
            self.mutate(Mutation::Space(SpaceCommand {
                space_id: None,
                expected_revision: None,
                request_id,
                action: SpaceAction::Create(draft),
            }));
        }
    }
    pub(super) fn close_picker(&mut self) {
        self.inspect_intent = None;
        self.window.set_picker_open(false);
        self.window.set_picker_loading(false);
        self.window.set_picker_more(false);
        self.window.set_picker_actions(ModelRc::default());
        self.picker = Picker::Closed;
        self.picker_generation = self.picker_generation.wrapping_add(1);
        self.picker_items.clear();
        self.picker_cursor = None;
    }
    fn show_actions(&mut self, title: &str, actions: Vec<crate::ActionVm>, searchable: bool) {
        self.finish_motion();
        self.window.set_picker_title(title.into());
        self.window.set_picker_searchable(searchable);
        self.window
            .set_picker_actions(ModelRc::new(VecModel::from(actions)));
        self.window.set_picker_query("".into());
        self.window.set_picker_more(false);
        self.window.set_picker_loading(false);
        self.report("", false);
        self.window.set_picker_open(true);
    }
    fn open_actions(&mut self, title: &str, actions: Vec<(&str, &str, bool)>) {
        self.show_actions(
            title,
            actions
                .into_iter()
                .map(|(key, label, danger)| crate::ActionVm {
                    key: key.into(),
                    label: label.into(),
                    detail: Default::default(),
                    danger,
                    enabled: true,
                })
                .collect(),
            false,
        );
    }
    pub(super) fn item_options(&mut self, key: RowKey) {
        self.picker = Picker::ItemMenu(key);
        let actions = if key.source == QuickInsertSource::History {
            vec![
                ("move", "Move into a space…", false),
                ("delete", "Delete this capture", true),
            ]
        } else {
            vec![
                ("share", "Add to another space…", false),
                ("duplicate", "Create an independent copy", false),
                ("remove", "Remove from this space", false),
                ("delete", "Delete everywhere…", true),
            ]
        };
        self.open_actions("Item options", actions);
    }
    fn target_picker(&mut self, key: RowKey) {
        self.picker = Picker::Target(key);
        let actions = self
            .spaces
            .iter()
            .filter(|s| s.id != SpaceId::HISTORY)
            .map(|s| crate::ActionVm {
                key: s.id.to_string().into(),
                label: s.title.clone().into(),
                detail: Default::default(),
                danger: false,
                enabled: true,
            })
            .collect();
        self.show_actions(
            if key.source == QuickInsertSource::History {
                "Move history into a space"
            } else {
                "Add shared content to a space"
            },
            actions,
            false,
        );
    }
    pub(super) fn picker_select(&mut self, key: &str) {
        if self.mutation.is_some() {
            return;
        }
        if key == "cancel" {
            self.close_picker();
            self.window.invoke_focus_search(false);
            return;
        }
        match self.picker.clone() {
            Picker::Navigator => {
                if let Some(id) = SpaceId::parse(key) {
                    self.close_picker();
                    self.navigate_to(id);
                }
            }
            Picker::SpaceMenu => match key {
                "clear" => {
                    self.close_picker();
                    self.ask_confirmation("Clear clipboard history?","This removes unpinned captures. Pinned History and saved content are kept.","Clear unpinned",true,Confirmation::ClearHistory);
                }
                "new-item" => {
                    self.close_picker();
                    self.new_item();
                }
                "existing" => self.space_action("add-existing", ""),
                "edit-space" => self.space_action("edit", ""),
                "delete-space" => self.space_action("delete", ""),
                _ => {}
            },
            Picker::ItemMenu(item) => match key {
                "move" | "share" => self.target_picker(item),
                "duplicate" => {
                    self.edit_created_copy = true;
                    self.space_mutation(self.surface.space, SpaceAction::DuplicateItem(item.id));
                }
                "remove" => {
                    self.space_mutation(self.surface.space, SpaceAction::RemoveItem(item.id))
                }
                "delete" => {
                    if item.source == QuickInsertSource::Favorite {
                        self.inspect_item("delete", item);
                    } else {
                        self.ask_confirmation(
                            "Delete this capture?",
                            "Only this History record will be removed. Saved copies are kept.",
                            "Delete capture",
                            true,
                            Confirmation::DeleteItem(item),
                        );
                    }
                }
                _ => {}
            },
            Picker::Target(item) => {
                if let Some(id) = SpaceId::parse(key).filter(|id| *id != SpaceId::HISTORY) {
                    self.space_mutation(
                        id,
                        if item.source == QuickInsertSource::History {
                            SpaceAction::MoveHistory(vec![item.id])
                        } else {
                            SpaceAction::AddItems(vec![item.id])
                        },
                    );
                }
            }
            Picker::Catalog(target) => {
                if let Some(item) = self
                    .picker_items
                    .iter()
                    .find(|item| RowKey::of(item).to_string() == key)
                {
                    self.space_mutation(target, SpaceAction::AddItems(vec![item.id]));
                }
            }
            Picker::Closed => {}
        }
    }
    pub(super) fn picker_query(&mut self, query: String) {
        if !matches!(self.picker, Picker::Catalog(_)) || query.len() > 16 * 1024 {
            return;
        }
        self.picker_generation = self.picker_generation.wrapping_add(1);
        self.picker_items.clear();
        self.picker_cursor = None;
        self.window.set_picker_actions(ModelRc::default());
        self.window.set_picker_query(query.clone().into());
        self.window.set_picker_loading(true);
        self.window.set_picker_more(false);
        if !self.send(Work::Catalog(self.picker_generation, query, None)) {
            self.window.set_picker_loading(false);
        }
    }
    pub(super) fn picker_more(&mut self) {
        if !matches!(self.picker, Picker::Catalog(_)) || self.window.get_picker_loading() {
            return;
        }
        let Some(cursor) = self.picker_cursor else {
            return;
        };
        self.picker_generation = self.picker_generation.wrapping_add(1);
        self.window.set_picker_loading(true);
        if !self.send(Work::Catalog(
            self.picker_generation,
            self.window.get_picker_query().to_string(),
            Some(cursor),
        )) {
            self.window.set_picker_loading(false);
        }
    }
    pub(super) fn catalog_loaded(
        &mut self,
        generation: u64,
        result: Result<QuickInsertPage, String>,
    ) {
        if generation != self.picker_generation
            || !matches!(self.picker, Picker::Catalog(_))
            || !self.window.get_picker_open()
        {
            return;
        }
        self.window.set_picker_loading(false);
        match result {
            Ok(page) => {
                self.picker_cursor = page.next_cursor;
                self.picker_items = page.items;
                let actions = self
                    .picker_items
                    .iter()
                    .map(|item| crate::ActionVm {
                        key: RowKey::of(item).to_string().into(),
                        label: item
                            .name
                            .as_deref()
                            .unwrap_or("Saved content")
                            .chars()
                            .take(54)
                            .collect::<String>()
                            .into(),
                        detail: item.content_type.clone().into(),
                        danger: false,
                        enabled: true,
                    })
                    .collect::<Vec<_>>();
                self.window
                    .set_picker_actions(ModelRc::new(VecModel::from(actions)));
                self.window.set_picker_more(self.picker_cursor.is_some());
                if self.picker_items.is_empty() {
                    self.report("No matching saved content", false);
                }
            }
            Err(e) => self.report(e, true),
        }
    }
    pub(super) fn inspect_item(&mut self, action: &str, key: RowKey) {
        self.serial = self.serial.wrapping_add(1);
        let generation = self.serial;
        self.inspect_intent = Some((generation, action.into(), key));
        self.send(Work::Inspect(generation, key.id));
    }
    pub(super) fn inspected(
        &mut self,
        generation: u64,
        result: Result<crate::events::ItemDetails, String>,
    ) {
        let Some((expected, action, key)) = self.inspect_intent.clone() else {
            return;
        };
        if generation != expected {
            return;
        }
        self.inspect_intent = None;
        if !self.surface.visible {
            return;
        }
        match result {
            Ok(details)=>match action.as_str() {
                "edit"=>{self.close_picker();self.open_editor(Some(details.item),details.spaces.len());},
                "delete"=>self.ask_confirmation("Delete saved content everywhere?",
                    &format!("This item is used in {} space(s). Its saved content and all memberships will be removed. This cannot be undone.",details.spaces.len()),
                    "Delete everywhere",true,Confirmation::DeleteItem(key)),_=>{},
            },Err(e)=>self.report(e,true),
        }
    }
    pub(super) fn new_item(&mut self) {
        if self.surface.space == SpaceId::HISTORY {
            self.report(
                "Choose Favorites or a custom space to create content",
                false,
            );
            return;
        }
        if self.mutation.is_some() || self.session.busy() || self.window.get_modal() {
            return;
        }
        self.finish_motion();
        self.open_editor(None, 1);
    }
    fn open_editor(&mut self, item: Option<QuickInsertItem>, memberships: usize) {
        self.window.set_editor_new(item.is_none());
        self.editor_key = item.as_ref().map(RowKey::of);
        self.window.set_editing_key(
            self.editor_key
                .map(|k| k.to_string())
                .unwrap_or_default()
                .into(),
        );
        self.window.set_draft_name(
            item.as_ref()
                .and_then(|i| i.name.clone())
                .unwrap_or_default()
                .into(),
        );
        self.window.set_draft_content(
            item.as_ref()
                .and_then(|i| i.editable_text.clone().or_else(|| i.preview_text.clone()))
                .unwrap_or_default()
                .into(),
        );
        self.window.set_draft_tags(
            item.as_ref()
                .map(|i| i.tags.join(", "))
                .unwrap_or_default()
                .into(),
        );
        self.window.set_draft_icon(
            item.as_ref()
                .and_then(|i| i.icon_key.clone())
                .unwrap_or_default()
                .into(),
        );
        self.window
            .set_content_editable(item.as_ref().is_none_or(|i| i.editable_text.is_some()));
        self.window.set_editor_sharing(if memberships>1 {format!("Used in {memberships} spaces. Edits update all of them. Use Item options → Independent copy to edit separately.").into()}else{"".into()});
        self.editor_original = Some(self.editor_values());
        self.report("", false);
        self.window.set_editor_open(true);
    }
    fn editor_values(&self) -> (String, String, String, String) {
        (
            self.window.get_draft_name().to_string(),
            self.window.get_draft_content().to_string(),
            self.window.get_draft_tags().to_string(),
            self.window.get_draft_icon().to_string(),
        )
    }
    fn editor_dirty(&self) -> bool {
        self.window.get_editor_open()
            && self
                .editor_original
                .as_ref()
                .is_some_and(|original| original != &self.editor_values())
    }
    fn close_editor(&mut self) {
        self.window.set_editor_open(false);
        self.editor_key = None;
        self.editor_original = None;
        self.inspect_intent = None;
        self.window.set_draft_content("".into());
        self.window.set_draft_name("".into());
        self.window.set_draft_tags("".into());
        if self.window.get_route().as_str() == "history" {
            self.window.invoke_focus_search(false);
        }
    }
    pub(super) fn cancel_editor(&mut self) {
        if self.mutation.is_some() {
            return;
        }
        if self.editor_dirty() {
            self.ask_confirmation(
                "Discard content draft?",
                "Saved content will not change.",
                "Discard",
                false,
                Confirmation::CancelEditor,
            );
        } else {
            self.close_editor();
        }
    }
    pub(super) fn save_favorite(&mut self) {
        if !self.window.get_editor_open() || self.mutation.is_some() {
            return;
        }
        let (name, content, tags, icon) = self.editor_values();
        let name = formatting::optional(&name);
        let tags = formatting::tags(&tags);
        let icon_key = formatting::optional(&icon);
        if let Some(key) = self.editor_key {
            self.mutate(Mutation::Update(
                key.id,
                FavoriteUpdate {
                    name,
                    tags,
                    icon_key,
                    editable_text: self.window.get_content_editable().then_some(content),
                },
            ));
        } else {
            self.space_mutation(
                self.surface.space,
                SpaceAction::CreateItem(FavoriteDraft {
                    content,
                    name,
                    tags,
                    icon_key,
                }),
            );
        }
    }
}
