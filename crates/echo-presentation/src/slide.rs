//! Ready-before-motion translation and a single latest pending destination.
//! This owns no models, timers, renderer resources or native input state.
use echo_engine::SpaceId;

/// Map logical neighbors into the physical slot of an anchored popup.
pub fn popup_neighbors<T>(
    left: Option<T>,
    right: Option<T>,
    side_right: Option<bool>,
) -> (Option<T>, Option<T>) {
    match side_right {
        Some(true) => (None, right.or(left)),
        Some(false) => (left.or(right), None),
        None => (left, right),
    }
}

#[cfg(test)]
mod popup_neighbor_tests {
    use super::popup_neighbors;

    #[test]
    fn carousel_keeps_other_space_in_available_slot() {
        assert_eq!(popup_neighbors(Some(1), None, Some(true)), (None, Some(1)));
        assert_eq!(popup_neighbors(None, Some(2), Some(false)), (Some(2), None));
        assert_eq!(popup_neighbors(Some(1), Some(3), None), (Some(1), Some(3)));
        assert_eq!(popup_neighbors(None::<u8>, None, Some(true)), (None, None));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentFrame {
    pub preparation: u64,
    pub session: u64,
    pub query: u64,
    pub revision: i64,
    pub space: SpaceId,
}

/// Admission policy for a prepared navigation frame. The presentation layer
/// owns the transition contract; the desktop adapter supplies the count of
/// pending display resources without moving Slint or decoder state here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NavigationReadiness {
    #[default]
    RequireComplete,
    AllowPendingImages,
}

impl NavigationReadiness {
    pub fn permits_pending(self) -> bool {
        matches!(self, Self::AllowPendingImages)
    }

    pub fn gate_open(self, pending_count: usize) -> bool {
        self.permits_pending() || pending_count == 0
    }
}

/// Matches the retained GPU version's visible side-card width, without perspective.
pub const SIDE_WIDTH_RATIO: f32 = 0.7;
pub fn side_width(main_width: f32) -> f32 {
    main_width * SIDE_WIDTH_RATIO
}
pub const SIDE_GAP: f32 = 12.0;
pub const SIDE_HEIGHT_INSET: f32 = 16.0;

/// Snappy card transitions always complete in 140 ms.
pub const DURATION_MS: u64 = 140;

/// Cubic ease-out: monotonic, no spring or overshoot, exact finite end.
pub fn progress(elapsed_ms: u64, duration_ms: u64) -> f32 {
    if duration_ms == 0 || elapsed_ms >= duration_ms {
        return 1.0;
    }
    let remaining = 1.0 - elapsed_ms as f32 / duration_ms as f32;
    1.0 - remaining * remaining * remaining
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    Unchanged,
    /// Replace the incoming load; retain an existing outgoing panel if present.
    Load {
        retain_outgoing: bool,
    },
    Queued,
}

#[derive(Debug)]
struct Transition {
    from: SpaceId,
    to: SpaceId,
    direction: f32,
    started_ms: Option<u64>,
    duration_ms: u64,
}

#[derive(Debug, Default)]
pub struct Slide {
    active: Option<Transition>,
    pending: Option<SpaceId>,
}

impl Slide {
    pub fn target(&self) -> Option<SpaceId> {
        self.active.as_ref().map(|t| t.to)
    }
    pub fn intent(&self) -> Option<SpaceId> {
        self.pending.or_else(|| self.target())
    }
    pub fn source(&self) -> Option<SpaceId> {
        self.active.as_ref().map(|t| t.from)
    }
    pub fn moving(&self) -> bool {
        self.active.as_ref().is_some_and(|t| t.started_ms.is_some())
    }
    pub fn loading(&self) -> bool {
        self.active.as_ref().is_some_and(|t| t.started_ms.is_none())
    }
    pub fn request(&mut self, from: SpaceId, to: SpaceId, direction: f32) -> Request {
        if self.moving() {
            // A request back to the active destination cancels an older pending one.
            self.pending = (self.target() != Some(to)).then_some(to);
            return Request::Queued;
        }
        if self.target() == Some(to) || (self.active.is_none() && from == to) {
            return Request::Unchanged;
        }
        let retain_outgoing = self.active.is_some();
        let from = self.source().unwrap_or(from);
        self.active = Some(Transition {
            from,
            to,
            direction: if direction < 0.0 { -1.0 } else { 1.0 },
            started_ms: None,
            duration_ms: 0,
        });
        self.pending = None;
        Request::Load { retain_outgoing }
    }
    /// A stale completion cannot start a transition. The caller also checks its
    /// query ticket and data version before publishing the incoming model.
    pub fn ready(&mut self, to: SpaceId, now_ms: u64, motion: bool) -> bool {
        let Some(active) = &mut self.active else {
            return false;
        };
        if active.to != to || active.started_ms.is_some() {
            return false;
        }
        active.started_ms = Some(now_ms);
        active.duration_ms = if motion && active.from != to {
            DURATION_MS
        } else {
            0
        };
        true
    }
    pub fn offsets(&self, now_ms: u64, width: f32) -> Option<[f32; 2]> {
        let active = self.active.as_ref()?;
        let p = progress(
            now_ms.saturating_sub(active.started_ms?),
            active.duration_ms,
        );
        Some([
            -active.direction * width * p,
            active.direction * width * (1.0 - p),
        ])
    }
    pub fn motion(&self, now_ms: u64) -> Option<(f32, f32)> {
        let active = self.active.as_ref()?;
        Some((
            progress(
                now_ms.saturating_sub(active.started_ms?),
                active.duration_ms,
            ),
            active.direction,
        ))
    }
    pub fn finished(&self, now_ms: u64) -> bool {
        self.active.as_ref().is_some_and(|t| {
            t.started_ms
                .is_some_and(|start| now_ms.saturating_sub(start) >= t.duration_ms)
        })
    }
    /// Release the two-panel transition before admitting the last pending load.
    pub fn finish(&mut self) -> Option<SpaceId> {
        self.active = None;
        self.pending.take()
    }
    pub fn cancel(&mut self) {
        self.active = None;
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_readiness_allows_pending_images_only_for_transitions() {
        assert!(NavigationReadiness::AllowPendingImages.gate_open(3));
        assert!(NavigationReadiness::RequireComplete.gate_open(0));
        assert!(!NavigationReadiness::RequireComplete.gate_open(1));
    }

    #[test]
    fn loading_and_stale_results_never_start_motion() {
        let mut slide = Slide::default();
        assert_eq!(
            slide.request(SpaceId(1), SpaceId(2), 1.0),
            Request::Load {
                retain_outgoing: false
            }
        );
        assert_eq!(slide.offsets(1000, 520.0), None);
        assert_eq!(
            slide.request(SpaceId(1), SpaceId(3), 1.0),
            Request::Load {
                retain_outgoing: true
            }
        );
        assert!(!slide.ready(SpaceId(2), 1000, true));
        assert!(slide.ready(SpaceId(3), 1000, true));
        assert_eq!(slide.offsets(1000, 520.0), Some([0.0, 520.0]));
        assert!(!slide.finished(1139));
        assert!(slide.finished(1140));
    }
    #[test]
    fn active_motion_finishes_before_only_the_latest_pending_destination() {
        let mut slide = Slide::default();
        slide.request(SpaceId(1), SpaceId(2), 1.0);
        slide.ready(SpaceId(2), 0, true);
        let before = slide.offsets(60, 520.0);
        for id in 3..=100 {
            assert_eq!(slide.request(SpaceId(2), SpaceId(id), 1.0), Request::Queued);
        }
        assert_eq!(slide.offsets(60, 520.0), before);
        assert_eq!(slide.target(), Some(SpaceId(2)));
        assert_eq!(slide.intent(), Some(SpaceId(100)));
        assert_eq!(slide.finish(), Some(SpaceId(100)));
        assert_eq!(slide.offsets(200, 520.0), None);
        assert_eq!(slide.finish(), None);
    }
    #[test]
    fn repeated_active_target_cancels_pending_and_hide_cancels_everything() {
        let mut slide = Slide::default();
        slide.request(SpaceId(1), SpaceId(2), 1.0);
        slide.ready(SpaceId(2), 0, true);
        slide.request(SpaceId(2), SpaceId(3), 1.0);
        slide.request(SpaceId(2), SpaceId(2), 1.0);
        assert_eq!(slide.finish(), None);
        slide.request(SpaceId(2), SpaceId(3), 1.0);
        slide.cancel();
        assert!(!slide.ready(SpaceId(3), 100, true));
        assert!(!slide.moving());
        assert!(!slide.loading());
    }
    #[test]
    fn snappy_reverse_travel_finishes_in_140_ms() {
        let mut slide = Slide::default();
        slide.request(SpaceId(2), SpaceId(1), -1.0);
        assert!(slide.ready(SpaceId(1), 900, true));
        assert!(!slide.finished(1039));
        assert!(slide.finished(1040));
        assert_eq!(slide.offsets(1040, 520.0), Some([520.0, 0.0]));
        let mut previous = 0.0;
        for t in 0..=DURATION_MS + 1 {
            let p = progress(t, DURATION_MS);
            assert!((previous..=1.0).contains(&p));
            previous = p;
        }
    }
}
