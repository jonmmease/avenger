//! Helper methods for rendering with the Guide API

use super::PlotRenderer;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::guide::OverflowSpaceRequirement;
use avenger_scenegraph::marks::mark::SceneMark;
use std::collections::HashMap;

impl<C: CoordinateSystem> PlotRenderer<'_, C> {
    /// Create a guide with all axis configurations applied.
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
            .create_default_axes(scales, &self.plot.marks);

        // Apply user axis customizations to defaults
        let mut all_axes = default_axes;
        for (channel, axis_spec) in &self.plot.axis_specs {
            // Only apply customizations if there's a default axis
            // Axes without scales won't be rendered anyway
            if let Some(base_axis) = all_axes.get(channel).cloned() {
                // Apply the customization function
                match axis_spec {
                    crate::plot::AxisSpec::Local(f) => {
                        let customized = f(base_axis);
                        all_axes.insert(channel.clone(), customized);
                    }
                    crate::plot::AxisSpec::Reference(_) => {
                        // Reference axes not yet supported, keep default
                    }
                }
            }
        }

        // Apply axis configurations from mark channels (last mark wins for conflicts)
        for mark in &self.plot.marks {
            for (channel, axis_config) in mark.state().axis_configs.iter() {
                if let Some(base_axis) = all_axes.get(channel).cloned() {
                    let configured = axis_config(base_axis);
                    all_axes.insert(channel.clone(), configured);
                }
            }
        }

        // Create the guide with the configured axes
        let mut guide =
            self.plot
                .coord_system()
                .create_default_guide(all_axes, scales, &self.plot.marks);

        // Apply user guide configuration if specified
        if let Some(guide_config) = &self.plot.guide_spec {
            guide = guide_config(guide);
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
            .measure_guide_overflow(guide, scales, width_estimate, height_estimate, &theme)
            .await
    }
}
