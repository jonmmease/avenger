//! Core trait for coordinate system guides
use crate::facet::coordination::FacetCoordinationContext;
use crate::theme::Theme;

use crate::axis::Axis;
use crate::error::AvengerChartError;
use crate::guide::{MeasurementResult, OverflowSpaceRequirement};
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
    ///
    /// # Arguments
    /// * `data_override` - Optional DataFrame to use instead of compiled data.
    ///   This enables nested facets to pass filtered data to inner guides at runtime.
    ///   When Some, guides should use this data. When None, use compiled data.
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        row_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        col_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        coordination_context: Option<&FacetCoordinationContext>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &datafusion::prelude::SessionContext,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError>;

    /// Measure only the intrinsic subplot overflow, excluding facet-level decorative content
    ///
    /// For facet guides (FacetRow, FacetCol), this returns only the overflow needed by
    /// the Cartesian subplots (axes, tick labels), WITHOUT adding space for facet labels,
    /// titles, or unified axis titles. This is used when measuring nested facets to avoid
    /// double-counting facet-level spacing.
    ///
    /// For non-facet guides (Cartesian, Polar), this is equivalent to `measure_overflow()`.
    ///
    /// # Arguments
    /// Same as `measure_overflow()`
    async fn measure_intrinsic_overflow(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        row_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        col_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        coordination_context: Option<&FacetCoordinationContext>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &datafusion::prelude::SessionContext,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        // Default implementation: same as measure_overflow()
        // Facet guides will override this to exclude facet-level content
        self.measure_overflow(
            scales,
            row_overflow,
            col_overflow,
            plot_width,
            plot_height,
            theme,
            params,
            coordination_context,
            data_override,
            ctx,
        )
        .await
    }

    /// Measure overflow with internal self-coordination
    ///
    /// This method enables facet guides to perform their own internal two-pass measurement,
    /// computing both overflow AND spacing needs (e.g., inter-row/col gaps).
    ///
    /// # Default Implementation
    /// Delegates to `measure_overflow()` and wraps the result in a `MeasurementResult`
    /// with empty `spacing_needs`. Non-facet guides (Cartesian, Polar) use this default.
    ///
    /// # Override Pattern
    /// Facet guides (FacetRowGuide, FacetColGuide) override this to:
    /// 1. Check for recursion guard (coordinated_spacing already set)
    /// 2. Perform Pass 1 measurement with zero gap
    /// 3. Compute required gap from Pass 1 overflow
    /// 4. Perform Pass 2 measurement with computed gap
    /// 5. Return MeasurementResult with overflow + spacing_needs
    ///
    /// # Arguments
    /// Same as `measure_overflow()`
    async fn measure_with_coordination(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        row_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        col_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        coordination_context: Option<&FacetCoordinationContext>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &datafusion::prelude::SessionContext,
    ) -> Result<MeasurementResult, AvengerChartError> {
        // Default: measure once, return with empty spacing_needs
        let overflow = self
            .measure_overflow(
                scales,
                row_overflow,
                col_overflow,
                plot_width,
                plot_height,
                theme,
                params,
                coordination_context,
                data_override,
                ctx,
            )
            .await?;
        Ok(MeasurementResult::new(overflow))
    }

    /// Evaluate this guide to scene marks
    ///
    /// # Arguments
    /// * `data_override` - Optional DataFrame to use instead of compiled data.
    ///   This enables nested facets to pass filtered data to inner guides at runtime.
    ///   When Some, guides should use this data. When None, use compiled data.
    async fn evaluate(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        row_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        col_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        theme: &Theme,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        coordination_context: Option<&FacetCoordinationContext>,
        ctx: &datafusion::prelude::SessionContext,
        data_override: Option<&datafusion::dataframe::DataFrame>,
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

    /// Check if this guide suppresses the specified channel's axis title
    ///
    /// This is used by outer facet guides to determine whether they should render
    /// a unified axis title. If a subplot guide returns true, it means the subplot
    /// will NOT render that axis title (expecting the outer facet to handle it).
    ///
    /// # Arguments
    /// * `channel` - The channel name (e.g., "x", "y")
    ///
    /// # Returns
    /// true if this guide suppresses the channel's axis title, false otherwise
    ///
    /// # Examples
    /// - CartesianGuide always returns false (renders its own axis titles)
    /// - FacetRowGuide returns true for "y" (suppresses y-axis titles in subplots)
    /// - FacetColGuide returns true for "x" (suppresses x-axis titles in subplots)
    fn unifies_channel(&self, _channel: &str) -> bool {
        // Default: guides render their own axis titles
        false
    }

    /// Get this guide as Any for downcasting to concrete types
    ///
    /// This enables runtime type checking and downcasting of CompiledGuide
    /// trait objects to their concrete types (e.g., CartesianGuide).
    fn as_any(&self) -> &dyn std::any::Any;
}
