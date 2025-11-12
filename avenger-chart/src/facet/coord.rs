use crate::coords::{CoordinateSystem, CoordinateSystemTransform, OverflowSpaceRequirement};
use crate::error::AvengerChartError;
use crate::facet::guide::{FacetColGuide, FacetRowGuide, GridFacetGuide};
use avenger_common::value::ScalarOrArray;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Row faceting coordinate system
///
/// Facets data along the row dimension, creating a vertical stack of subplots.
/// Each subplot represents one unique value from the `row` channel.
///
/// # Example
/// ```ignore
/// let plot = Plot::<FacetRow>::new()
///     .data(df)
///     .mark(
///         Facet::new()
///             .row(col("species"))
///             .subplot(
///                 Plot::<Cartesian>::new().mark(Symbol::new()...),
///             ),
///     );
/// ```
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FacetRow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) padding_px: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
}

impl FacetRow {
    pub(crate) fn new_with_state(
        padding_px: Option<f32>,
        overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
    ) -> Self {
        Self {
            padding_px,
            overflow_by_facet,
        }
    }
}

impl CoordinateSystem for FacetRow {
    type Guide = FacetRowGuide;

    fn required_channels(&self) -> &'static [&'static str] {
        &["row"]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for FacetRow {
    fn required_channels(&self) -> &'static [&'static str] {
        &["row"]
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    fn with_measured_padding(
        &self,
        padding_px: f32,
        overflow: Vec<crate::coords::OverflowSpaceRequirement>,
    ) -> Box<dyn CoordinateSystemTransform> {
        let mut updated = self.clone();
        updated.padding_px = Some(padding_px);
        updated.overflow_by_facet = Some(overflow);
        Box::new(updated)
    }

    fn transform(
        &self,
        _position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Box<dyn crate::coords::PlotGeometry>, AvengerChartError> {
        // FacetRow's transform is not used for direct geometry; subplots manage their own.
        // Return a degenerate point geometry to satisfy the trait.
        Ok(Box::new(crate::coords::PointGeometry {
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
        }))
    }

    fn default_range(
        &self,
        channel: &str,
        _plot_area_width: f64,
        plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        match channel {
            "row" => Some((0.0, plot_area_height)),
            _ => None,
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
    ) -> HashMap<String, datafusion::scalar::ScalarValue> {
        use datafusion::scalar::ScalarValue;
        let mut options = HashMap::new();
        if channel == "row" && scale_impl.scale_type() == "band" {
            // Set outer padding to 0 to avoid extra space at top/bottom
            // Set inner padding to 0.1 for default spacing between facets
            options.insert("padding_inner".to_string(), ScalarValue::Float64(Some(0.1)));
            options.insert("padding_outer".to_string(), ScalarValue::Float64(Some(0.0)));
            options.insert("round".to_string(), ScalarValue::Boolean(Some(true)));
        }
        options
    }
}

/// Column faceting coordinate system
///
/// Facets data along the column dimension, creating a horizontal row of subplots.
/// Each subplot represents one unique value from the `column` channel.
///
/// # Example
/// ```ignore
/// let plot = Plot::<FacetColumn>::new()
///     .data(df)
///     .mark(
///         Facet::new()
///             .column(col("year"))
///             .subplot(
///                 Plot::<Cartesian>::new().mark(Symbol::new()...),
///             ),
///     );
/// ```
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FacetColumn {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) padding_px: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
}

impl FacetColumn {
    pub(crate) fn new_with_state(
        padding_px: Option<f32>,
        overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
    ) -> Self {
        Self {
            padding_px,
            overflow_by_facet,
        }
    }
}

impl CoordinateSystem for FacetColumn {
    type Guide = FacetColGuide;

    fn required_channels(&self) -> &'static [&'static str] {
        &["column"]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for FacetColumn {
    fn required_channels(&self) -> &'static [&'static str] {
        &["column"]
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    fn with_measured_padding(
        &self,
        padding_px: f32,
        overflow: Vec<crate::coords::OverflowSpaceRequirement>,
    ) -> Box<dyn CoordinateSystemTransform> {
        let mut updated = self.clone();
        updated.padding_px = Some(padding_px);
        updated.overflow_by_facet = Some(overflow);
        Box::new(updated)
    }

    fn transform(
        &self,
        _position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Box<dyn crate::coords::PlotGeometry>, AvengerChartError> {
        // Facet mark handles subplot positioning; return degenerate geometry
        Ok(Box::new(crate::coords::PointGeometry {
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
        }))
    }

    fn default_range(
        &self,
        channel: &str,
        plot_area_width: f64,
        _plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        match channel {
            "column" => Some((0.0, plot_area_width)),
            _ => None,
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
    ) -> HashMap<String, datafusion::scalar::ScalarValue> {
        use datafusion::scalar::ScalarValue;
        let mut options = HashMap::new();
        if channel == "column" && scale_impl.scale_type() == "band" {
            // Set outer padding to 0 to avoid extra space at left/right
            // Set inner padding to 0.1 for default spacing between facets
            options.insert("padding_inner".to_string(), ScalarValue::Float64(Some(0.1)));
            options.insert("padding_outer".to_string(), ScalarValue::Float64(Some(0.0)));
            options.insert("round".to_string(), ScalarValue::Boolean(Some(true)));
        }
        options
    }
}

/// Grid faceting coordinate system
///
/// Facets data along both row and column dimensions simultaneously, creating
/// a 2D grid of subplots. Each cell in the grid represents one combination
/// of row and column values.
///
/// # Example
/// ```ignore
/// let plot = Plot::<FacetGrid>::new()
///     .data(df)
///     .mark(
///         Facet::new()
///             .row(col("species"))
///             .col(col("year"))
///             .subplot(
///                 Plot::<Cartesian>::new().mark(Symbol::new()...),
///             ),
///     );
/// ```
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FacetGrid {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) row_padding_px: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) col_padding_px: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) row_overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) col_overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
}

impl FacetGrid {
    pub(crate) fn new_with_state(
        row_padding_px: Option<f32>,
        col_padding_px: Option<f32>,
        row_overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
        col_overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
    ) -> Self {
        Self {
            row_padding_px,
            col_padding_px,
            row_overflow_by_facet,
            col_overflow_by_facet,
        }
    }
}

impl CoordinateSystem for FacetGrid {
    type Guide = GridFacetGuide;

    fn required_channels(&self) -> &'static [&'static str] {
        &["row", "column"]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for FacetGrid {
    fn required_channels(&self) -> &'static [&'static str] {
        &["row", "column"]
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    fn with_measured_padding(
        &self,
        padding_px: f32,
        overflow: Vec<crate::coords::OverflowSpaceRequirement>,
    ) -> Box<dyn CoordinateSystemTransform> {
        let mut updated = self.clone();
        updated.row_padding_px = Some(padding_px);
        updated.row_overflow_by_facet = Some(overflow);
        Box::new(updated)
    }

    fn transform(
        &self,
        _position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Box<dyn crate::coords::PlotGeometry>, AvengerChartError> {
        // Facet mark handles subplot positioning; return degenerate geometry
        Ok(Box::new(crate::coords::PointGeometry {
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
        }))
    }

    fn default_range(
        &self,
        channel: &str,
        plot_area_width: f64,
        plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        match channel {
            "row" => Some((0.0, plot_area_height)),
            "column" => Some((0.0, plot_area_width)),
            _ => None,
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
    ) -> HashMap<String, datafusion::scalar::ScalarValue> {
        use datafusion::scalar::ScalarValue;
        let mut options = HashMap::new();
        if (channel == "row" || channel == "column") && scale_impl.scale_type() == "band" {
            options.insert("padding_inner".to_string(), ScalarValue::Float64(Some(0.1)));
            options.insert("padding_outer".to_string(), ScalarValue::Float64(Some(0.0)));
            options.insert("round".to_string(), ScalarValue::Boolean(Some(true)));
        }
        options
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::OverflowSpaceRequirement;

    #[test]
    fn facet_row_new_with_state_constructs_struct() {
        let coord = FacetRow::new_with_state(
            Some(5.0),
            Some(vec![OverflowSpaceRequirement {
                top: 1.0,
                bottom: 2.0,
                left: 3.0,
                right: 4.0,
            }]),
        );
        assert_eq!(coord.padding_px, Some(5.0));
        assert!(coord.overflow_by_facet.is_some());
    }

    #[test]
    fn facet_grid_new_with_state_sets_fields() {
        let coord = FacetGrid::new_with_state(
            Some(2.0),
            Some(3.0),
            Some(vec![OverflowSpaceRequirement {
                top: 0.5,
                bottom: 0.5,
                left: 0.25,
                right: 0.25,
            }]),
            None,
        );
        assert_eq!(coord.row_padding_px, Some(2.0));
        assert_eq!(coord.col_padding_px, Some(3.0));
        assert!(coord.row_overflow_by_facet.is_some());
        assert!(coord.col_overflow_by_facet.is_none());
    }
}
