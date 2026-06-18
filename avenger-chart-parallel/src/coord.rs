use std::{
    any::Any,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use avenger_chart_core::{
    AvengerChartError, Axis, ChannelValue, CoordinateScaleSource, CoordinateSystem,
    CoordinateSystemCore, CoordinateSystemTransform, CoordinateSystemTransformCore, DataContext,
    GeneratedPositionSlot, GenericPositionConfig, IntoExpr, PlotAreaRangeEndpoint, PlotGeometry,
    PointGeometry, PositionConfig, RepeatContext, ScaleRangeBinding, ScaleTypePreference,
    repeat_placeholder_kind_from_id, resolve_repeat_channel_value, resolve_repeat_placeholders,
    validate_structural_id,
};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_scales::scales::{DomainKind, ScaleImpl};
use datafusion::{arrow::datatypes::DataType, common::ScalarValue, prelude::lit};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::frame::{resolve_display_state, resolve_order_state, validate_order_ids};
use crate::{
    ParallelAxis, ParallelDisplayState, ParallelFrameGeometry, ParallelGuide, ParallelOrderState,
    resolve_parallel_frame,
};

/// Prefix for hidden generated dimension channels.
pub const PARALLEL_DIMENSION_CHANNEL_PREFIX: &str = "__avenger_parallel_dim_";

/// Hidden event-coordinate channel for local plot-area x pixels.
pub const PARALLEL_LOCAL_X_CHANNEL: &str = "__avenger_parallel_local_x";

/// Hidden event-coordinate channel for local plot-area y pixels.
pub const PARALLEL_LOCAL_Y_CHANNEL: &str = "__avenger_parallel_local_y";

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
///
/// Dimension ids are stable structural ids used as scale names, order-state
/// values, guide event datum fields, and axis-overlay targets. Repeat
/// placeholders are supported in dimension expressions and axis expressions,
/// but not in dimension ids themselves.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Parallel {
    dimensions: Vec<ParallelDimensionSpec>,
    order: Option<Vec<String>>,
    order_state: Option<ParallelOrderState>,
    display_state: Option<ParallelDisplayState>,
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

    pub fn order_state(mut self, state: ParallelOrderState) -> Self {
        self.order_state = Some(state);
        self
    }

    /// Read committed axis order from a runtime parameter.
    ///
    /// The parameter should contain a nullable list of dimension id strings.
    /// Missing or null values fall back to static `order(...)` and then
    /// declaration order.
    pub fn order_param(self, param: impl Into<String>) -> Self {
        self.order_state(ParallelOrderState::param(param))
    }

    pub fn display_state(mut self, state: ParallelDisplayState) -> Self {
        self.display_state = Some(state);
        self
    }

    /// Read an active dragged axis id and display x from runtime parameters.
    ///
    /// This drives transient drag previews without changing the committed
    /// equilibrium order.
    pub fn active_axis_display_params(
        self,
        dimension_id_param: impl Into<String>,
        display_x_param: impl Into<String>,
    ) -> Self {
        self.display_state(ParallelDisplayState::active_axis(
            dimension_id_param,
            display_x_param,
        ))
    }

    pub fn dimensions(&self) -> &[ParallelDimensionSpec] {
        &self.dimensions
    }

    pub fn order_ids(&self) -> Option<&[String]> {
        self.order.as_deref()
    }

    pub fn order_state_ref(&self) -> Option<&ParallelOrderState> {
        self.order_state.as_ref()
    }

    pub fn display_state_ref(&self) -> Option<&ParallelDisplayState> {
        self.display_state.as_ref()
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
            for id in order {
                validate_dimension_id(id)?;
            }
            let ids = self
                .dimensions
                .iter()
                .map(|dimension| dimension.id.clone())
                .collect::<Vec<_>>();
            validate_order_ids(order, &ids, "Parallel dimension order")?;
        }

        Ok(())
    }

    fn resolve_repeat(&self, ctx: &RepeatContext) -> Result<Self, AvengerChartError> {
        let dimensions = self
            .dimensions
            .iter()
            .map(|dimension| {
                let axis = dimension
                    .axis
                    .as_ref()
                    .map(|axis| {
                        let mapped =
                            axis.map_exprs(&mut |expr| resolve_repeat_placeholders(expr, ctx))?;
                        mapped
                            .as_any()
                            .downcast_ref::<ParallelAxis>()
                            .cloned()
                            .ok_or_else(|| {
                                AvengerChartError::InternalError(
                                    "Parallel axis repeat resolution returned non-parallel axis"
                                        .to_string(),
                                )
                            })
                    })
                    .transpose()?;
                Ok(ParallelDimensionSpec {
                    id: dimension.id.clone(),
                    generated_channel: dimension.generated_channel.clone(),
                    channel_value: resolve_repeat_channel_value(
                        dimension.channel_value.clone(),
                        ctx,
                    )?,
                    axis,
                })
            })
            .collect::<Result<Vec<_>, AvengerChartError>>()?;
        Ok(Self {
            dimensions,
            order: self.order.clone(),
            order_state: self.order_state.clone(),
            display_state: self.display_state.clone(),
        })
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
            order_state: self.order_state.clone(),
            display_state: self.display_state.clone(),
        })
    }

    fn coordinate_scale_sources(&self) -> Vec<CoordinateScaleSource> {
        let mut data = DataContext::default();
        let mut axis_configs: HashMap<String, Arc<dyn Axis>> = HashMap::new();
        let order_index_by_id = self
            .order
            .as_ref()
            .map(|order| {
                order
                    .iter()
                    .enumerate()
                    .map(|(index, id)| (id.as_str(), index))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_else(|| {
                self.dimensions
                    .iter()
                    .enumerate()
                    .map(|(index, dimension)| (dimension.id.as_str(), index))
                    .collect()
            });

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
            axis = axis.with_dimension_metadata(
                dimension.id.clone(),
                *order_index_by_id
                    .get(dimension.id.as_str())
                    .unwrap_or(&usize::MAX),
            );
            axis = axis.with_frame_state(self.order_state.clone(), self.display_state.clone());
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
    pub order_state: Option<ParallelOrderState>,
    pub display_state: Option<ParallelDisplayState>,
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

    pub fn resolve_frame_with_params(
        &self,
        width: f32,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<ParallelFrameGeometry, AvengerChartError> {
        let dimension_ids = self
            .dimensions
            .iter()
            .map(|dimension| dimension.id.clone())
            .collect::<Vec<_>>();
        let param_order = resolve_order_state(self.order_state.as_ref(), params, &dimension_ids)?;
        let display_overrides =
            resolve_display_state(self.display_state.as_ref(), params, &dimension_ids)?;
        Ok(resolve_parallel_frame(
            &self.dimensions,
            param_order.as_deref().or(self.order.as_deref()),
            display_overrides.as_ref(),
            width,
        ))
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
        !matches!(channel, PARALLEL_LOCAL_X_CHANNEL | PARALLEL_LOCAL_Y_CHANNEL)
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

    fn generated_position_slots(
        &self,
        plot_width: f32,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<Vec<GeneratedPositionSlot>, AvengerChartError> {
        Ok(self
            .resolve_frame_with_params(plot_width, params)?
            .slots
            .into_iter()
            .map(|slot| GeneratedPositionSlot {
                channel: slot.generated_channel,
                id: slot.id,
                scale_name: slot.scale_name,
                order_index: slot.equilibrium_index,
                equilibrium_x: slot.equilibrium_x,
                display_x: slot.display_x,
                displacement_px: slot.displacement_px,
                displacement_slots: slot.displacement_slots,
            })
            .collect())
    }

    fn interaction_invertible_channels(&self) -> Vec<String> {
        let mut channels = Vec::with_capacity(self.dimensions.len() + 2);
        channels.push(PARALLEL_LOCAL_X_CHANNEL.to_string());
        channels.push(PARALLEL_LOCAL_Y_CHANNEL.to_string());
        channels.extend(self.ordered_dimensions().into_iter().map(|d| d.id.clone()));
        channels
    }

    fn invert_interaction_point(
        &self,
        request: avenger_chart_core::InteractionPointInversionRequest<'_>,
    ) -> Result<IndexMap<String, ScalarValue>, AvengerChartError> {
        let mut inverted = IndexMap::new();
        for channel in request.channels {
            if *channel == PARALLEL_LOCAL_X_CHANNEL {
                inverted.insert(
                    (*channel).to_string(),
                    ScalarValue::Float64(Some(request.local_point[0] as f64)),
                );
                continue;
            }
            if *channel == PARALLEL_LOCAL_Y_CHANNEL {
                inverted.insert(
                    (*channel).to_string(),
                    ScalarValue::Float64(Some(request.local_point[1] as f64)),
                );
                continue;
            }

            let dimension = self.dimension_for_channel(channel).ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "Parallel coordinate inversion does not support channel '{channel}'"
                ))
            })?;
            let scale = request.scales.get(&dimension.id).ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "Missing configured scale for parallel dimension '{}'",
                    dimension.id
                ))
            })?;

            let range_value = request.local_point[1];
            let value = if matches!(
                scale.scale_impl.domain_kind(),
                DomainKind::Categorical | DomainKind::NestedCategorical
            ) {
                let values = scale
                    .invert_range_interval((range_value, range_value))
                    .map_err(|err| {
                        AvengerChartError::InvalidArgument(format!(
                            "Cannot invert parallel dimension '{}' (scale type {}): {err}",
                            dimension.id,
                            scale.scale_impl.scale_type()
                        ))
                    })?;
                if values.is_empty() {
                    continue;
                }
                ScalarValue::try_from_array(values.as_ref(), 0).map_err(|err| {
                    AvengerChartError::InvalidArgument(format!(
                        "Cannot convert inverted parallel dimension '{}' value: {err}",
                        dimension.id
                    ))
                })?
            } else {
                let value = scale.invert_scalar(range_value).map_err(|err| {
                    AvengerChartError::InvalidArgument(format!(
                        "Cannot invert parallel dimension '{}' (scale type {}): {err}",
                        dimension.id,
                        scale.scale_impl.scale_type()
                    ))
                })?;
                ScalarValue::Float64(Some(value as f64))
            };
            inverted.insert((*channel).to_string(), value);
        }
        Ok(inverted)
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
    if repeat_placeholder_kind_from_id(id).is_some() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Invalid parallel dimension id '{id}': dimension ids must be stable structural ids; use a stable id and put repeat placeholders in the dimension expression instead"
        )));
    }
    validate_structural_id("parallel dimension", id)?;
    if id == PARALLEL_LOCAL_X_CHANNEL || id == PARALLEL_LOCAL_Y_CHANNEL {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Invalid parallel dimension id '{id}': id is reserved for interaction readback"
        )));
    }
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
    use avenger_chart_core::repeat;
    use avenger_scales::scales::linear::LinearScale;
    use datafusion::{
        arrow::datatypes::DataType,
        prelude::{SessionContext, col},
    };

    fn string_list(values: &[&str]) -> ScalarValue {
        ScalarValue::List(ScalarValue::new_list(
            &values
                .iter()
                .map(|value| ScalarValue::Utf8(Some((*value).to_string())))
                .collect::<Vec<_>>(),
            &DataType::Utf8,
            true,
        ))
    }

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
    fn validate_rejects_repeat_placeholder_dimension_ids() {
        let invalid = Parallel::new().dimension(repeat::column_name(), repeat::column());
        let err = invalid.validate().unwrap_err().to_string();
        assert!(err.contains("dimension ids must be stable"), "{err}");
        assert!(err.contains("dimension expression"), "{err}");
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
            .order(["origin", "mpg"])
            .order_param("axis_order")
            .active_axis_display_params("drag_dimension", "drag_display_x");
        let json = serde_json::to_string(&parallel).expect("serialize");
        let decoded: Parallel = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.dimensions().len(), 2);
        assert_eq!(
            decoded.order_ids(),
            Some(["origin".to_string(), "mpg".to_string()].as_slice())
        );
        assert_eq!(
            decoded.order_state_ref().map(|state| state.param.as_str()),
            Some("axis_order")
        );
        assert_eq!(
            decoded
                .display_state_ref()
                .map(|state| state.display_x_param.as_str()),
            Some("drag_display_x")
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
        assert!(transform.channel_uses_scale("stroke"));
        assert!(!transform.channel_uses_scale(PARALLEL_LOCAL_X_CHANNEL));
        assert!(!transform.channel_uses_scale(PARALLEL_LOCAL_Y_CHANNEL));
        assert!(transform.is_position_scale_channel(&generated_dimension_channel("mpg")));
        assert!(transform.is_position_scale_channel("mpg"));
        assert!(transform.default_range_binding("mpg").is_some());
    }

    #[test]
    fn interaction_channels_include_local_x_and_ordered_dimensions() {
        let transform = Parallel::new()
            .dimension("mpg", col("mpg"))
            .dimension("weight", col("weight"))
            .order(["weight", "mpg"])
            .create_transform();

        assert_eq!(
            transform.interaction_invertible_channels(),
            vec![
                PARALLEL_LOCAL_X_CHANNEL.to_string(),
                PARALLEL_LOCAL_Y_CHANNEL.to_string(),
                "weight".to_string(),
                "mpg".to_string()
            ]
        );
    }

    #[test]
    fn resolve_frame_with_params_uses_order_and_display_state() {
        let transform = Parallel::new()
            .dimension("speed", col("speed"))
            .dimension("cost", col("cost"))
            .dimension("stability", col("stability"))
            .order_param("axis_order")
            .active_axis_display_params("drag_dimension", "drag_display_x")
            .create_transform();
        let transform = transform
            .as_any()
            .downcast_ref::<ParallelTransform>()
            .expect("parallel transform");
        let params = IndexMap::from([
            (
                "axis_order".to_string(),
                string_list(&["cost", "speed", "stability"]),
            ),
            (
                "drag_dimension".to_string(),
                ScalarValue::Utf8(Some("speed".to_string())),
            ),
            (
                "drag_display_x".to_string(),
                ScalarValue::Float64(Some(180.0)),
            ),
        ]);
        let frame = transform
            .resolve_frame_with_params(300.0, &params)
            .expect("stateful frame");

        assert_eq!(
            frame
                .slots
                .iter()
                .map(|slot| slot.id.as_str())
                .collect::<Vec<_>>(),
            vec!["cost", "speed", "stability"]
        );
        assert_eq!(frame.slot("cost").unwrap().equilibrium_x, 0.0);
        assert_eq!(frame.slot("speed").unwrap().equilibrium_x, 150.0);
        assert_eq!(frame.slot("speed").unwrap().display_x, 180.0);
        assert_eq!(frame.slot("speed").unwrap().displacement_px, 30.0);
        assert_eq!(frame.slot("stability").unwrap().equilibrium_x, 300.0);
    }

    #[test]
    fn generated_position_slots_include_stateful_display_geometry() {
        let transform = Parallel::new()
            .dimension("speed", col("speed"))
            .dimension("cost", col("cost"))
            .active_axis_display_params("drag_dimension", "drag_display_x")
            .create_transform();
        let params = IndexMap::from([
            (
                "drag_dimension".to_string(),
                ScalarValue::Utf8(Some("cost".to_string())),
            ),
            (
                "drag_display_x".to_string(),
                ScalarValue::Float64(Some(65.0)),
            ),
        ]);
        let slots = transform
            .generated_position_slots(100.0, &params)
            .expect("generated slots");

        assert_eq!(slots[0].id, "speed");
        assert_eq!(slots[0].channel, generated_dimension_channel("speed"));
        assert_eq!(slots[0].display_x, 0.0);
        assert_eq!(slots[1].id, "cost");
        assert_eq!(slots[1].channel, generated_dimension_channel("cost"));
        assert_eq!(slots[1].equilibrium_x, 100.0);
        assert_eq!(slots[1].display_x, 65.0);
        assert_eq!(slots[1].displacement_px, -35.0);
    }

    #[test]
    fn interaction_inverts_local_x_and_dimension_y() {
        let transform = Parallel::new()
            .dimension("mpg", col("mpg"))
            .create_transform();
        let mut scales = HashMap::new();
        scales.insert(
            "mpg".to_string(),
            LinearScale::configured((0.0, 40.0), (200.0, 0.0)),
        );

        let inverted = transform
            .invert_interaction_point(avenger_chart_core::InteractionPointInversionRequest {
                local_point: [32.0, 50.0],
                plot_area_width: 300.0,
                plot_area_height: 200.0,
                channels: &[PARALLEL_LOCAL_X_CHANNEL, PARALLEL_LOCAL_Y_CHANNEL, "mpg"],
                scales: &scales,
            })
            .expect("invert parallel interaction point");

        match inverted.get(PARALLEL_LOCAL_X_CHANNEL) {
            Some(ScalarValue::Float64(Some(value))) => assert!((value - 32.0).abs() < 1e-6),
            other => panic!("expected local x=32, got {other:?}"),
        }
        match inverted.get(PARALLEL_LOCAL_Y_CHANNEL) {
            Some(ScalarValue::Float64(Some(value))) => assert!((value - 50.0).abs() < 1e-6),
            other => panic!("expected local y=50, got {other:?}"),
        }
        match inverted.get("mpg") {
            Some(ScalarValue::Float64(Some(value))) => assert!((value - 30.0).abs() < 1e-6),
            other => panic!("expected mpg=30, got {other:?}"),
        }
    }
}
