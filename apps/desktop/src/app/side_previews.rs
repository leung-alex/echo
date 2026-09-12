//! Two small, read-only neighbor projections share the central query generation.
//! Searches use complete documents; only their first four display rows are retained.
use super::*;
use std::hash::{Hash, Hasher};

#[derive(Hash)]
pub(super) struct PreviewError {
    query_epoch: u64,
    revision: i64,
}

impl App {
    fn software_side_ids(&self) -> Vec<SpaceId> {
        let w = &self.window;
        let room = w.get_side_width() + 28.0;
        [
            (w.get_requested_left_space(), w.get_panel_left() >= room),
            (
                w.get_requested_right_space(),
                w.get_stage_width() - w.get_panel_left() - w.get_panel_width() >= room,
            ),
        ]
        .into_iter()
        .filter(|(_, visible)| *visible)
        .filter_map(|(side, _)| {
            self.spaces
                .iter()
                .find(|s| s.id.to_string() == side.key.as_str())
                .map(|s| s.id)
        })
        .collect()
    }
    pub(super) fn prepare_software_neighbors(&mut self) {
        if !self.surface.visible
            || !self.surface.ready
            || self.surface.loading
            || self.surface.dirty
            || self.software.slide.moving()
            || self.window.get_route().as_str() != "history"
            || self.window.get_modal()
        {
            return;
        }
        let ids = self.software_side_ids();
        self.previews.retain(|id, _| ids.contains(id));
        self.software.side_errors.retain(|id, error| {
            ids.contains(id)
                && error.query_epoch == self.surface.query_epoch()
                && self
                    .spaces
                    .iter()
                    .any(|s| s.id == *id && s.revision == error.revision)
        });
        self.pending_previews.retain(|id| ids.contains(id));
        for id in ids {
            if !self.has_current_preview(id)
                && !self.software.side_errors.contains_key(&id)
                && self.pending_previews.insert(id)
            {
                if !self.send(Work::SidePreview(
                    id,
                    self.preview_epoch,
                    self.surface.query.clone(),
                )) {
                    self.pending_previews.remove(&id);
                    self.set_software_preview_error(id);
                }
            }
        }
        if !self.software.slide.loading() {
            self.render_software_side_previews();
        }
    }
    pub(super) fn software_neighbors_ready(&self) -> bool {
        self.software_side_ids()
            .iter()
            .all(|id| self.has_current_preview(*id) || self.software.side_errors.contains_key(id))
    }
    fn set_software_preview_error(&mut self, id: SpaceId) {
        self.previews.remove(&id);
        if let Some(space) = self.spaces.iter().find(|s| s.id == id) {
            self.software.side_errors.insert(
                id,
                PreviewError {
                    query_epoch: self.surface.query_epoch(),
                    revision: space.revision,
                },
            );
        }
    }
    pub(super) fn software_preview_loaded(
        &mut self,
        id: SpaceId,
        epoch: u64,
        result: Result<crate::events::LoadedPage, String>,
    ) {
        if epoch != self.preview_epoch
            || !self.surface.visible
            || !self.software_side_ids().contains(&id)
        {
            return;
        }
        self.pending_previews.remove(&id);
        match result {
            Ok(data) => {
                self.software.side_errors.remove(&id);
                if self
                    .spaces
                    .iter()
                    .find(|s| s.id == id)
                    .is_none_or(|s| s.revision != data.revision)
                {
                    self.send(Work::Spaces);
                    return;
                }
                let hashes: Vec<_> = data
                    .page
                    .items
                    .iter()
                    .filter_map(|i| i.thumbnail.as_ref().map(|t| t.content_hash.clone()))
                    .collect();
                self.previews.insert(
                    id,
                    Preview {
                        query: self.surface.query.clone(),
                        items: data.page.items,
                        revision: data.revision,
                        total: data.total,
                    },
                );
                for hash in hashes {
                    if !self.images.cache.contains_key(&hash)
                        && self.images.pending.insert(hash.clone())
                    {
                        if !self.send(Work::Thumbnail(self.images.epoch, hash.clone())) {
                            self.images.pending.remove(&hash);
                        }
                    }
                }
            }
            Err(_) => {
                self.set_software_preview_error(id);
            }
        }
        if !self.software.slide.loading() {
            self.render_software_side_previews();
        }
        self.software_content_ready();
    }
    pub(super) fn make_side_preview(
        &self,
        items: &[QuickInsertItem],
        loading: bool,
        message: &str,
    ) -> (crate::SidePreview, usize) {
        echo_windows::allocation::measure_owned(|| {
            let rows: Vec<_> = items
                .iter()
                .take(4)
                .map(|item| crate::SidePreviewRow {
                    title: item
                        .name
                        .as_deref()
                        .unwrap_or("")
                        .chars()
                        .take(160)
                        .collect::<String>()
                        .into(),
                    body: item
                        .preview_text
                        .as_deref()
                        .unwrap_or(&item.content_type)
                        .chars()
                        .take(384)
                        .collect::<String>()
                        .into(),
                    image: item
                        .thumbnail
                        .as_ref()
                        .and_then(|t| self.images.cache.get(&t.content_hash))
                        .map(|(image, _)| image.clone())
                        .unwrap_or_default(),
                })
                .collect();
            crate::SidePreview {
                rows: ModelRc::new(VecModel::from(rows)),
                loading,
                message: message.into(),
            }
        })
    }
    pub(super) fn render_software_side_previews(&mut self) {
        if !self.surface.visible {
            return;
        }
        for (index, mut side) in [self.window.get_left_space(), self.window.get_right_space()]
            .into_iter()
            .enumerate()
        {
            let id = self
                .spaces
                .iter()
                .find(|s| s.id.to_string() == side.key.as_str())
                .map(|s| s.id);
            let preview = id
                .filter(|id| self.has_current_preview(*id))
                .and_then(|id| self.previews.get(&id));
            let error = id.and_then(|id| self.software.side_errors.get(&id));
            let loading = id.is_some_and(|id| !self.has_current_preview(id)) && error.is_none();
            if error.is_some() {
                side.count = "".into();
            }
            if !loading {
                if let Some(p) = preview {
                    side.count = p.total.to_string().into();
                }
            }
            let mut signature = std::collections::hash_map::DefaultHasher::new();
            side.key.as_str().hash(&mut signature);
            self.surface.query_epoch().hash(&mut signature);
            self.software.image_version.hash(&mut signature);
            self.images.epoch.hash(&mut signature);
            loading.hash(&mut signature);
            error.hash(&mut signature);
            if let Some(p) = preview {
                p.query.hash(&mut signature);
                p.revision.hash(&mut signature);
            }
            let signature = signature.finish();
            if index == 0 {
                self.window.set_left_space(side.clone());
            } else {
                self.window.set_right_space(side.clone());
            }
            if self.software.side_signatures[index] == Some(signature) {
                continue;
            }
            let message = if error.is_some() {
                "Preview unavailable"
            } else if self.surface.query.is_empty() {
                "No items in this space"
            } else {
                "No matches in this space"
            };
            let (view, bytes) =
                self.make_side_preview(preview.map_or(&[], |p| &p.items), loading, message);
            if index == 0 {
                self.window.set_left_space(side);
                self.window.set_left_preview(view);
            } else {
                self.window.set_right_space(side);
                self.window.set_right_preview(view);
            }
            self.software.side_model_bytes[index] = bytes;
            self.software.side_signatures[index] = Some(signature);
        }
    }
    pub(super) fn clear_software_side_models(&mut self) {
        self.window.set_left_preview(Default::default());
        self.window.set_right_preview(Default::default());
        self.software.side_model_bytes = [0; 2];
        self.software.side_signatures = [None; 2];
    }
    pub(super) fn software_side_bytes(&self) -> usize {
        self.software.side_model_bytes.iter().sum::<usize>()
            + self
                .previews
                .values()
                .map(|p| {
                    p.query.capacity()
                        + p.items
                            .iter()
                            .map(QuickInsertItem::held_bytes)
                            .sum::<usize>()
                        + (p.items.capacity() - p.items.len())
                            * std::mem::size_of::<QuickInsertItem>()
                })
                .sum::<usize>()
    }
}
