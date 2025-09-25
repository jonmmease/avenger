//! Helper methods for rendering with the Guide API

use super::PlotRenderer;
use crate::axis::Axis;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::guide::{CoordinateGuideBuilder, OverflowSpaceRequirement};
use std::collections::HashMap;

impl<C: CoordinateSystem> PlotRenderer<'_, C> {
    /// Create a guide with all axis and guide configurations applied
    /// In contrast to `create_default_guide`, this applies all user customizations
    /// and mark-level axis configurations to the default axes.
    pub(crate) fn create_configured_guide(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> C::Guide {
        // Get default axes for all channels with scales
        let default_axes = self
            .plot
            .coord_system()
            .create_default_axes(scales, &self.plot.mark_renderers);

        // Apply user axis customizations to defaults
        let mut all_axes = default_axes;
        for (channel, axis_spec) in &self.plot.axis_specs {
            // Apply the customization (update the base axis with user-specified config)
            match axis_spec {
                crate::plot::AxisSpec::Local(axis_config) => {
                    if let Some(base_axis) = all_axes.get(channel).cloned() {
                        // Update base axis with user configuration
                        let mut customized = base_axis.clone();
                        customized.update(axis_config.as_ref());
                        all_axes.insert(channel.clone(), customized);
                    } else {
                        // No base axis exists - create one from the config
                        let axis_config = axis_config.as_any().downcast_ref::<<<C as CoordinateSystem>::Guide as CoordinateGuideBuilder>::Axis>().unwrap();
                        all_axes.insert(channel.clone(), axis_config.clone());
                    }
                }
            }
        }

        // Create the guide with the configured axes
        let mut guide = self.plot.coord_system().create_default_guide(
            all_axes,
            scales,
            &self.plot.mark_renderers,
        );

        // Apply user guide configuration if specified
        if let Some(guide_config) = &self.plot.guide_config {
            guide.update(guide_config.clone());
        }

        guide
    }

    /// Measure guide overflow for layout calculation
    pub(crate) async fn measure_guide_overflow(
        &self,
        guide: &C::Guide,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        width_estimate: f32,
        height_estimate: f32,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        let theme = self.plot.get_theme();
        self.plot
            .coord_system()
            .measure_guide_overflow(
                guide,
                scales,
                width_estimate,
                height_estimate,
                theme.as_ref(),
            )
            .await
    }
}
