//! Space identity, latest-intent motion and insertion barriers. No UI or GPU types.
use crate::echo_tokens as t;
use echo_engine::{MotionSpeed, SpaceId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Animating,
    AwaitingContent,
    Suspended,
}
#[derive(Debug, Clone, Copy)]
pub struct Spring {
    pub position: f64,
    pub velocity: f64,
    pub target: f64,
}
impl Spring {
    pub fn new(position: f64) -> Self {
        Self {
            position,
            velocity: 0.0,
            target: position,
        }
    }
    pub fn advance(&mut self, seconds: f64, omega: f64) {
        if !seconds.is_finite() || seconds <= 0.0 {
            return;
        }
        let e = self.position - self.target;
        let c = self.velocity + omega * e;
        let decay = (-omega * seconds).exp();
        self.position = self.target + (e + c * seconds) * decay;
        self.velocity = (self.velocity - omega * c * seconds) * decay;
    }
    pub fn snap(&mut self) {
        self.position = self.target;
        self.velocity = 0.0;
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub space: SpaceId,
    pub offset: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub scale: f32,
    pub opacity: f32,
    pub shade: f32,
}
#[derive(Debug)]
pub struct Deck {
    order: Vec<SpaceId>,
    pub requested: SpaceId,
    pub presented: SpaceId,
    pub interaction: Option<SpaceId>,
    pub phase: Phase,
    pub spring: Spring,
    last_ms: u64,
    latest_ms: u64,
}
impl Default for Deck {
    fn default() -> Self {
        Self {
            order: vec![SpaceId::HISTORY, SpaceId::FAVORITES],
            requested: SpaceId::HISTORY,
            presented: SpaceId::HISTORY,
            interaction: None,
            phase: Phase::Suspended,
            spring: Spring::new(0.0),
            last_ms: 0,
            latest_ms: 0,
        }
    }
}
impl Deck {
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
        self.spring = Spring::new(self.index(self.requested).unwrap_or(0) as f64);
        self.interaction = None;
        if self.phase != Phase::Suspended {
            self.phase = Phase::AwaitingContent;
        }
    }
    pub fn show(&mut self, id: SpaceId, now_ms: u64) {
        let id = if self.index(id).is_some() {
            id
        } else {
            self.order[0]
        };
        self.requested = id;
        self.presented = id;
        self.interaction = None;
        self.spring = Spring::new(self.index(id).unwrap_or(0) as f64);
        self.last_ms = now_ms;
        self.latest_ms = now_ms;
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
    pub fn step(&mut self, delta: i32, looping: bool, now_ms: u64, motion: bool) -> bool {
        if delta == 0 || self.order.len() < 2 || self.phase == Phase::Suspended {
            return false;
        }
        let index = self.index(self.requested).unwrap_or(0) as i64;
        let next = if looping {
            (index + i64::from(delta)).rem_euclid(self.order.len() as i64)
        } else {
            (index + i64::from(delta)).clamp(0, self.order.len() as i64 - 1)
        } as usize;
        self.request(self.order[next], now_ms, motion)
    }
    pub fn request(&mut self, id: SpaceId, now_ms: u64, motion: bool) -> bool {
        let Some(index) = self.index(id) else {
            return false;
        };
        if id == self.requested || self.phase == Phase::Suspended {
            return false;
        }
        self.requested = id;
        self.interaction = None;
        let count = self.order.len() as f64;
        // For two panels, the one real neighboring panel determines travel direction.
        // Larger rings select the nearest equivalent position, not a queue of old animations.
        self.spring.target = if self.order.len() <= 2 {
            index as f64
        } else {
            index as f64 + ((self.spring.position - index as f64) / count).round() * count
        };
        self.latest_ms = now_ms;
        self.last_ms = now_ms;
        self.phase = Phase::Animating;
        if !motion {
            self.snap();
        }
        true
    }
    pub fn tick(&mut self, now_ms: u64, speed: MotionSpeed, panel_width: f32) -> bool {
        if self.phase != Phase::Animating {
            return false;
        }
        let elapsed = now_ms.saturating_sub(self.last_ms);
        self.last_ms = now_ms;
        let factor = match speed {
            MotionSpeed::Snappy => 1.25,
            MotionSpeed::Standard => 1.0,
            MotionSpeed::Relaxed => 0.75,
        };
        self.spring.advance(
            elapsed as f64 / 1000.0,
            f64::from(t::MOTION_SPRING_OMEGA) * factor,
        );
        let deadline = match speed {
            MotionSpeed::Snappy => 220,
            MotionSpeed::Standard => t::MOTION_MAX_SETTLE as u64,
            MotionSpeed::Relaxed => 340,
        };
        let distance = (self.spring.position - self.spring.target).abs()
            * f64::from(panel_width * t::FLOW_SIDE_X_RATIO);
        let velocity = self.spring.velocity.abs() * f64::from(panel_width * t::FLOW_SIDE_X_RATIO);
        if now_ms.saturating_sub(self.latest_ms) >= deadline
            || (distance < f64::from(t::MOTION_SETTLE_POSITION_PX)
                && velocity < f64::from(t::MOTION_SETTLE_VELOCITY_PX_S))
        {
            self.snap();
        }
        true
    }
    pub fn snap(&mut self) {
        self.spring.snap();
        self.spring = Spring::new(self.index(self.requested).unwrap_or(0) as f64);
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
    pub fn poses(&self, panel_width: f32) -> Vec<Pose> {
        let n = self.order.len();
        if n == 0 || self.phase == Phase::Suspended {
            return vec![];
        }
        let p = self.spring.position;
        let mut slots: Vec<(SpaceId, f64)> = Vec::new();
        if n <= 2 {
            for (index, id) in self.order.iter().enumerate() {
                slots.push((*id, index as f64 - p));
            }
        } else {
            let start = p.floor() as i64 - 1;
            let end = p.ceil() as i64 + 1;
            for slot in start..=end {
                let id = self.order[slot.rem_euclid(n as i64) as usize];
                let offset = slot as f64 - p;
                if let Some(old) = slots.iter_mut().find(|(old, _)| *old == id) {
                    if offset.abs() < old.1.abs() {
                        old.1 = offset;
                    }
                } else {
                    slots.push((id, offset));
                }
            }
        }
        slots
            .into_iter()
            .filter(|(_, d)| d.abs() < 2.0)
            .take(t::BUDGET_TRANSITION_PANELS)
            .map(|(space, d)| panel_pose(space, d as f32, panel_width))
            .collect()
    }

    /// A caret popup has one neighboring card on its available side. Keep the
    /// nearest two real spaces during travel; folding the travel around the front
    /// leaves the input-adjacent card fixed without mirroring its text or identity.
    pub fn popup_poses(&self, panel_width: f32, right: bool) -> Vec<Pose> {
        let mut poses = self.poses(panel_width);
        poses.sort_by(|a, b| {
            a.offset
                .abs()
                .total_cmp(&b.offset.abs())
                .then_with(|| b.offset.total_cmp(&a.offset))
        });
        poses.truncate(2);
        poses
            .into_iter()
            .map(|p| {
                panel_pose(
                    p.space,
                    p.offset.abs().min(1.0) * if right { 1.0 } else { -1.0 },
                    panel_width,
                )
            })
            .collect()
    }
}

fn panel_pose(space: SpaceId, d: f32, panel_width: f32) -> Pose {
    let amount = d.abs().min(1.0);
    let far = (d.abs() - 1.0).max(0.0);
    Pose {
        space,
        offset: d,
        x: d * panel_width * t::FLOW_SIDE_X_RATIO,
        y: t::FLOW_SIDE_Y * amount,
        z: panel_width * (t::FLOW_SIDE_Z_RATIO * amount - 0.15 * far),
        yaw: -d.signum() * t::FLOW_SIDE_ANGLE.to_radians() * amount,
        scale: 1.0 - (1.0 - t::FLOW_SIDE_SCALE) * amount - 0.1 * far,
        opacity: (1.0 - (1.0 - t::FLOW_SIDE_OPACITY) * amount) * (1.0 - far),
        shade: t::FLOW_SIDE_SHADE * amount,
    }
}

/// Logical distances from the front-card center to the outside of the popup.
/// Include the shadow/reflection envelope, not just the visible card rectangle.
pub fn popup_horizontal_extents(width: f32, height: f32) -> [f32; 2] {
    let padding = t::FLOW_SHADOW_MARGIN
        .max(height * t::FLOW_REFLECTION_HEIGHT_RATIO + t::FLOW_REFLECTION_GAP);
    let front = width / 2.0 + padding;
    let side = project_panel(
        panel_pose(SpaceId::FAVORITES, 1.0, width),
        width,
        height,
        padding,
    )
    .expect("positive popup dimensions")
    .into_iter()
    .map(|p| p[0])
    .fold(front, f32::max);
    [front.ceil() + 2.0, side.ceil() + 2.0]
}
#[cfg(test)]
mod tests {
    use super::*;
    fn deck(n: i64) -> Deck {
        let mut d = Deck::default();
        d.set_order((1..=n).map(SpaceId));
        d.show(SpaceId(1), 0);
        d.ready(SpaceId(1));
        d
    }
    #[test]
    fn two_spaces_never_repeat_a_panel() {
        let mut d = deck(2);
        assert!(d.step(1, true, 0, true));
        for time in 0..=260 {
            d.tick(time, MotionSpeed::Standard, 740.0);
            let poses = d.poses(740.0);
            assert_eq!(poses.len(), 2);
            assert_ne!(poses[0].space, poses[1].space);
        }
        assert_eq!(d.requested, SpaceId(2));
        d.ready(SpaceId(2));
        assert!(d.step(1, true, 300, true));
        assert_eq!(d.spring.target, 0.0);
    }
    #[test]
    fn rapid_input_keeps_latest_intent_and_velocity() {
        let mut d = deck(8);
        d.step(1, true, 0, true);
        d.tick(50, MotionSpeed::Standard, 740.0);
        let velocity = d.spring.velocity;
        d.step(1, true, 50, true);
        assert_eq!(d.spring.velocity, velocity);
        for i in 1..=101 {
            d.step(1, true, 50 + i, true);
            assert!(d.poses(740.0).len() <= 4);
        }
        let wanted = d.requested;
        d.tick(420, MotionSpeed::Standard, 740.0);
        assert_eq!(d.presented, wanted);
        assert_eq!(d.phase, Phase::AwaitingContent);
    }
    #[test]
    fn tab_enter_cannot_insert_previous_or_late_content() {
        let mut d = deck(3);
        assert!(d.can_insert(SpaceId(1)));
        d.step(1, true, 0, true);
        assert!(!d.can_insert(SpaceId(1)));
        d.snap();
        d.ready(SpaceId(1));
        assert!(!d.can_insert(SpaceId(1)));
        assert!(!d.can_insert(SpaceId(2)));
        d.ready(SpaceId(2));
        assert!(d.can_insert(SpaceId(2)));
    }
    #[test]
    fn hidden_state_never_ticks_or_accepts_readiness() {
        let mut d = deck(3);
        d.step(1, true, 0, true);
        d.hide();
        assert!(!d.tick(99, MotionSpeed::Standard, 740.0));
        d.ready(SpaceId(2));
        assert_eq!(d.phase, Phase::Suspended);
        assert!(d.poses(740.0).is_empty());
        assert!(d.interaction.is_none());
    }
    #[test]
    fn no_loop_stops_and_deleted_space_falls_back_safely() {
        let mut d = deck(3);
        assert!(!d.step(-1, false, 0, true));
        d.request(SpaceId(3), 0, false);
        assert!(!d.step(1, false, 0, true));
        d.set_order([SpaceId(1), SpaceId(2)]);
        assert_eq!(d.requested, SpaceId(1));
        assert!(!d.can_insert(SpaceId(1)));
    }
    #[test]
    fn analytic_spring_is_frame_rate_independent() {
        let mut a = Spring::new(0.0);
        a.target = 1.0;
        let mut b = a;
        a.advance(0.2, 40.0);
        for _ in 0..24 {
            b.advance(0.2 / 24.0, 40.0);
        }
        assert!((a.position - b.position).abs() < 1e-12);
        assert!((a.velocity - b.velocity).abs() < 1e-12);
    }
    #[test]
    fn center_is_pixel_aligned_and_side_signs_are_correct() {
        let d = deck(3);
        let poses = d.poses(740.0);
        let c = poses.iter().find(|p| p.space == SpaceId(1)).unwrap();
        assert_eq!(c.scale, 1.0);
        assert_eq!(c.x, 0.0);
        assert_eq!(c.yaw, 0.0);
        assert!(poses.iter().any(|p| p.offset < 0.0 && p.yaw > 0.0));
        assert!(poses.iter().any(|p| p.offset > 0.0 && p.yaw < 0.0));
    }
}

/// Screen-space hit testing uses the same projective transform as the GPU shader.
pub fn hit_panel(pose: Pose, point: [f32; 2], width: f32, height: f32) -> bool {
    if width <= 0.0 || height <= 0.0 || !point.iter().all(|v| v.is_finite()) {
        return false;
    }
    let Some(vertices) = project_panel(pose, width, height, 0.0) else {
        return false;
    };
    let mut positive = false;
    let mut negative = false;
    for i in 0..4 {
        let a = vertices[i];
        let b = vertices[(i + 1) % 4];
        let cross = (b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0]);
        positive |= cross > 0.0;
        negative |= cross < 0.0;
    }
    !(positive && negative)
}

/// Project a panel or its shadow margin using the same camera as Cover Flow.
pub fn project_panel(pose: Pose, width: f32, height: f32, padding: f32) -> Option<[[f32; 2]; 4]> {
    if [width, height].iter().any(|v| !v.is_finite() || *v <= 0.0)
        || !padding.is_finite()
        || padding < 0.0
    {
        return None;
    }
    let mut vertices = [[0.0; 2]; 4];
    let w = width / 2.0 + padding;
    let h = height / 2.0 + padding;
    for (slot, [x, y]) in vertices
        .iter_mut()
        .zip([[-w, -h], [w, -h], [w, h], [-w, h]])
    {
        let x = x * pose.scale;
        let y = y * pose.scale;
        let depth = 1.0 - (-x * pose.yaw.sin() + pose.z) / (width * t::FLOW_PERSPECTIVE_RATIO);
        if !depth.is_finite() || depth <= 0.01 {
            return None;
        }
        *slot = [(x * pose.yaw.cos() + pose.x) / depth, (y + pose.y) / depth];
        if slot.iter().any(|v| !v.is_finite()) {
            return None;
        }
    }
    Some(vertices)
}

#[cfg(test)]
mod cover_readability_tests {
    use super::*;
    #[test]
    fn popup_motion_and_shadows_fit_the_reserved_side() {
        for width in [520.0, 740.0] {
            let extents = popup_horizontal_extents(width, 560.0);
            for height in [180.0, 300.0, 520.0, 560.0] {
                let padding = t::FLOW_SHADOW_MARGIN
                    .max(height * t::FLOW_REFLECTION_HEIGHT_RATIO + t::FLOW_REFLECTION_GAP);
                for step in 0..=1000 {
                    let pose = panel_pose(SpaceId::HISTORY, step as f32 / 1000.0, width);
                    for [x, _] in project_panel(pose, width, height, padding).unwrap() {
                        assert!(
                            x >= -extents[0] && x <= extents[1],
                            "{pose:?}: {x} outside {extents:?}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn popup_switches_real_spaces_on_the_selected_side() {
        for count in [2, 4] {
            for right in [false, true] {
                let mut deck = Deck::default();
                deck.set_order((1..=count).map(SpaceId));
                deck.show(SpaceId::HISTORY, 0);
                deck.ready(SpaceId::HISTORY);
                assert!(deck.step(1, true, 0, true));
                for now in 0..=300 {
                    deck.tick(now, MotionSpeed::Standard, 520.0);
                    let poses = deck.popup_poses(520.0, right);
                    assert_eq!(poses.len(), 2);
                    assert_ne!(poses[0].space, poses[1].space);
                    assert!(poses
                        .iter()
                        .all(|p| if right { p.x >= 0.0 } else { p.x <= 0.0 }));
                }
                let poses = deck.popup_poses(520.0, right);
                assert_eq!(poses[0].space, SpaceId::FAVORITES);
                assert_eq!(poses[0].x, 0.0);
                let side = poses[1];
                let quad = project_panel(side, 520.0, 300.0, 0.0).unwrap();
                let center = [
                    quad.iter().map(|p| p[0]).sum::<f32>() / 4.0,
                    quad.iter().map(|p| p[1]).sum::<f32>() / 4.0,
                ];
                assert!(hit_panel(side, center, 520.0, 300.0));
                assert!(!hit_panel(side, [-center[0], center[1]], 520.0, 300.0));
            }
        }
    }
    #[test]
    fn outer_edges_face_viewer_and_right_title_is_not_behind_center() {
        let width = 640.0;
        let height = 752.0;
        let mut deck = Deck::default();
        deck.set_order([SpaceId(1), SpaceId(2), SpaceId(3)]);
        deck.show(SpaceId(1), 0);
        for pose in deck.poses(width).into_iter().filter(|p| p.offset != 0.0) {
            let quad = project_panel(pose, width, height, 0.0).unwrap();
            let left_height = quad[3][1] - quad[0][1];
            let right_height = quad[2][1] - quad[1][1];
            if pose.offset > 0.0 {
                assert!(right_height > left_height, "right cover turned away");
                let local_x = (-width / 2.0 + t::PANEL_PADDING + 14.0) * pose.scale;
                let depth = 1.0
                    - (-local_x * pose.yaw.sin() + pose.z) / (width * t::FLOW_PERSPECTIVE_RATIO);
                let title_x = (local_x * pose.yaw.cos() + pose.x) / depth;
                assert!(title_x > width / 2.0, "right title is obscured by center");
            } else {
                assert!(left_height > right_height, "left cover turned away");
            }
        }
    }
}
