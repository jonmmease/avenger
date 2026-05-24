use std::collections::HashMap;

use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::ScaleImpl;
use datafusion::{common::ScalarValue, scalar::ScalarValue as DfScalarValue};
use serde::{Deserialize, Serialize};
use tracing::trace;

use crate::{
    coords::{
        CoordMeasureRequest, CoordMeasurement, CoordinateSystem, CoordinateSystemTransform,
        CoordinateSystemTransformCore, PaddingSpec, PlotGeometry, SubplotGeometry, SubplotRect,
    },
    error::AvengerChartError,
    facet::{coord::measure_facet_row, guide::FacetRowGuideConfig},
    scales::{PlotAreaRangeEndpoint, ScaleRangeBinding},
};

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
///         Subplot::new(
///             Plot::<Cartesian>::new().mark(Symbol::new()...),
///         )
///         .row(col("species")),
///     );
/// ```
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct FacetRow;

impl CoordinateSystem for FacetRow {
    type Guide = FacetRowGuideConfig;

    fn required_channels(&self) -> &'static [&'static str] {
        &["row"]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

pub(crate) fn compute_band_layout(positions: &[f32], extent: f32) -> (Vec<f32>, f32) {
    if positions.is_empty() {
        return (Vec::new(), 0.0);
    }

    let mut sorted = positions.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));

    // Spacing is owned by the band scale configuration (padding_inner[_px]).
    // Derive cell bandwidth directly from the positioned starts and total extent.
    let inferred_bandwidth = if sorted.len() > 1 {
        let first = sorted.first().copied().unwrap_or(0.0);
        let last = sorted.last().copied().unwrap_or(first);
        (extent - (last - first)).max(0.0)
    } else {
        extent.max(0.0)
    };

    // Fallback to min adjacent distance if inference is invalid.
    let fallback_bandwidth = sorted
        .windows(2)
        .filter_map(|pair| {
            let gap = (pair[1] - pair[0]).abs();
            if gap.is_finite() && gap > 0.0 {
                Some(gap)
            } else {
                None
            }
        })
        .fold(f32::INFINITY, f32::min);

    let bandwidth = if inferred_bandwidth.is_finite() && inferred_bandwidth > 0.0 {
        inferred_bandwidth
    } else if fallback_bandwidth.is_finite() && fallback_bandwidth > 0.0 {
        fallback_bandwidth
    } else {
        extent.max(0.0)
    };

    trace!(
        positions = ?positions,
        inferred_bandwidth,
        fallback_bandwidth,
        bandwidth,
        "compute_band_layout"
    );

    // Input positions are already starts (not centers), so use them directly
    let starts: Vec<f32> = positions.to_vec();

    (starts, bandwidth)
}

impl CoordinateSystemTransformCore for FacetRow {
    fn required_channels(&self) -> &'static [&'static str] {
        &["row"]
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<datafusion::common::ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        let row_positions = position_channels.get("row").ok_or_else(|| {
            AvengerChartError::InternalError("Missing 'row' channel for FacetRow transform".into())
        })?;

        let count = row_positions.len();
        if count == 0 {
            return Ok(Box::new(SubplotGeometry::default()));
        }

        let centers = row_positions.as_vec(count, None);
        let (starts, bandwidth) = compute_band_layout(&centers, plot_height);

        if starts.is_empty() {
            return Ok(Box::new(SubplotGeometry::default()));
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
                SubplotRect::new(value, 0.0, start, plot_width, bandwidth)
            })
            .collect();

        Ok(Box::new(SubplotGeometry::new(rects)))
    }

    fn default_range_binding(&self, channel: &str) -> Option<ScaleRangeBinding> {
        match channel {
            "row" => Some(ScaleRangeBinding::plot_area(
                PlotAreaRangeEndpoint::ZERO,
                PlotAreaRangeEndpoint::HEIGHT,
            )),
            _ => None,
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, DfScalarValue> {
        let mut options = HashMap::new();
        if channel == "row" && scale_impl.scale_type() == "band" {
            // Set outer padding to 0 to avoid extra space at top/bottom
            // Set inner padding to 0.1 for default spacing between facets
            options.insert(
                "padding_inner".to_string(),
                DfScalarValue::Float64(Some(0.1)),
            );
            options.insert(
                "padding_outer".to_string(),
                DfScalarValue::Float64(Some(0.0)),
            );
            options.insert("round".to_string(), DfScalarValue::Boolean(Some(true)));
        }
        options
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for FacetRow {
    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    async fn measure(
        &self,
        request: CoordMeasureRequest<'_>,
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        measure_facet_row(
            request.scales(),
            request.plot_width(),
            request.eval_ctx(),
            request.data(),
            request.compiled_marks(),
            request.facet_path(),
        )
        .await
    }

    fn with_measured_padding(&self, _spec: &PaddingSpec) -> Box<dyn CoordinateSystemTransform> {
        // Facet spacing is encoded in the column/row band scale options.
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facet_row_constructs_struct() {
        let _coord = FacetRow;
    }

    #[test]
    fn compute_band_layout_derives_bandwidth_from_scale_positions() {
        let positions = vec![0.0, 100.0, 200.0];
        let (starts, bandwidth) = compute_band_layout(&positions, 260.0);
        assert_eq!(starts, positions);
        assert!((bandwidth - 60.0).abs() < 0.01);
    }

    #[test]
    fn compute_band_layout_falls_back_when_inference_invalid() {
        let positions = vec![10.0, 20.0, 30.0];
        let (_starts, bandwidth) = compute_band_layout(&positions, 0.0);
        assert!((bandwidth - 10.0).abs() < 0.01);
    }
}
