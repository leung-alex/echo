//! Space identity, latest-intent motion and insertion barriers. No UI or GPU types.
use echo_engine::SpaceId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Animating,
    AwaitingContent,
    Suspended,
}
#[derive(Debug)]
pub struct Navigation {
    order: Vec<SpaceId>,
    pub requested: SpaceId,
    pub presented: SpaceId,
    pub interaction: Option<SpaceId>,
    pub phase: Phase,
}
impl Default for Navigation {
    fn default() -> Self {
        Self {
            order: vec![SpaceId::HISTORY, SpaceId::FAVORITES],
            requested: SpaceId::HISTORY,
            presented: SpaceId::HISTORY,
            interaction: None,
            phase: Phase::Suspended,
        }
    }
}
impl Navigation {
    pub fn order(&self) -> &[SpaceId] {
        &self.order
    }
    pub fn index(&self, id: SpaceId) -> Option<usize> {
        self.order.iter().position(|x| *x == id)
    }
    pub fn set_order(&mut self, ids: impl IntoIterator<Item = SpaceId>) {
        let mut order = Vec::new();
        for id in ids {
            if id.0 > 0 && !order.contains(&id) {
                order.push(id);
            }
        }
        if order.is_empty() {
            order.push(SpaceId::HISTORY);
        }
        if self.order == order {
            return;
        }
        self.order = order;
        if self.index(self.requested).is_none() {
            self.requested = SpaceId::HISTORY;
            if self.index(self.requested).is_none() {
                self.requested = self.order[0];
            }
        }
        self.presented = self.requested;
        self.interaction = None;
        if self.phase != Phase::Suspended {
            self.phase = Phase::AwaitingContent;
        }
    }
    pub fn show(&mut self, id: SpaceId) {
        let id = if self.index(id).is_some() {
            id
        } else {
            self.order[0]
        };
        self.requested = id;
        self.presented = id;
        self.interaction = None;
        self.phase = Phase::AwaitingContent;
    }
    pub fn hide(&mut self) {
        self.snap();
        self.interaction = None;
        self.phase = Phase::Suspended;
    }
    pub fn block_content(&mut self) {
        self.interaction = None;
        if self.phase == Phase::Idle {
            self.phase = Phase::AwaitingContent;
        }
    }
    pub fn ready(&mut self, id: SpaceId) {
        if id == self.requested && self.phase == Phase::AwaitingContent {
            self.presented = id;
            self.interaction = Some(id);
            self.phase = Phase::Idle;
        }
    }
    pub fn request(&mut self, id: SpaceId) -> bool {
        if self.index(id).is_none() || id == self.requested || self.phase == Phase::Suspended {
            return false;
        }
        self.requested = id;
        self.snap();
        true
    }
    pub fn snap(&mut self) {
        self.presented = self.requested;
        self.interaction = None;
        if self.phase != Phase::Suspended {
            self.phase = Phase::AwaitingContent;
        }
    }
    pub fn can_insert(&self, id: SpaceId) -> bool {
        self.phase == Phase::Idle
            && self.interaction == Some(id)
            && self.presented == id
            && self.requested == id
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readiness_and_latest_navigation_protect_insertion() {
        let mut deck = Navigation::default();
        deck.show(SpaceId::HISTORY);
        deck.ready(SpaceId::HISTORY);
        assert!(deck.can_insert(SpaceId::HISTORY));
        assert!(deck.request(SpaceId::FAVORITES));
        deck.ready(SpaceId::HISTORY);
        assert!(!deck.can_insert(SpaceId::HISTORY));
        assert!(!deck.can_insert(SpaceId::FAVORITES));
        deck.ready(SpaceId::FAVORITES);
        assert!(deck.can_insert(SpaceId::FAVORITES));
        deck.hide();
        deck.ready(SpaceId::FAVORITES);
        assert!(!deck.can_insert(SpaceId::FAVORITES));
        assert!(!deck.request(SpaceId::HISTORY));
    }
    #[test]
    fn deleted_space_and_duplicate_order_remain_safe() {
        let mut deck = Navigation::default();
        deck.set_order([SpaceId::HISTORY, SpaceId(3), SpaceId(3)]);
        assert_eq!(deck.order().len(), 2);
        deck.show(SpaceId(3));
        deck.set_order([SpaceId::HISTORY]);
        assert_eq!(deck.requested, SpaceId::HISTORY);
        assert!(!deck.request(SpaceId(9)));
        deck.set_order([]);
        assert_eq!(deck.order(), &[SpaceId::HISTORY]);
    }
}
