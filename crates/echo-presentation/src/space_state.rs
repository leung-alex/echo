//! Bounded, memory-only per-space position hints; no row or payload duplication.
use crate::{RowKey, Surface};
use echo_engine::{PageCursor, SpaceId};
use std::collections::VecDeque;
#[derive(Clone)]
pub struct SpacePosition {
    pub space: SpaceId,
    pub query: String,
    pub selection: Option<RowKey>,
    pub scroll: f32,
    pub window_start: Option<PageCursor>,
    previous: Vec<Option<PageCursor>>,
}
#[derive(Default)]
pub struct SpacePositions(VecDeque<SpacePosition>);
impl SpacePositions {
    pub fn remember(&mut self, surface: &Surface, scroll: f32) {
        if !surface.ready {
            return;
        }
        self.0.retain(|p| p.space != surface.space);
        self.0.push_back(SpacePosition {
            space: surface.space,
            query: surface.query.clone(),
            selection: surface.selection,
            scroll: if scroll.is_finite() {
                scroll.min(0.0)
            } else {
                0.0
            },
            window_start: surface.window_start,
            previous: surface.previous_windows.clone(),
        });
        while self.0.len() > 3 {
            self.0.pop_front();
        }
    }
    pub fn restore(&self, surface: &mut Surface) -> f32 {
        let Some(position) = self
            .0
            .iter()
            .find(|p| p.space == surface.space && p.query == surface.query)
        else {
            return 0.0;
        };
        surface.selection = position.selection;
        surface.window_start = position.window_start;
        surface.previous_windows = position.previous.clone();
        position.scroll
    }
    pub fn remove(&mut self, id: SpaceId) {
        self.0.retain(|p| p.space != id);
    }
    pub fn retain(&mut self, ids: &[SpaceId]) {
        self.0.retain(|p| ids.contains(&p.space));
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
}
impl Surface {
    /// Identity decoding stays inside presentation. Callers only return opaque UI keys.
    pub fn resolve_key(&self, value: &str) -> Option<RowKey> {
        self.items
            .iter()
            .map(RowKey::of)
            .find(|key| key.to_string() == value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use echo_engine::QuickInsertView;
    #[test]
    fn positions_are_bounded_and_never_reused_for_another_query() {
        let mut p = SpacePositions::default();
        let mut s = Surface::new(QuickInsertView::Favorites);
        for id in 2..12 {
            s.space = SpaceId(id);
            s.ready = true;
            p.remember(&s, -100.0);
        }
        assert_eq!(p.0.len(), 3);
        assert_eq!(p.restore(&mut s), -100.0);
        s.query = "other".into();
        assert_eq!(p.restore(&mut s), 0.0);
    }
}
