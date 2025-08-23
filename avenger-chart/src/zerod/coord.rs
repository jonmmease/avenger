//! Zero-dimensional coordinate system
//!
//! The ZeroDCoord type represents a zero-dimensional coordinate system - essentially
//! a single point with no spatial extent. This is useful in contexts where marks need
//! to be rendered without any coordinate mapping, such as:
//!
//! - Legend symbols that show mark appearance without position
//! - Default value extraction from marks
//! - Standalone mark previews
//!
//! # Conceptual Model
//!
//! In a 0D coordinate system, there are no position channels, no axes, and no spatial
//! transformations. Marks rendered in this system appear at a fixed location without
//! any data-driven positioning.
//!
//! # Important
//!
//! ZeroDCoord should NOT be used for actual data visualization. All spatial methods
//! will panic with `unreachable!()` as they are meaningless in zero dimensions.

use crate::axis::AxisTrait;
use crate::coords::{CoordinateSystem, OverflowSpaceRequirement, TransformResult};
use crate::error::AvengerChartError;
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::logical_expr::Expr;
use std::any::Any;
use std::collections::HashMap;

/// A zero-dimensional coordinate system
///
/// Represents a 0D space (a single point) where marks have no spatial extent
/// or position channels. Useful for legends and other non-spatial mark rendering.
pub struct ZeroDCoord;

/// A placeholder axis for the zero-dimensional coordinate system
#[derive(Debug, Clone)]
pub struct ZeroDAxis {
    // No axes exist in 0D space
}

impl AxisTrait for ZeroDAxis {
    fn clone_box(&self) -> Box<dyn AxisTrait> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

#[async_trait::async_trait]
impl CoordinateSystem for ZeroDCoord {
    type Axis = ZeroDAxis;

    fn required_channels(&self) -> &'static [&'static str] {
        // ZeroDCoord has no position channels (0D space)
        &[]
    }

    fn default_range(&self, _channel: &str, _width: f64, _height: f64) -> Option<(f64, f64)> {
        // No ranges in 0D space
        None
    }

    fn transform_expressions(
        &self,
        _channels: HashMap<String, Expr>,
    ) -> Result<TransformResult, AvengerChartError> {
        unreachable!("ZeroDCoord has no spatial dimensions to transform")
    }

    fn create_default_axes(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _marks: &[Box<dyn crate::marks::Mark<Self>>],
    ) -> HashMap<String, Self::Axis>
    where
        Self: Sized,
    {
        // No axes exist in zero-dimensional space
        HashMap::new()
    }

    async fn measure_guide_overflow(
        &self,
        _axes: HashMap<String, Self::Axis>,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _width: f32,
        _height: f32,
        _plot_area_ratio: f32,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        unreachable!("ZeroDCoord has no spatial extent for guides")
    }

    async fn render_axes(
        &self,
        _axes: &HashMap<String, Self::Axis>,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _padding: &crate::render::Padding,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        unreachable!("ZeroDCoord has no axes to render (0D space)")
    }

    fn get_clip(
        &self,
        _plot_width: f32,
        _plot_height: f32,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Clip {
        // No clipping needed in 0D space
        Clip::None
    }

    fn prepare_scalar_batch(
        &self,
        batch: datafusion::arrow::record_batch::RecordBatch,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<datafusion::arrow::record_batch::RecordBatch, AvengerChartError> {
        // Pass through unchanged - no coordinate preparation needed
        Ok(batch)
    }
}
