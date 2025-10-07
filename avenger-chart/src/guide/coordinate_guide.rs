//! Core trait for coordinate system guides
use crate::theme::Theme;

use crate::axis::Axis;
use crate::error::AvengerChartError;
use crate::guide::OverflowSpaceRequirement;
use crate::layout::LayoutBounds;
use avenger_scenegraph::marks::mark::SceneMark;
use std::collections::HashMap;

/// Trait for visual guides in coordinate systems
///
/// A CoordinateGuide represents the visual reference elements for a coordinate system.
/// This includes both axes (configured at the channel level) and coordinate-specific
/// options (configured at the plot level).
pub trait CoordinateGuide: Clone + Default {
    type Axis: Axis + Clone;

    /// Set axes that were configured at the channel level
    ///
    /// This is called during guide creation to apply all axis configurations
    /// from both plot-level and mark-level specifications.
    fn set_axes(&mut self, axes: HashMap<String, Self::Axis>);

    /// Set compiled marks for extracting default axis titles
    ///
    /// This is called during guide creation to provide access to compiled mark
    /// so that default axis titles can be extracted at render time.
    fn set_compiled_marks(
        &mut self,
        compiled_marks: Vec<std::sync::Arc<dyn crate::marks::CompiledMark>>,
        session_context: &datafusion::prelude::SessionContext,
    );

    fn update(&mut self, other: Self);

    fn build(self) -> Box<dyn CompiledGuide>;
}

#[async_trait::async_trait]
#[typetag::serde(tag = "type")]
pub trait CompiledGuide: Send + Sync + 'static {
    /// Measure how much space this guide needs outside the plot area
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &datafusion::prelude::SessionContext,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError>;

    /// Render this guide to scene marks
    async fn render(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        theme: &Theme,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &datafusion::prelude::SessionContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;

    /// Get the clipping region for the coordinate system
    ///
    /// Returns the appropriate clip region for marks in this coordinate system.
    /// This is used to ensure marks don't overflow the plot area.
    ///
    /// # Arguments
    /// * `plot_width` - Width of the plot area
    /// * `plot_height` - Height of the plot area
    /// * `scales` - Configured scales for the plot
    ///
    /// # Returns
    /// The clip region for the coordinate system
    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> avenger_scenegraph::marks::group::Clip;
}
