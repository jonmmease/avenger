use std::{any::Any, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, ChannelValue, CompiledDataContext, CompiledMark,
    CompiledMarkCore, CompiledMarkState, CompiledSubplotPayload, CoordinateSystemTransformCore,
    DefaultLogicalExprNodeExt, Mark, MarkRuntimeContext, PositionConfig, RadiusExpression,
    SerializableExpr, SubplotContainerCoordinateSystem, SubplotMarkCore, compile_subplot_payload,
    contains_aggregate,
};
use avenger_chart_marks::Subplot;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::{arrow::record_batch::RecordBatch, logical_expr::Expr, prelude::SessionContext};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{Cartesian, CartesianPositionConfig};

#[doc(hidden)]
pub const CARTESIAN_SUBPLOT_PARTITION_CHANNEL: &str = "partition";

/// Position-channel builder methods for `Subplot<Cartesian>`.
pub trait CartesianSubplotPositionChannels: Sized {
    /// Set the parent x-position for coordinate-positioned child plot frames.
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self;

    /// Set the parent y-position for coordinate-positioned child plot frames.
    fn y<V: Into<ChannelValue>>(self, value: V) -> Self;

    /// Partition parent data into one coordinate-positioned child frame per value.
    fn partition_by<V: Into<ChannelValue>>(self, value: V) -> Self;

    /// Configure the parent x-position channel for coordinate-positioned child plot frames.
    fn x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig;

    /// Configure the parent y-position channel for coordinate-positioned child plot frames.
    fn y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig;

    /// Set the child plot-area width used for each positioned child frame.
    fn plot_width(self, width: f32) -> Self;

    /// Set the child plot-area height used for each positioned child frame.
    fn plot_height(self, height: f32) -> Self;

    /// Set both child plot-area dimensions used for each positioned child frame.
    fn plot_size(self, width: f32, height: f32) -> Self;
}

impl CartesianSubplotPositionChannels for Subplot<Cartesian> {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    fn y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    fn partition_by<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value(CARTESIAN_SUBPLOT_PARTITION_CHANNEL, value.into().no_scale())
    }

    fn x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        with_position_config(self, "x", value.into(), f)
    }

    fn y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        with_position_config(self, "y", value.into(), f)
    }

    fn plot_width(mut self, width: f32) -> Self {
        self.set_plot_width_config(Some(width));
        self
    }

    fn plot_height(mut self, height: f32) -> Self {
        self.set_plot_height_config(Some(height));
        self
    }

    fn plot_size(mut self, width: f32, height: f32) -> Self {
        self.set_plot_width_config(Some(width));
        self.set_plot_height_config(Some(height));
        self
    }
}

fn with_position_config<F>(
    mark: Subplot<Cartesian>,
    channel: &'static str,
    value: ChannelValue,
    f: F,
) -> Subplot<Cartesian>
where
    F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
{
    let configured = f(CartesianPositionConfig::new(value));
    let (channel_value, axis_config) = configured.take_axis_config();
    let mut mark = mark.with_channel_value(channel, channel_value);
    if let Some(axis_config) = axis_config {
        mark.state_mut()
            .axis_configs
            .insert(channel.to_string(), Arc::new(axis_config));
    }
    mark
}

fn partition_expr_node(
    subplot: &dyn SubplotMarkCore,
    session_context: &SessionContext,
) -> Result<Option<LogicalExprNode>, AvengerChartError> {
    let Some(channel) = subplot
        .data_context_ref()
        .channels()
        .get(CARTESIAN_SUBPLOT_PARTITION_CHANNEL)
    else {
        return Ok(None);
    };

    let Some(expr) = channel.expr(session_context) else {
        return Err(AvengerChartError::InvalidArgument(
            "Cartesian subplot partition_by does not support conditional values".to_string(),
        ));
    };

    validate_partitioned_subplot(subplot, session_context, &expr)?;
    LogicalExprNode::from_expr(expr).map(Some)
}

fn validate_partitioned_subplot(
    subplot: &dyn SubplotMarkCore,
    session_context: &SessionContext,
    partition_expr: &Expr,
) -> Result<(), AvengerChartError> {
    if subplot.has_plot_level_data() {
        return Err(AvengerChartError::InvalidArgument(
            "Partitioned Cartesian subplots inherit parent data; remove plot-level data from the child plot".to_string(),
        ));
    }

    for channel_name in ["x", "y"] {
        let Some(channel) = subplot.data_context_ref().channels().get(channel_name) else {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Partitioned Cartesian subplots require channel `{channel_name}`"
            )));
        };
        let Some(expr) = channel.expr(session_context) else {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Partitioned Cartesian subplot channel `{channel_name}` does not support conditional values"
            )));
        };

        if !valid_partition_position_expr(&expr, partition_expr) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Partitioned Cartesian subplot channel `{channel_name}` must be an aggregate, literal/constant, or the partition expression"
            )));
        }
    }

    Ok(())
}

fn valid_partition_position_expr(expr: &Expr, partition_expr: &Expr) -> bool {
    contains_aggregate(expr)
        || !expr.any_column_refs()
        || expr == partition_expr
        || expr.to_string() == partition_expr.to_string()
}

#[async_trait::async_trait]
impl SubplotContainerCoordinateSystem for Cartesian {
    async fn compile_subplot_mark(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        subplot.validate_no_facet_channels("Cartesian")?;
        let partition_expr = partition_expr_node(subplot, session_context)?;

        Ok(Arc::new(CompiledCartesianSubplot {
            payload: compile_subplot_payload(subplot, compiled_state, session_context).await?,
            plot_width: subplot.plot_width_config().unwrap_or(80.0).max(1.0),
            plot_height: subplot.plot_height_config().unwrap_or(80.0).max(1.0),
            partition_expr,
        }))
    }
}

/// Compiled child-plot mark positioned by Cartesian x/y channels.
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianSubplot {
    payload: CompiledSubplotPayload,
    plot_width: f32,
    plot_height: f32,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    partition_expr: Option<LogicalExprNode>,
}

impl CompiledCartesianSubplot {
    pub fn payload(&self) -> &CompiledSubplotPayload {
        &self.payload
    }

    pub fn label(&self) -> Option<&str> {
        self.payload.label()
    }

    pub fn key(&self) -> Option<&str> {
        self.payload.key()
    }

    pub fn mark_index(&self) -> usize {
        self.payload.mark_index()
    }

    pub fn plot_width(&self) -> f32 {
        self.plot_width
    }

    pub fn plot_height(&self) -> f32 {
        self.plot_height
    }

    #[doc(hidden)]
    pub fn is_partitioned(&self) -> bool {
        self.partition_expr.is_some()
    }

    #[doc(hidden)]
    pub fn partition_expr(&self) -> Option<&LogicalExprNode> {
        self.partition_expr.as_ref()
    }

    pub fn inherits_parent_data(&self) -> bool {
        self.payload.inherits_parent_data()
    }

    pub fn group_name(&self, child_index: usize) -> String {
        match self.key() {
            Some(key) => format!(
                "cartesian_subplot_{}_{}_{}",
                self.mark_index(),
                child_index,
                key
            ),
            None => format!("cartesian_subplot_{}_{}", self.mark_index(), child_index),
        }
    }
}

impl CompiledMarkCore for CompiledCartesianSubplot {
    fn state(&self) -> &CompiledMarkState {
        self.payload.compiled_state()
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        self.payload.compiled_state_mut()
    }

    fn data_context(&self) -> &CompiledDataContext {
        &self.payload.compiled_state().data
    }

    fn mark_type(&self) -> &str {
        "subplot"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: "x",
                required: true,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "y",
                required: true,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: CARTESIAN_SUBPLOT_PARTITION_CHANNEL,
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn radius_expression(
        &self,
        _dimension: &str,
        _resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        None
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianSubplot {
    async fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _context: &dyn MarkRuntimeContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Cartesian subplot marks require the top-level layout render dispatcher".to_string(),
        ))
    }
}
