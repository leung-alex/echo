//! Platform-neutral caret geometry evidence and arbitration.
//!
//! This module deliberately contains no Windows, COM, renderer, or paste
//! concerns. Native adapters convert their provider observations into these
//! validated candidates before the source-selection boundary.

use crate::{InputTargetGeometry, PhysicalRect};
use std::time::{Duration, Instant};

pub const GEOMETRY_TTL: Duration = Duration::from_millis(150);
pub const MODE_TTL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometrySource {
    TsfCaret,
    NativeCaret,
    UiaCaret,
    MsaaCaret,
    AdjacentCharacter,
    ImmExclusion,
    Control,
    Pointer,
}

impl GeometrySource {
    /// Sources eligible for the passive caret-follow badge, in precedence order.
    pub const fn priority(self) -> Option<u8> {
        match self {
            Self::TsfCaret => Some(0),
            Self::NativeCaret => Some(1),
            Self::UiaCaret => Some(2),
            Self::MsaaCaret => Some(3),
            Self::AdjacentCharacter => Some(4),
            Self::ImmExclusion | Self::Control | Self::Pointer => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryConfidence {
    Exact,
    Estimated,
    Fallback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryStamp {
    pub source: GeometrySource,
    pub confidence: GeometryConfidence,
    pub observed_at: Instant,
    pub sequence: u64,
    pub context_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometrySafety {
    Allowed,
    Denied,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GeometryIdentity {
    pub generation: u64,
    pub process: u32,
    pub process_started: u64,
    pub input_thread: u32,
    pub root_window: isize,
    pub focused_window: isize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryInvalidReason {
    GenerationOrIdentityMismatch,
    SensitivityDenied,
    SensitivityUnknown,
    UnsupportedSource,
    FutureTimestamp,
    GeometryStale,
    RequestMismatch,
    ContextMismatch,
    WrongViewOwner,
    Clipped,
    InterimCharacter,
    SelectionNotCollapsed,
    InvalidDpi,
    InvalidRectangle,
    Offscreen,
    OutsideControl,
}

#[derive(Clone, Copy, Debug)]
pub struct GeometryCandidate {
    pub identity: GeometryIdentity,
    pub source: GeometrySource,
    pub confidence: GeometryConfidence,
    pub geometry: InputTargetGeometry,
    pub observed_at: Instant,
    pub sequence: u64,
    pub context_epoch: u64,
    pub safety: GeometrySafety,
    pub clipped: bool,
    pub interim_character: bool,
    pub noncollapsed_selection: bool,
    pub view_verified: bool,
    pub control: Option<PhysicalRect>,
}

impl GeometryCandidate {
    pub const fn exact(
        identity: GeometryIdentity,
        source: GeometrySource,
        geometry: InputTargetGeometry,
        observed_at: Instant,
        sequence: u64,
        context_epoch: u64,
    ) -> Self {
        Self {
            identity,
            source,
            confidence: GeometryConfidence::Exact,
            geometry,
            observed_at,
            sequence,
            context_epoch,
            safety: GeometrySafety::Allowed,
            clipped: false,
            interim_character: false,
            noncollapsed_selection: false,
            view_verified: true,
            control: None,
        }
    }
}

pub fn invalid_reason(
    candidate: &GeometryCandidate,
    current: GeometryIdentity,
    now: Instant,
    context_epoch: u64,
) -> Option<GeometryInvalidReason> {
    if candidate.identity != current {
        return Some(GeometryInvalidReason::GenerationOrIdentityMismatch);
    }
    match candidate.safety {
        GeometrySafety::Allowed => {}
        GeometrySafety::Denied => return Some(GeometryInvalidReason::SensitivityDenied),
        GeometrySafety::Unknown => return Some(GeometryInvalidReason::SensitivityUnknown),
    }
    if candidate.source.priority().is_none() {
        return Some(GeometryInvalidReason::UnsupportedSource);
    }
    if candidate.observed_at > now {
        return Some(GeometryInvalidReason::FutureTimestamp);
    }
    if now.duration_since(candidate.observed_at) > GEOMETRY_TTL {
        return Some(GeometryInvalidReason::GeometryStale);
    }
    if candidate.sequence == 0 {
        return Some(GeometryInvalidReason::RequestMismatch);
    }
    if candidate.source == GeometrySource::TsfCaret && candidate.context_epoch != context_epoch {
        return Some(GeometryInvalidReason::ContextMismatch);
    }
    if !candidate.view_verified {
        return Some(GeometryInvalidReason::WrongViewOwner);
    }
    if candidate.clipped {
        return Some(GeometryInvalidReason::Clipped);
    }
    if candidate.interim_character {
        return Some(GeometryInvalidReason::InterimCharacter);
    }
    if candidate.noncollapsed_selection {
        return Some(GeometryInvalidReason::SelectionNotCollapsed);
    }

    let geometry = candidate.geometry;
    if !(48..=768).contains(&geometry.dpi) {
        return Some(GeometryInvalidReason::InvalidDpi);
    }
    let rect = geometry.target;
    let right = i64::from(rect.x) + i64::from(rect.width);
    let bottom = i64::from(rect.y) + i64::from(rect.height);
    let max_width = i64::from(geometry.dpi) * 64 / 96;
    let max_height = i64::from(geometry.dpi) * 256 / 96;
    if rect.width < 0
        || rect.height <= 0
        || right < i64::from(i32::MIN)
        || right > i64::from(i32::MAX)
        || bottom < i64::from(i32::MIN)
        || bottom > i64::from(i32::MAX)
        || i64::from(rect.width) > max_width
        || i64::from(rect.height) > max_height
    {
        return Some(GeometryInvalidReason::InvalidRectangle);
    }

    let work = geometry.work_area;
    let work_right = i64::from(work.x) + i64::from(work.width);
    let work_bottom = i64::from(work.y) + i64::from(work.height);
    if i64::from(rect.x) >= work_right
        || i64::from(rect.y) >= work_bottom
        || right < i64::from(work.x)
        || bottom <= i64::from(work.y)
    {
        return Some(GeometryInvalidReason::Offscreen);
    }

    if let Some(control) = candidate.control {
        let tolerance = i64::from(geometry.dpi) * 8 / 96;
        let control_right = i64::from(control.x) + i64::from(control.width);
        let control_bottom = i64::from(control.y) + i64::from(control.height);
        if i64::from(rect.x) < i64::from(control.x) - tolerance
            || i64::from(rect.y) < i64::from(control.y) - tolerance
            || right > control_right + tolerance
            || bottom > control_bottom + tolerance
        {
            return Some(GeometryInvalidReason::OutsideControl);
        }
    }
    None
}

/// Choose the highest-precedence fresh candidate. A zero-width caret is
/// normalized only after all validation, preserving its insertion position.
pub fn arbitrate_geometry(
    current: GeometryIdentity,
    candidates: impl IntoIterator<Item = GeometryCandidate>,
    now: Instant,
    context_epoch: u64,
) -> Option<GeometryCandidate> {
    candidates
        .into_iter()
        .filter(|candidate| invalid_reason(candidate, current, now, context_epoch).is_none())
        .min_by_key(|candidate| {
            (
                candidate.source.priority().unwrap_or(u8::MAX),
                std::cmp::Reverse(candidate.sequence),
            )
        })
        .map(|mut candidate| {
            candidate.geometry.target.width = candidate.geometry.target.width.max(1);
            candidate
        })
}

pub fn mode_is_fresh(mode_observed_at: Instant, now: Instant) -> bool {
    now >= mode_observed_at && now.duration_since(mode_observed_at) <= MODE_TTL
}

pub fn geometry_is_fresh(observed_at: Instant, now: Instant) -> bool {
    now >= observed_at && now.duration_since(observed_at) <= GEOMETRY_TTL
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompositionState, InputAnchor, InputMode, InputStatus};

    const ID: GeometryIdentity = GeometryIdentity {
        generation: 1,
        process: 10,
        process_started: 100,
        input_thread: 11,
        root_window: 20,
        focused_window: 21,
    };

    fn work() -> PhysicalRect {
        PhysicalRect {
            x: -1920,
            y: 0,
            width: 3840,
            height: 1080,
        }
    }

    fn candidate(now: Instant) -> GeometryCandidate {
        GeometryCandidate::exact(
            ID,
            GeometrySource::TsfCaret,
            InputTargetGeometry {
                target: PhysicalRect {
                    x: 100,
                    y: 100,
                    width: 1,
                    height: 20,
                },
                work_area: work(),
                dpi: 96,
            },
            now,
            1,
            1,
        )
    }

    #[test]
    fn tsf_precedes_newer_native_and_same_source_uses_latest_sequence() {
        let now = Instant::now();
        let mut tsf = candidate(now);
        let mut native = tsf;
        native.source = GeometrySource::NativeCaret;
        native.sequence = 2;
        native.geometry.target.x = 0;
        assert_eq!(
            arbitrate_geometry(ID, [native, tsf], now, 1)
                .unwrap()
                .source,
            GeometrySource::TsfCaret
        );
        tsf.sequence = 3;
        assert_eq!(
            arbitrate_geometry(ID, [candidate(now), tsf], now, 1)
                .unwrap()
                .sequence,
            3
        );
    }

    #[test]
    fn freshness_boundaries_and_mode_clock_are_independent() {
        let now = Instant::now();
        assert!(
            arbitrate_geometry(ID, [candidate(now - Duration::from_millis(150))], now, 1).is_some()
        );
        assert!(
            arbitrate_geometry(ID, [candidate(now - Duration::from_millis(151))], now, 1).is_none()
        );
        assert!(mode_is_fresh(now - Duration::from_millis(250), now));
        assert!(!geometry_is_fresh(now - Duration::from_millis(151), now));
    }

    #[test]
    fn invalid_identity_safety_and_diagnostic_sources_are_rejected() {
        let now = Instant::now();
        let mut c = candidate(now);
        c.identity.focused_window += 1;
        assert!(arbitrate_geometry(ID, [c], now, 1).is_none());
        let mut c = candidate(now);
        c.safety = GeometrySafety::Unknown;
        assert!(arbitrate_geometry(ID, [c], now, 1).is_none());
        for source in [
            GeometrySource::Control,
            GeometrySource::ImmExclusion,
            GeometrySource::Pointer,
        ] {
            let mut c = candidate(now);
            c.source = source;
            assert!(arbitrate_geometry(ID, [c], now, 1).is_none());
        }
    }

    #[test]
    fn selection_view_clip_dpi_and_rect_checks_fail_closed() {
        let now = Instant::now();
        let mut c = candidate(now);
        c.noncollapsed_selection = true;
        assert!(arbitrate_geometry(ID, [c], now, 1).is_none());
        let mut c = candidate(now);
        c.view_verified = false;
        assert!(arbitrate_geometry(ID, [c], now, 1).is_none());
        let mut c = candidate(now);
        c.geometry.dpi = 769;
        assert!(arbitrate_geometry(ID, [c], now, 1).is_none());
        let mut c = candidate(now);
        c.geometry.target.height = 0;
        assert!(arbitrate_geometry(ID, [c], now, 1).is_none());
    }

    #[test]
    fn zero_width_negative_monitor_and_control_tolerance() {
        let now = Instant::now();
        let mut c = candidate(now);
        c.geometry.target.width = 0;
        assert_eq!(
            arbitrate_geometry(ID, [c], now, 1)
                .unwrap()
                .geometry
                .target
                .width,
            1
        );
        c.geometry.target.x = -1800;
        assert!(arbitrate_geometry(ID, [c], now, 1).is_some());
        c.control = Some(PhysicalRect {
            x: 100,
            y: 100,
            width: 200,
            height: 100,
        });
        c.geometry.target.x = 92;
        assert!(arbitrate_geometry(ID, [c], now, 1).is_some());
    }

    #[test]
    fn mode_refresh_cannot_rejuvenate_geometry_and_status_carries_two_clocks() {
        let now = Instant::now();
        let geometry_stamp = GeometryStamp {
            source: GeometrySource::TsfCaret,
            confidence: GeometryConfidence::Exact,
            observed_at: now - Duration::from_millis(151),
            sequence: 1,
            context_epoch: 1,
        };
        let status = InputStatus {
            generation: 1,
            window: 1,
            focused_window: 2,
            process: 10,
            process_started: 100,
            mode: InputMode::English,
            composition: CompositionState::Idle,
            anchor: InputAnchor::Caret,
            geometry: candidate(now).geometry,
            sampled_at: now,
            geometry_stamp,
        };
        assert!(mode_is_fresh(status.sampled_at, now));
        assert!(!geometry_is_fresh(status.geometry_stamp.observed_at, now));
    }
}
