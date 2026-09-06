//! Pixel budgets affect moving textures only; the settled native card stays at full DPI.
use echo_presentation::echo_tokens as t;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RasterPolicy {
    #[default]
    Standard,
    Economical,
}
impl RasterPolicy {
    pub fn texture_limit(self) -> u64 {
        (match self {
            Self::Standard => 48,
            Self::Economical => 32,
        }) * 1024
            * 1024
    }
    pub fn maximum_scale(self) -> f32 {
        match self {
            Self::Standard => t::QUALITY_CARD_RASTER_STANDARD,
            Self::Economical => t::QUALITY_CARD_RASTER_ECONOMICAL,
        }
    }
    pub fn frame_cap(self) -> u32 {
        match self {
            Self::Standard => 144,
            Self::Economical => 60,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Economical => "economical",
        }
    }
    /// Cache high-density card text without supersampling the whole stage.
    /// Reserve the output and four card targets before choosing a resolution.
    pub fn panel_scale(
        self,
        stage: [f32; 2],
        panel: [f32; 2],
        native: f32,
        max_dimension: u32,
    ) -> Result<f32, String> {
        let output = f64::from(self.scale(stage, panel, native, max_dimension)?);
        let stage_bytes = 4.0 * f64::from(stage[0]) * f64::from(stage[1]) * output * output;
        let remaining = (self.texture_limit() - 1024 * 1024) as f64 - stage_bytes;
        let cards = 16.0 * f64::from(panel[0]) * f64::from(panel[1]);
        let memory_scale = (remaining.max(0.0) / cards).sqrt() as f32;
        let dimensions = max_dimension as f32 / panel[0].max(panel[1]);
        let scale = self.maximum_scale().min(memory_scale).min(dimensions);
        let scale = (scale * 16.0).floor() / 16.0;
        if scale < 0.25 {
            return Err("Card quality exceeds the bounded texture budget".into());
        }
        Ok(scale)
    }
    pub fn scale(
        self,
        stage: [f32; 2],
        panel: [f32; 2],
        native: f32,
        max_dimension: u32,
    ) -> Result<f32, String> {
        if [stage[0], stage[1], panel[0], panel[1], native]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || max_dimension == 0
        {
            return Err("Invalid motion texture geometry".into());
        }
        // Reserve four real panels even when only two or three are currently visible.
        // The spare MiB accounts for integer rounding; driver/glyph allocations are separate.
        let coefficient = 4.0
            * (f64::from(stage[0]) * f64::from(stage[1])
                + 4.0 * f64::from(panel[0]) * f64::from(panel[1]));
        let memory_scale =
            ((self.texture_limit() - 1024 * 1024) as f64 / coefficient).sqrt() as f32;
        let largest = stage.into_iter().chain(panel).fold(0.0_f32, f32::max);
        let scale = native
            .min(self.maximum_scale())
            .min(memory_scale)
            .min(max_dimension as f32 / largest);
        let scale = (scale * 16.0).floor() / 16.0;
        if scale < 0.25 {
            return Err("Display exceeds the bounded motion texture budget".into());
        }
        Ok(scale)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn bytes(stage: [f32; 2], panel: [f32; 2], scale: f32) -> u64 {
        let area = |v: [f32; 2]| (v[0] * scale).ceil() as u64 * (v[1] * scale).ceil() as u64;
        4 * (area(stage) + 4 * area(panel))
    }
    #[test]
    fn ordinary_one_x_textures_are_not_downsampled() {
        for policy in [RasterPolicy::Standard, RasterPolicy::Economical] {
            assert_eq!(
                policy
                    .scale([1120.0, 800.0], [740.0, 752.0], 1.0, 8192)
                    .unwrap(),
                1.0
            );
        }
    }
    #[test]
    fn high_dpi_and_large_windows_stay_bounded() {
        for policy in [RasterPolicy::Standard, RasterPolicy::Economical] {
            for (stage, panel) in [
                ([1120.0, 800.0], [740.0, 752.0]),
                ([3840.0, 2160.0], [740.0, 2112.0]),
            ] {
                for dpi in [1.0, 1.25, 1.5, 2.0, 3.0, 4.0] {
                    let scale = policy.scale(stage, panel, dpi, 8192).unwrap();
                    assert!(scale <= dpi && scale <= policy.maximum_scale());
                    assert!(bytes(stage, panel, scale) < policy.texture_limit());
                }
            }
        }
    }
    #[test]
    fn device_dimension_limit_is_respected() {
        let scale = RasterPolicy::Standard
            .scale([3840.0, 2160.0], [740.0, 2112.0], 3.0, 2048)
            .unwrap();
        assert!(3840.0 * scale <= 2048.0 && 2160.0 * scale <= 2048.0);
    }
    #[test]
    fn invalid_or_unrepresentable_geometry_fails_explicitly() {
        for native in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(RasterPolicy::Standard
                .scale([1120.0, 800.0], [740.0, 752.0], native, 8192)
                .is_err());
        }
        assert!(RasterPolicy::Economical
            .scale([100_000.0, 100_000.0], [740.0, 752.0], 2.0, 2048)
            .is_err());
    }
    #[test]
    fn economical_policy_preserves_a_sixty_hz_target() {
        assert_eq!(RasterPolicy::Economical.frame_cap(), 60);
        assert_eq!(RasterPolicy::Economical.texture_limit(), 32 * 1024 * 1024);
        assert!(RasterPolicy::Standard.frame_cap() >= 120);
    }
}
impl RasterPolicy {
    pub fn for_device(economical: bool) -> Self {
        // Exercise low-resource policy on CI without claiming it is an actual iGPU measurement.
        #[cfg(feature = "native-test")]
        if std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref() == Ok("1")
            && std::env::var("ECHO_ACCEPTANCE_GRAPHICS_BUDGET").as_deref() == Ok("integrated")
        {
            return Self::Economical;
        }
        if economical {
            Self::Economical
        } else {
            Self::Standard
        }
    }
}

#[cfg(test)]
mod quality_tests {
    use super::*;
    #[test]
    fn cached_cards_gain_detail_without_larger_output() {
        let stage = [1600.0, 800.0];
        let panel = [640.0, 752.0];
        for policy in [RasterPolicy::Standard, RasterPolicy::Economical] {
            assert_eq!(policy.scale(stage, panel, 1.0, 8192).unwrap(), 1.0);
        }
        assert_eq!(
            RasterPolicy::Standard
                .panel_scale(stage, panel, 1.0, 8192)
                .unwrap(),
            2.0
        );
        assert_eq!(
            RasterPolicy::Economical
                .panel_scale(stage, panel, 1.0, 8192)
                .unwrap(),
            1.5
        );
    }
    #[test]
    fn output_and_four_high_density_cards_fit_the_same_budget() {
        let area = |v: [f32; 2], s: f32| (v[0] * s).ceil() as u64 * (v[1] * s).ceil() as u64;
        for policy in [RasterPolicy::Standard, RasterPolicy::Economical] {
            for (stage, panel) in [
                ([1600.0, 800.0], [640.0, 752.0]),
                ([3840.0, 2160.0], [740.0, 2112.0]),
            ] {
                for native in [1.0, 1.25, 1.5, 2.0, 3.0, 4.0] {
                    let output = policy.scale(stage, panel, native, 8192).unwrap();
                    let card = policy.panel_scale(stage, panel, native, 8192).unwrap();
                    assert!(
                        4 * (area(stage, output) + 4 * area(panel, card)) < policy.texture_limit()
                    );
                    assert!(card <= policy.maximum_scale() && output <= native);
                }
            }
        }
    }
}
