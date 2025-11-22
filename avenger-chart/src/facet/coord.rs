use crate::coords::{CoordinateSystem, CoordinateSystemTransform, OverflowSpaceRequirement};
use crate::error::AvengerChartError;
use crate::facet::guide::{FacetColGuide, FacetRowGuide, GridFacetGuide};
use avenger_common::value::ScalarOrArray;
use datafusion::common::ScalarValue;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
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
    pub(crate) padding_px: Option<f32>,
    pub(crate) overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
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

fn compute_band_layout(positions: &[f32], extent: f32, padding_px: Option<f32>) -> (Vec<f32>, f32) {
    if positions.is_empty() {
        return (Vec::new(), 0.0);
    }

    let mut sorted = positions.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

    let mut base_bandwidth = if sorted.len() > 1 {
        sorted
            .windows(2)
            .filter_map(|pair| {
                let gap = (pair[1] - pair[0]).abs();
                if gap.is_finite() && gap > 0.0 {
                    Some(gap)
                } else {
                    None
                }
            })
            .fold(f32::INFINITY, f32::min)
    } else {
        extent
    };

    if !base_bandwidth.is_finite() || base_bandwidth <= 0.0 {
        base_bandwidth = extent;
    }

    let effective_bandwidth = (base_bandwidth - padding_px.unwrap_or(0.0)).max(0.0);

    // Input positions are already starts (not centers), so use them directly
    let starts: Vec<f32> = positions.to_vec();

    (starts, effective_bandwidth)
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
        spec: &crate::coords::PaddingSpec,
    ) -> Box<dyn CoordinateSystemTransform> {
        match spec {
            crate::coords::PaddingSpec::Single { padding_px, overflow } => {
                let mut updated = self.clone();
                updated.padding_px = Some(*padding_px);
                updated.overflow_by_facet = Some(overflow.clone());
                Box::new(updated)
            }
            crate::coords::PaddingSpec::Grid { .. } => {
                // FacetRow requires Single padding, not Grid
                panic!("FacetRow::with_measured_padding requires PaddingSpec::Single, got Grid");
            }
        }
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<datafusion::common::ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn crate::coords::PlotGeometry>, AvengerChartError> {
        let row_positions = position_channels.get("row").ok_or_else(|| {
            AvengerChartError::InternalError("Missing 'row' channel for FacetRow transform".into())
        })?;

        let count = row_positions.len();
        if count == 0 {
            return Ok(Box::new(crate::coords::SubplotGeometry::default()));
        }

        let centers = row_positions.as_vec(count, None);
        let (starts, bandwidth) = compute_band_layout(&centers, plot_height, self.padding_px);

        if starts.is_empty() {
            return Ok(Box::new(crate::coords::SubplotGeometry::default()));
        }

        // Extract actual row values from position_values (if provided)
        let row_values = position_values
            .and_then(|pv| pv.get("row"))
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let rects = starts
            .into_iter()
            .enumerate()
            .map(|(i, start)| {
                // Use actual facet value if available, otherwise Null
                let value = row_values.get(i).cloned().unwrap_or(ScalarValue::Null);
                crate::coords::SubplotRect::new(
                    value,
                    0.0,
                    start,
                    plot_width,
                    bandwidth,
                )
            })
            .collect();

        Ok(Box::new(crate::coords::SubplotGeometry::new(rects)))
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
    pub(crate) padding_px: Option<f32>,
    pub(crate) overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
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
        spec: &crate::coords::PaddingSpec,
    ) -> Box<dyn CoordinateSystemTransform> {
        match spec {
            crate::coords::PaddingSpec::Single { padding_px, overflow } => {
                let mut updated = self.clone();
                updated.padding_px = Some(*padding_px);
                updated.overflow_by_facet = Some(overflow.clone());
                Box::new(updated)
            }
            crate::coords::PaddingSpec::Grid { .. } => {
                // FacetColumn requires Single padding, not Grid
                panic!("FacetColumn::with_measured_padding requires PaddingSpec::Single, got Grid");
            }
        }
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn crate::coords::PlotGeometry>, AvengerChartError> {
        let column_positions = position_channels.get("column").ok_or_else(|| {
            AvengerChartError::InternalError(
                "Missing 'column' channel for FacetColumn transform".into(),
            )
        })?;

        let count = column_positions.len();
        if count == 0 {
            return Ok(Box::new(crate::coords::SubplotGeometry::default()));
        }

        let centers = column_positions.as_vec(count, None);
        let (starts, bandwidth) = compute_band_layout(&centers, plot_width, self.padding_px);

        if starts.is_empty() {
            return Ok(Box::new(crate::coords::SubplotGeometry::default()));
        }

        // Extract actual column values from position_values (if provided)
        let column_values = position_values
            .and_then(|pv| pv.get("column"))
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let rects = starts
            .into_iter()
            .enumerate()
            .map(|(i, start)| {
                // Use actual facet value if available, otherwise Null
                let value = column_values.get(i).cloned().unwrap_or(ScalarValue::Null);
                crate::coords::SubplotRect::new(
                    value,
                    start,
                    0.0,
                    bandwidth,
                    plot_height,
                )
            })
            .collect();

        Ok(Box::new(crate::coords::SubplotGeometry::new(rects)))
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
    pub(crate) row_padding_px: Option<f32>,
    pub(crate) col_padding_px: Option<f32>,
    pub(crate) row_overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
    pub(crate) col_overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
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
        spec: &crate::coords::PaddingSpec,
    ) -> Box<dyn CoordinateSystemTransform> {
        match spec {
            crate::coords::PaddingSpec::Grid {
                row_padding_px,
                col_padding_px,
                row_overflow,
                col_overflow
            } => {
                let mut updated = self.clone();
                updated.row_padding_px = Some(*row_padding_px);
                updated.col_padding_px = Some(*col_padding_px);
                updated.row_overflow_by_facet = Some(row_overflow.clone());
                updated.col_overflow_by_facet = Some(col_overflow.clone());
                Box::new(updated)
            }
            crate::coords::PaddingSpec::Single { .. } => {
                // FacetGrid requires Grid padding, not Single
                panic!("FacetGrid::with_measured_padding requires PaddingSpec::Grid, got Single");
            }
        }
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<datafusion::common::ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn crate::coords::PlotGeometry>, AvengerChartError> {
        let row_positions = position_channels.get("row").ok_or_else(|| {
            AvengerChartError::InternalError("Missing 'row' channel for FacetGrid transform".into())
        })?;
        let column_positions = position_channels.get("column").ok_or_else(|| {
            AvengerChartError::InternalError(
                "Missing 'column' channel for FacetGrid transform".into(),
            )
        })?;

        let row_count = row_positions.len();
        let col_count = column_positions.len();
        if row_count == 0 || col_count == 0 {
            return Ok(Box::new(crate::coords::SubplotGeometry::default()));
        }

        let row_centers = row_positions.as_vec(row_count, None);
        let col_centers = column_positions.as_vec(col_count, None);

        let (row_starts, row_bandwidth) =
            compute_band_layout(&row_centers, plot_height, self.row_padding_px);
        let (col_starts, col_bandwidth) =
            compute_band_layout(&col_centers, plot_width, self.col_padding_px);

        if row_starts.is_empty() || col_starts.is_empty() {
            return Ok(Box::new(crate::coords::SubplotGeometry::default()));
        }

        // Extract actual row and column values from position_values (if provided)
        let row_values = position_values
            .and_then(|pv| pv.get("row"))
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let col_values = position_values
            .and_then(|pv| pv.get("column"))
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let mut rects = Vec::with_capacity(row_starts.len() * col_starts.len());
        for (row_idx, &y) in row_starts.iter().enumerate() {
            for (col_idx, &x) in col_starts.iter().enumerate() {
                let row_value = row_values.get(row_idx).cloned().unwrap_or(ScalarValue::Null);
                let col_value = col_values.get(col_idx).cloned().unwrap_or(ScalarValue::Null);

                rects.push(crate::coords::SubplotRect::new_grid(
                    row_value,
                    col_value,
                    row_idx,
                    col_idx,
                    x,
                    y,
                    col_bandwidth,
                    row_bandwidth,
                ));
            }
        }

        Ok(Box::new(crate::coords::SubplotGeometry::new(rects)))
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
    fn facet_row_constructs_struct() {
        let coord = FacetRow {
            padding_px: Some(5.0),
            overflow_by_facet: Some(vec![OverflowSpaceRequirement {
                top: 1.0,
                bottom: 2.0,
                left: 3.0,
                right: 4.0,
            }]),
        };
        assert_eq!(coord.padding_px, Some(5.0));
        assert!(coord.overflow_by_facet.is_some());
    }

    #[test]
    fn facet_grid_sets_fields() {
        let coord = FacetGrid {
            row_padding_px: Some(2.0),
            col_padding_px: Some(3.0),
            row_overflow_by_facet: Some(vec![OverflowSpaceRequirement {
                top: 0.5,
                bottom: 0.5,
                left: 0.25,
                right: 0.25,
            }]),
            col_overflow_by_facet: None,
        };
        assert_eq!(coord.row_padding_px, Some(2.0));
        assert_eq!(coord.col_padding_px, Some(3.0));
        assert!(coord.row_overflow_by_facet.is_some());
        assert!(coord.col_overflow_by_facet.is_none());
    }
}
