//! Core trait for coordinate system guides

use crate::axis::Axis;
use crate::error::AvengerChartError;
use crate::guide::{GuideUpdate, OverflowSpaceRequirement};
use crate::layout::LayoutBounds;
use crate::theme::Theme;
use avenger_scenegraph::marks::mark::SceneMark;
use std::collections::HashMap;

/// Trait for visual guides in coordinate systems
///
/// A CoordinateGuide represents the visual reference elements for a coordinate system.
/// This includes both axes (configured at the channel level) and coordinate-specific
/// options (configured at the plot level).
#[async_trait::async_trait]
pub trait CoordinateGuide: Clone + Send + Sync + GuideUpdate + 'static {
    /// The axis type used by this guide (if any)
    type Axis: Axis;

    /// Set axes that were configured at the channel level
    ///
    /// This is called during guide creation to apply all axis configurations
    /// from both plot-level and mark-level specifications.
    fn set_axes(&mut self, axes: HashMap<String, Self::Axis>);

    /// Get the configured axes
    ///
    /// Returns a reference to the map of channel names to axis configurations.
    fn axes(&self) -> &HashMap<String, Self::Axis>;

    /// Measure how much space this guide needs outside the plot area
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &dyn Theme,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError>;

    /// Render this guide to scene marks
    async fn render(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        theme: &dyn Theme,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;
}
