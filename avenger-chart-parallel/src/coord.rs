use std::{
    any::Any,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use avenger_chart_core::{
    AvengerChartError, Axis, ChannelValue, CoordinateScaleSource, CoordinateSystem,
    CoordinateSystemCore, CoordinateSystemTransform, CoordinateSystemTransformCore, DataContext,
    GenericPositionConfig, IntoExpr, PlotAreaRangeEndpoint, PlotGeometry, PointGeometry,
    PositionConfig, ScaleRangeBinding, ScaleTypePreference, validate_structural_id,
};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_scales::scales::ScaleImpl;
use datafusion::{arrow::datatypes::DataType, common::ScalarValue, prelude::lit};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{ParallelAxis, ParallelGuide};
use crate::{ParallelFrameGeometry, resolve_parallel_frame};

/// Prefix for hidden generated dimension channels.
pub const PARALLEL_DIMENSION_CHANNEL_PREFIX: &str = "__avenger_parallel_dim_";

/// Position-channel configuration for a parallel-coordinate dimension.
pub type ParallelDimensionConfig = GenericPositionConfig<ParallelAxis>;

/// Return the hidden generated channel name for a dimension id.
pub fn generated_dimension_channel(id: &str) -> String {
    format!("{PARALLEL_DIMENSION_CHANNEL_PREFIX}{id}")
}

/// Return the dimension id encoded in a hidden generated channel.
pub fn dimension_id_from_generated_channel(channel: &str) -> Option<&str> {
    channel.strip_prefix(PARALLEL_DIMENSION_CHANNEL_PREFIX)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParallelDimensionSpec {
    pub id: String,
    pub generated_channel: String,
    pub channel_value: ChannelValue,
    pub axis: Option<ParallelAxis>,
}

/// Wide-form parallel-coordinate system.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Parallel {
    dimensions: Vec<ParallelDimensionSpec>,
    order: Option<Vec<String>>,
}

impl Parallel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn dimension(self, id: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.dimension_with(id, expr, |dimension| dimension)
    }

    pub fn dimension_with<F>(
        mut self,
        id: impl Into<String>,
        expr: impl IntoExpr,
        configure: F,
    ) -> Self
    where
        F: FnOnce(ParallelDimensionConfig) -> ParallelDimensionConfig,
    {
        let id = id.into();
        let generated_channel = generated_dimension_channel(&id);
        let config = ParallelDimensionConfig::new(ChannelValue::from(expr.into_expr()))
            .with_scale_name(id.clone());
        let (channel_value, axis) = configure(config).take_axis_config();
        self.dimensions.push(ParallelDimensionSpec {
            id: id.clone(),
            generated_channel,
            channel_value: channel_value.with_scale_name(id),
            axis,
        });
        self
    }

    pub fn order<I, S>(mut self, order: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.order = Some(order.into_iter().map(Into::into).collect());
        self
    }

    pub fn dimensions(&self) -> &[ParallelDimensionSpec] {
        &self.dimensions
    }

    pub fn order_ids(&self) -> Option<&[String]> {
        self.order.as_deref()
    }
}

impl CoordinateSystemCore for Parallel {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn validate(&self) -> Result<(), AvengerChartError> {
        let mut seen = HashSet::new();
        for dimension in &self.dimensions {
            validate_dimension_id(&dimension.id)?;
            if !seen.insert(dimension.id.as_str()) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Duplicate parallel dimension id '{}'",
                    dimension.id
                )));
            }
        }

        if let Some(order) = &self.order {
            if order.len() != self.dimensions.len() {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Parallel dimension order must contain exactly {} id(s), got {}",
                    self.dimensions.len(),
                    order.len()
                )));
            }
            let ids = self
                .dimensions
                .iter()
                .map(|dimension| dimension.id.as_str())
                .collect::<HashSet<_>>();
            let mut ordered = HashSet::new();
            for id in order {
                validate_dimension_id(id)?;
                if !ids.contains(id.as_str()) {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Parallel dimension order references unknown id '{id}'"
                    )));
                }
                if !ordered.insert(id.as_str()) {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Parallel dimension order repeats id '{id}'"
                    )));
                }
            }
        }

        Ok(())
    }
}

impl CoordinateSystem for Parallel {
    type Guide = ParallelGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(ParallelTransform {
            dimensions: self
                .dimensions
                .iter()
                .map(|dimension| ParallelTransformDimension {
                    id: dimension.id.clone(),
                    generated_channel: dimension.generated_channel.clone(),
                    channel_value: dimension
                        .channel_value
                        .clone()
                        .with_scale_name(dimension.id.clone()),
                })
                .collect(),
            order: self.order.clone(),
        })
    }

    fn coordinate_scale_sources(&self) -> Vec<CoordinateScaleSource> {
        let mut data = DataContext::default();
        let mut axis_configs: HashMap<String, Arc<dyn Axis>> = HashMap::new();

        for dimension in &self.dimensions {
            data = data.with_channel_value(
                &dimension.generated_channel,
                dimension
                    .channel_value
                    .clone()
                    .with_scale_name(dimension.id.clone()),
            );
            let mut axis = dimension.axis.clone().unwrap_or_default();
            axis.set_default_title_expr(lit(dimension.id.clone()))
                .expect("literal parallel axis title should serialize");
            axis_configs.insert(dimension.generated_channel.clone(), Arc::new(axis));
        }

        vec![CoordinateScaleSource {
            data,
            axis_configs,
            ..CoordinateScaleSource::default()
        }]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParallelTransformDimension {
    pub id: String,
    pub generated_channel: String,
    pub channel_value: ChannelValue,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ParallelTransform {
    pub dimensions: Vec<ParallelTransformDimension>,
    pub order: Option<Vec<String>>,
}

impl ParallelTransform {
    pub fn dimension_for_channel(&self, channel: &str) -> Option<&ParallelTransformDimension> {
        self.dimensions
            .iter()
            .find(|dimension| dimension.generated_channel == channel || dimension.id == channel)
    }

    pub fn resolve_frame(&self, width: f32) -> ParallelFrameGeometry {
        resolve_parallel_frame(&self.dimensions, self.order.as_deref(), None, width)
    }

    pub fn ordered_dimensions(&self) -> Vec<&ParallelTransformDimension> {
        let Some(order) = self.order.as_deref() else {
            return self.dimensions.iter().collect();
        };
        order
            .iter()
            .filter_map(|id| self.dimensions.iter().find(|dimension| dimension.id == *id))
            .collect()
    }
}

impl CoordinateSystemTransformCore for ParallelTransform {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn channel_uses_scale(&self, channel: &str) -> bool {
        self.dimension_for_channel(channel).is_some()
    }

    fn is_position_scale_channel(&self, channel: &str) -> bool {
        self.dimension_for_channel(channel).is_some()
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        _position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        let len = position_channels
            .values()
            .find_map(|value| match value.value() {
                ScalarOrArrayValue::Array(values) => Some(values.len()),
                ScalarOrArrayValue::Scalar(_) => None,
            })
            .unwrap_or(1);
        let x = plot_width / 2.0;
        let y = plot_height / 2.0;
        let (x, y) = if len == 1 {
            (ScalarOrArray::new_scalar(x), ScalarOrArray::new_scalar(y))
        } else {
            (
                ScalarOrArray::new_array(vec![x; len]),
                ScalarOrArray::new_array(vec![y; len]),
            )
        };
        Ok(Box::new(PointGeometry { x, y }))
    }

    fn default_range_binding(&self, channel: &str) -> Option<ScaleRangeBinding> {
        self.dimension_for_channel(channel).map(|_| {
            ScaleRangeBinding::plot_area(PlotAreaRangeEndpoint::HEIGHT, PlotAreaRangeEndpoint::ZERO)
        })
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, ScalarValue> {
        HashMap::new()
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        if self.dimension_for_channel(channel).is_none() {
            return None;
        }
        match data_type {
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View | DataType::Boolean => {
                Some(ScaleTypePreference::Point)
            }
            _ => None,
        }
    }

    fn generated_position_channels(&self) -> IndexMap<String, ChannelValue> {
        self.ordered_dimensions()
            .into_iter()
            .map(|dimension| {
                (
                    dimension.generated_channel.clone(),
                    dimension
                        .channel_value
                        .clone()
                        .with_scale_name(dimension.id.clone()),
                )
            })
            .collect()
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for ParallelTransform {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

fn validate_dimension_id(id: &str) -> Result<(), AvengerChartError> {
    validate_structural_id("parallel dimension", id)?;
    if id.starts_with(PARALLEL_DIMENSION_CHANNEL_PREFIX) {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Invalid parallel dimension id '{id}': ids may not start with '{PARALLEL_DIMENSION_CHANNEL_PREFIX}'"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::prelude::{SessionContext, col};

    #[test]
    fn generated_dimension_channels_are_reversible() {
        let channel = generated_dimension_channel("mpg");
        assert_eq!(channel, "__avenger_parallel_dim_mpg");
        assert_eq!(dimension_id_from_generated_channel(&channel), Some("mpg"));
    }

    #[test]
    fn dimension_stores_id_expr_generated_channel_and_scale_name() {
        let parallel = Parallel::new().dimension("mpg", col("mpg"));
        let dimension = &parallel.dimensions()[0];
        assert_eq!(dimension.id, "mpg");
        assert_eq!(
            dimension.generated_channel,
            generated_dimension_channel("mpg")
        );
        assert_eq!(
            dimension
                .channel_value
                .get_scale_name(&dimension.generated_channel),
            Some("mpg".to_string())
        );
        let ctx = SessionContext::new();
        assert_eq!(
            dimension
                .channel_value
                .expr(&ctx)
                .expect("dimension expression")
                .to_string(),
            "mpg"
        );
    }

    #[test]
    fn dimension_with_stores_axis_config() {
        let parallel = Parallel::new().dimension_with("origin", col("origin"), |dimension| {
            dimension.axis(|axis| axis.title("Origin"))
        });
        assert!(
            parallel.dimensions()[0]
                .axis
                .as_ref()
                .expect("axis config")
                .title
                .is_set()
        );
    }

    #[test]
    fn validate_rejects_duplicate_and_invalid_ids() {
        let duplicate = Parallel::new()
            .dimension("mpg", col("mpg"))
            .dimension("mpg", col("mpg2"));
        assert!(
            duplicate
                .validate()
                .unwrap_err()
                .to_string()
                .contains("Duplicate")
        );

        let invalid = Parallel::new().dimension("bad id", col("mpg"));
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .to_string()
                .contains("Invalid")
        );
    }

    #[test]
    fn explicit_order_must_match_dimension_ids() {
        let parallel = Parallel::new()
            .dimension("a", col("a"))
            .dimension("b", col("b"))
            .order(["b", "a"]);
        parallel.validate().expect("valid order");

        let missing = Parallel::new()
            .dimension("a", col("a"))
            .dimension("b", col("b"))
            .order(["b"]);
        assert!(
            missing
                .validate()
                .unwrap_err()
                .to_string()
                .contains("exactly")
        );

        let unknown = Parallel::new()
            .dimension("a", col("a"))
            .dimension("b", col("b"))
            .order(["b", "c"]);
        assert!(
            unknown
                .validate()
                .unwrap_err()
                .to_string()
                .contains("unknown")
        );
    }

    #[test]
    fn serde_round_trip_preserves_dimensions_and_order() {
        let parallel = Parallel::new()
            .dimension("mpg", col("mpg"))
            .dimension_with("origin", col("origin"), |dimension| {
                dimension.axis(|axis| axis.title("Origin"))
            })
            .order(["origin", "mpg"]);
        let json = serde_json::to_string(&parallel).expect("serialize");
        let decoded: Parallel = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.dimensions().len(), 2);
        assert_eq!(
            decoded.order_ids(),
            Some(["origin".to_string(), "mpg".to_string()].as_slice())
        );
    }

    #[test]
    fn coordinate_scale_source_exposes_generated_dimension_channels() {
        let parallel = Parallel::new().dimension("mpg", col("mpg")).dimension_with(
            "origin",
            col("origin"),
            |dimension| dimension.axis(|axis| axis.title("Origin")),
        );
        let sources = parallel.coordinate_scale_sources();
        assert_eq!(sources.len(), 1);
        let channels = sources[0].data.channels();
        assert!(channels.contains_key(&generated_dimension_channel("mpg")));
        assert!(channels.contains_key(&generated_dimension_channel("origin")));
        assert!(
            sources[0]
                .axis_configs
                .contains_key(&generated_dimension_channel("origin"))
        );
    }

    #[test]
    fn transform_classifies_dimension_scales_as_positional() {
        let transform = Parallel::new()
            .dimension("mpg", col("mpg"))
            .create_transform();
        assert!(transform.channel_uses_scale(&generated_dimension_channel("mpg")));
        assert!(transform.is_position_scale_channel(&generated_dimension_channel("mpg")));
        assert!(transform.is_position_scale_channel("mpg"));
        assert!(transform.default_range_binding("mpg").is_some());
    }
}
