use std::{collections::HashMap, sync::Arc};

use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::{DomainKind, RangeKind, ScaleImpl};
use datafusion::{common::ScalarValue as DFScalarValue, dataframe::DataFrame, scalar::ScalarValue};
use serde::{Deserialize, Serialize};

use crate::{
    cartesian::CartesianGuide,
    coords::{
        CoordMeasurement, CoordinateSystem, CoordinateSystemTransform, PlotGeometry, PointGeometry,
    },
    error::AvengerChartError,
    marks::CompiledMark,
    render::EvaluationContext,
    scales::{PlotAreaRangeEndpoint, ScaleRangeBinding},
};

/// Cartesian coordinate system with x and y axes
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Cartesian;

impl CoordinateSystem for Cartesian {
    type Guide = CartesianGuide;

    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for Cartesian {
    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    async fn measure(
        &self,
        scales: &HashMap<String, crate::scales::ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        eval_ctx: &EvaluationContext,
        data: Option<&DataFrame>,
        compiled_marks: &[Arc<dyn CompiledMark>],
        facet_path: &[DFScalarValue],
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        crate::cartesian::positioned_subplot::measure_cartesian_positioned_subplots(
            scales,
            plot_width,
            plot_height,
            eval_ctx,
            data,
            compiled_marks,
            facet_path,
        )
        .await
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<datafusion::common::ScalarValue>>>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        let _ = position_values;
        // In Cartesian coordinates, the scaled values are already in plot coordinates
        // Just extract x and y from the position channels
        let x = position_channels
            .get("x")
            .ok_or_else(|| {
                AvengerChartError::InternalError("Missing x position channel".to_string())
            })?
            .clone();

        let y = position_channels
            .get("y")
            .ok_or_else(|| {
                AvengerChartError::InternalError("Missing y position channel".to_string())
            })?
            .clone();

        Ok(Box::new(PointGeometry { x, y }))
    }

    fn default_range_binding(&self, channel: &str) -> Option<ScaleRangeBinding> {
        match channel {
            "x" => Some(ScaleRangeBinding::plot_area(
                PlotAreaRangeEndpoint::ZERO,
                PlotAreaRangeEndpoint::WIDTH,
            )),
            "y" => Some(ScaleRangeBinding::plot_area(
                PlotAreaRangeEndpoint::HEIGHT,
                PlotAreaRangeEndpoint::ZERO,
            )),
            _ => None,
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, ScalarValue> {
        // Get domain and range kinds directly from scale implementation
        let domain_kind = scale_impl.domain_kind();
        let range_kind = scale_impl.range_kind();
        let scale_type = scale_impl.scale_type();

        let mut options = HashMap::new();

        // Apply coordinate-specific defaults
        if channel == "x" || channel == "y" {
            // For quantitative scales (linear, log, etc.)
            if domain_kind == DomainKind::Numeric && range_kind == RangeKind::Continuous {
                // Include zero for y-axis by default (bar charts) - but only for linear scales
                if channel == "y" && scale_type == "linear" {
                    options.insert("zero".to_string(), ScalarValue::Boolean(Some(true)));
                }

                // Nice domain for better tick values (both axes)
                options.insert("nice".to_string(), ScalarValue::Boolean(Some(true)));

                // Pixel-aligned positions for crisp rendering
                options.insert("round".to_string(), ScalarValue::Boolean(Some(true)));
            }
            // For temporal scales
            else if domain_kind == DomainKind::Temporal && range_kind == RangeKind::Continuous {
                // Pixel-aligned positions
                options.insert("round".to_string(), ScalarValue::Boolean(Some(true)));
            }
            // For categorical scales
            else if domain_kind == DomainKind::Categorical && range_kind == RangeKind::Continuous
            {
                // Check if scale actually supports padding option
                // Band scales typically support padding, point scales may support different padding
                let option_defs = scale_impl.option_definitions();
                if option_defs.iter().any(|def| def.name == "padding") {
                    // Only set padding for band scales (which typically have padding default of 0.0)
                    // Point scales typically default to 0.5 padding already
                    if scale_type == "band" {
                        options.insert("padding".to_string(), ScalarValue::Float64(Some(0.1)));
                    }
                }
            }
        }

        options
    }
}
