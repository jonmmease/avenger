//! Core trait for coordinate system guides
use crate::theme::Theme;

use crate::axis::Axis;
use crate::error::AvengerChartError;
use crate::guide::OverflowSpaceRequirement;
use crate::layout::LayoutBounds;
use avenger_scenegraph::marks::mark::SceneMark;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Direction of faceting for determining which channel can be unified
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FacetDirection {
    /// Vertical stacking (row faceting) - can potentially unify y-axis
    Row,
    /// Horizontal arrangement (column faceting) - can potentially unify x-axis
    Column,
}

/// Information about which channel can be unified in faceting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiableChannelInfo {
    /// The channel name (e.g., "y", "x", "r")
    pub channel: String,
    /// The title to use for the unified axis (extracted from marks)
    pub title: Option<String>,
}

/// Trait for visual guides in coordinate systems
///
/// A CoordinateGuide represents the visual reference elements for a coordinate system.
/// This includes both axes (configured at the channel level) and coordinate-specific
/// options (configured at the plot level).
pub trait CoordinateGuide: Clone + Default + Send + Sync {
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

    /// Evaluate this guide to scene marks
    async fn evaluate(
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

    /// Determine which channel axis can be unified when this subplot is used in faceting.
    ///
    /// This allows each guide type to declare what makes sense to unify based on:
    /// - The faceting direction (row vs column)
    /// - The guide's coordinate system semantics
    /// - The marks in the subplot (for extracting channel titles)
    ///
    /// # Arguments
    /// * `facet_direction` - Whether faceting is by row (vertical) or column (horizontal)
    /// * `marks` - The marks in the subplot, used to extract channel titles
    /// * `session_context` - For expression evaluation when extracting titles
    ///
    /// # Returns
    /// Information about the unifiable channel, or None if no axis can be unified
    ///
    /// # Design Rationale
    /// The Guide (not the coordinate system) declares what makes sense to unify because:
    /// - 3D Cartesian has ["x", "y", "z"] but only z might make sense for row faceting
    /// - Polar has ["r", "theta"] and might want to unify r for row faceting
    /// - Geographic coordinates have special semantics
    ///
    /// # Examples
    /// - CartesianGuide: Returns "y" for Row, "x" for Column
    /// - PolarGuide: Could return "r" for Row (future)
    /// - 3DCartesianGuide: Could return "z" for Row (future)
    fn facet_unifiable_channel(
        &self,
        _facet_direction: FacetDirection,
        _marks: &[std::sync::Arc<dyn crate::marks::CompiledMark>],
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Option<UnifiableChannelInfo> {
        // Default: no unification
        None
    }

    /// Query the position of an axis by channel name
    ///
    /// Returns the position (Top/Bottom/Left/Right) for the specified channel's axis,
    /// or None if the channel has no axis or position cannot be determined.
    ///
    /// This is used by faceting to determine which rows/columns should show axis labels.
    /// For example, in row faceting with x-axis at bottom, only the bottom row shows
    /// x-axis labels. If x-axis is at top, only the top row shows labels.
    ///
    /// # Arguments
    /// * `channel` - The channel name (e.g., "x", "y", "r")
    ///
    /// # Returns
    /// The axis position, or None if not applicable or cannot be determined
    ///
    /// # Limitations (Phase 2)
    /// Currently cannot evaluate axis position expressions - returns None if axis
    /// has an explicit position expression. This will be improved in Phase 3 when
    /// we add GuideContext with evaluated state.
    fn axis_position(&self, _channel: &str) -> Option<crate::cartesian::axis::AxisPosition> {
        // Default: no position info available
        None
    }
}
