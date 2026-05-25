//! Shared mark-channel preparation for measurement and rendering.
//!
//! Rendering and coordinate-system measurement sometimes need the same prepared
//! channel batches. Keeping this logic here avoids duplicating scale expression
//! handling between mark rendering and child-frame container measurement.

use std::{collections::HashMap, sync::Arc};

use avenger_common::types::ColorOrGradient;
use datafusion::{
    arrow::{
        array::Int32Array,
        compute::concat_batches,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrame,
    logical_expr::{Expr, lit, when},
    prelude::SessionContext,
};
use datafusion_proto::protobuf::LogicalPlanNode;

use avenger_chart_core::{EvaluationContext, color::parse_color_string, params_to_datafusion};

use crate::{
    channel::{
        resolution::resolve_all_channel_refs,
        value::{ChannelValue, ConditionalValue, strip_trailing_numbers},
    },
    error::AvengerChartError,
    marks::CompiledMark,
    render::RenderState,
    scales::{ConfiguredScaleDataFusionExt, ConfiguredScaleWithSpec},
    serialization::{LogicalExprNodeExt, LogicalPlanNodeExt},
};

/// Prepared data for mark evaluation.
pub(crate) struct PreparedMarkData {
    /// Array data batch (multiple rows), or None if all channels are scalar.
    pub(crate) data_batch: Option<RecordBatch>,
    /// Scalar data batch (single row) for channels that do not vary per mark.
    pub(crate) scalar_batch: RecordBatch,
    /// Render state with plot dimensions and configured scales.
    pub(crate) render_state: RenderState,
}

pub(crate) struct MarkDataRequest<'a> {
    pub(crate) mark: &'a dyn CompiledMark,
    pub(crate) plot_data: Option<&'a LogicalPlanNode>,
    pub(crate) provided_plot_df: Option<&'a DataFrame>,
    pub(crate) eval_ctx: &'a EvaluationContext,
    pub(crate) scales: &'a HashMap<String, ConfiguredScaleWithSpec>,
    pub(crate) plot_width: f32,
    pub(crate) plot_height: f32,
}

/// Apply a scale transformation to a channel expression.
pub(crate) fn apply_channel_scale(
    channel_name: &str,
    channel_value: &ChannelValue,
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    ctx: &SessionContext,
) -> Result<Expr, AvengerChartError> {
    match channel_value {
        ChannelValue::Value { expr } => expr.to_expr(ctx),
        ChannelValue::Conditional {
            conditions,
            otherwise,
            ..
        } => {
            let convert_color_literal = |expr: &Expr| -> Expr {
                if let Expr::Literal(scalar_value, _) = expr
                    && let ScalarValue::Utf8(Some(s)) = scalar_value
                    && let Some(color_or_gradient) = parse_color_string(s)
                    && let ColorOrGradient::Color(rgba) = color_or_gradient
                {
                    let values: Vec<ScalarValue> = rgba
                        .into_iter()
                        .map(|v| ScalarValue::Float32(Some(v)))
                        .collect();
                    let list_array = ScalarValue::new_list_nullable(&values, &DataType::Float32);
                    return lit(ScalarValue::List(list_array));
                }
                expr.clone()
            };

            let scale_key = strip_trailing_numbers(channel_name).to_string();
            let needs_color_conversion = matches!(channel_name, "fill" | "stroke" | "color");

            let apply_to_conditional =
                |cond_val: &ConditionalValue| -> Result<Expr, AvengerChartError> {
                    match cond_val {
                        ConditionalValue::Scaled { expr } => {
                            if let Some(scale) = scales.get(&scale_key) {
                                expr.to_expr(ctx).and_then(|e| scale.to_expr(e))
                            } else {
                                expr.to_expr(ctx)
                            }
                        }
                        ConditionalValue::Value { expr } => {
                            let expr_df = expr.to_expr(ctx)?;
                            if needs_color_conversion {
                                Ok(convert_color_literal(&expr_df))
                            } else {
                                Ok(expr_df)
                            }
                        }
                    }
                };

            let first_cond = &conditions[0];
            let first_value = apply_to_conditional(&first_cond.1)?;
            let first_cond_expr = first_cond.0.to_expr(ctx)?;
            let mut case_expr = when(first_cond_expr, first_value);

            for (condition, value) in &conditions[1..] {
                let scaled_value = apply_to_conditional(value)?;
                let condition_expr = condition.to_expr(ctx)?;
                case_expr = case_expr.when(condition_expr, scaled_value);
            }

            let otherwise_value = apply_to_conditional(otherwise)?;
            Ok(case_expr.otherwise(otherwise_value)?)
        }
        ChannelValue::Scaled {
            expr,
            scale_name,
            band,
            ..
        } => {
            let default_scale_name = strip_trailing_numbers(channel_name).to_string();
            let scale_key = scale_name.as_ref().unwrap_or(&default_scale_name);
            let scale = scales.get(scale_key).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Scale '{}' not found for channel '{}'",
                    scale_key, channel_name
                ))
            })?;

            let expr_df = expr.to_expr(ctx)?;
            if let Some(band) = band {
                scale.to_expr_with_band(expr_df.clone(), *band)
            } else {
                scale.to_expr(expr_df)
            }
        }
    }
}

/// Prepare data batches for a compiled mark.
pub(crate) async fn prepare_mark_data(
    request: MarkDataRequest<'_>,
) -> Result<Option<PreparedMarkData>, AvengerChartError> {
    let ctx = &*request.eval_ctx.session_context;
    let params = &request.eval_ctx.params;
    let mark = request.mark;

    let channels = resolve_all_channel_refs(mark.data_context().channels(), ctx)?;

    let references_columns = channels.values().any(|channel_value| match channel_value {
        ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => expr
            .to_expr(ctx)
            .map(|e| !e.column_refs().is_empty())
            .unwrap_or(false),
        ChannelValue::Conditional {
            conditions,
            otherwise,
            ..
        } => {
            conditions.iter().any(|(condition, value)| {
                let cond_has_refs = condition
                    .to_expr(ctx)
                    .map(|e| !e.column_refs().is_empty())
                    .unwrap_or(false);
                let value_has_refs = value
                    .expr(ctx)
                    .map(|e| !e.column_refs().is_empty())
                    .unwrap_or(false);
                cond_has_refs || value_has_refs
            }) || otherwise
                .expr(ctx)
                .map(|e| !e.column_refs().is_empty())
                .unwrap_or(false)
        }
    });

    let df_ref = if let Some(mark_df) = mark.data_context().dataframe_with_context(ctx) {
        Some(mark_df)
    } else if let Some(df_override) = request.provided_plot_df.cloned() {
        Some(df_override)
    } else if !references_columns {
        None
    } else if let Some(df) = request.plot_data.and_then(|node| {
        node.to_logical_plan(ctx)
            .ok()
            .map(|plan| DataFrame::new(ctx.state().clone(), plan))
    }) {
        Some(df)
    } else {
        return Err(AvengerChartError::InternalError(
            "Mark expressions reference columns but no data is available".to_string(),
        ));
    };

    let df = if let Some(df_ref) = df_ref {
        if let Some(sort_channel_name) = mark.sorting_channel() {
            if let Some(sort_channel) = channels.get(sort_channel_name) {
                let sort_expr =
                    apply_channel_scale(sort_channel_name, sort_channel, request.scales, ctx)?;
                Arc::new(df_ref.sort(vec![sort_expr.sort(true, false)])?)
            } else {
                Arc::new(df_ref)
            }
        } else {
            Arc::new(df_ref)
        }
    } else {
        let empty_df = ctx
            .sql("SELECT 1 as _dummy")
            .await
            .map_err(AvengerChartError::DataFusionError)?;
        Arc::new(empty_df)
    };

    let supported_channels = mark.supported_channels();
    let mut array_channels = Vec::new();
    let mut scalar_channels = Vec::new();
    let mut has_array_data = false;
    for channel_desc in &supported_channels {
        if let Some(channel_value) = channels.get(channel_desc.name) {
            let scaled_expr =
                apply_channel_scale(channel_desc.name, channel_value, request.scales, ctx)?;
            if channel_desc.allow_column_ref && scaled_expr.any_column_refs() {
                array_channels.push((channel_desc.name, scaled_expr));
                has_array_data = true;
            } else {
                scalar_channels.push((channel_desc.name, scaled_expr));
            }
        }
    }

    let data_batch = if mark.wants_full_data_batch() {
        let datafusion_params = params_to_datafusion(params);
        let batch = if let Some(param_values) = datafusion_params {
            (*df)
                .clone()
                .with_param_values(param_values)?
                .collect()
                .await?
        } else {
            (*df).clone().collect().await?
        };

        if batch.is_empty() {
            let arrow_schema = std::sync::Arc::new(df.schema().as_arrow().clone());
            Some(RecordBatch::new_empty(arrow_schema))
        } else {
            let schema = batch[0].schema();
            Some(concat_batches(&schema, &batch)?)
        }
    } else if has_array_data {
        let mut select_exprs = vec![];
        for (name, expr) in &array_channels {
            select_exprs.push(expr.clone().alias(*name));
        }
        let datafusion_params = params_to_datafusion(params);
        let batch = if let Some(param_values) = datafusion_params {
            (*df)
                .clone()
                .select(select_exprs)?
                .with_param_values(param_values)?
                .collect()
                .await?
        } else {
            (*df).clone().select(select_exprs)?.collect().await?
        };
        if batch.is_empty() {
            None
        } else {
            let schema = batch[0].schema();
            Some(concat_batches(&schema, &batch)?)
        }
    } else {
        None
    };

    let mut scalar_select_exprs = vec![];
    for (name, expr) in &scalar_channels {
        scalar_select_exprs.push(expr.clone().alias(*name));
    }
    let scalar_batch = if !scalar_select_exprs.is_empty() {
        let datafusion_params = params_to_datafusion(params);
        let batch = if let Some(param_values) = datafusion_params {
            (*df)
                .clone()
                .select(scalar_select_exprs)?
                .with_param_values(param_values)?
                .collect()
                .await?
        } else {
            (*df).clone().select(scalar_select_exprs)?.collect().await?
        };
        if batch.is_empty() {
            return Ok(None);
        } else {
            let schema = batch[0].schema();
            concat_batches(&schema, &batch)?
        }
    } else {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "_dummy",
                DataType::Int32,
                false,
            )])),
            vec![Arc::new(Int32Array::from(vec![0]))],
        )?
    };

    Ok(Some(PreparedMarkData {
        data_batch,
        scalar_batch,
        render_state: RenderState::new(
            request.plot_width,
            request.plot_height,
            request.scales.clone(),
        ),
    }))
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use avenger_scales::scales::{ConfiguredScale, ScaleConfig};
    use datafusion::{
        arrow::{
            array::Float64Array,
            compute::cast,
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        },
        logical_expr::{Expr, col},
        prelude::SessionContext,
    };
    use datafusion_proto::protobuf::{LogicalExprNode, LogicalPlanNode};
    use indexmap::IndexMap;

    use super::*;
    use crate::{
        cartesian::{Cartesian, CartesianSymbolPositionChannels},
        concat::HConcat,
        error::AvengerChartError,
        marks::{ChannelValue, Mark, Subplot, symbol::Symbol},
        plot::Plot,
        scales::{Linear, Scale, ScaleRangeBinding, ScaleSpec},
        serialization::{LogicalExprNodeExt, LogicalPlanNodeExt},
        theme::Theme,
        zerod::ZeroDCoord,
    };
    use avenger_chart_core::EvaluationContext;

    fn eval_context(session: Arc<SessionContext>) -> EvaluationContext {
        EvaluationContext::new(Arc::new(Theme::light()), session, IndexMap::new())
    }

    fn xy_dataframe(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 5.0, 10.0])),
                Arc::new(Float64Array::from(vec![10.0, 5.0, 0.0])),
            ],
        )
        .expect("test batch");
        ctx.read_batch(batch).expect("test dataframe")
    }

    fn plot_data_node(
        df: &datafusion::dataframe::DataFrame,
    ) -> Result<LogicalPlanNode, AvengerChartError> {
        LogicalPlanNode::from_logical_plan(df.logical_plan())
    }

    fn value_channel(expr: Expr) -> ChannelValue {
        ChannelValue::Value {
            expr: LogicalExprNode::from_expr(expr).expect("serialize test expression"),
        }
    }

    fn linear_scale() -> ConfiguredScaleWithSpec {
        let scale = Scale::<Linear>::new().into_auto();
        let configured = ConfiguredScale {
            scale_impl: Linear.create_impl(),
            config: ScaleConfig::empty(),
        }
        .with_domain_interval((0.0, 10.0))
        .with_range_interval((0.0, 100.0));
        ConfiguredScaleWithSpec::with_range_binding(
            scale,
            configured,
            ScaleRangeBinding::Independent,
        )
    }

    fn values_as_f64(batch: &RecordBatch, column_name: &str) -> Vec<f64> {
        let column = batch
            .column_by_name(column_name)
            .unwrap_or_else(|| panic!("missing column {column_name}"));
        let casted = cast(column, &DataType::Float64).expect("cast test values");
        let values = casted
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("float64 values");
        (0..values.len()).map(|idx| values.value(idx)).collect()
    }

    #[tokio::test]
    async fn prepare_mark_data_applies_scaled_position_channels() -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = xy_dataframe(&session);
        let plot_node = plot_data_node(&df)?;
        let mark = Symbol::<Cartesian>::new().x(col("x")).y(col("y"));
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session);
        let scales = HashMap::from([
            ("x".to_string(), linear_scale()),
            ("y".to_string(), linear_scale()),
        ]);

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            plot_data: Some(&plot_node),
            provided_plot_df: None,
            eval_ctx: &eval_ctx,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
        })
        .await?
        .expect("prepared data");

        let data_batch = prepared.data_batch.expect("array data");
        assert_eq!(values_as_f64(&data_batch, "x"), vec![0.0, 50.0, 100.0]);
        assert_eq!(values_as_f64(&data_batch, "y"), vec![100.0, 50.0, 0.0]);
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_uses_inherited_plot_data() -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = xy_dataframe(&session);
        let plot_node = plot_data_node(&df)?;
        let mark = Symbol::<Cartesian>::new()
            .with_channel_value("x", value_channel(col("x")))
            .y(5.0);
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session);
        let scales = HashMap::new();

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            plot_data: Some(&plot_node),
            provided_plot_df: None,
            eval_ctx: &eval_ctx,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
        })
        .await?
        .expect("prepared data");

        let data_batch = prepared.data_batch.expect("inherited array data");
        assert_eq!(values_as_f64(&data_batch, "x"), vec![0.0, 5.0, 10.0]);
        assert!(prepared.scalar_batch.column_by_name("y").is_some());
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_handles_scalar_only_marks() -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let mark = Symbol::<Cartesian>::new().x(2.0).y(3.0);
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session);
        let scales = HashMap::new();

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            plot_data: None,
            provided_plot_df: None,
            eval_ctx: &eval_ctx,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
        })
        .await?
        .expect("prepared data");

        assert!(prepared.data_batch.is_none());
        assert_eq!(prepared.scalar_batch.num_rows(), 1);
        assert_eq!(values_as_f64(&prepared.scalar_batch, "x"), vec![2.0]);
        assert_eq!(values_as_f64(&prepared.scalar_batch, "y"), vec![3.0]);
        Ok(())
    }

    #[tokio::test]
    async fn prepare_mark_data_preserves_full_data_for_container_marks()
    -> Result<(), AvengerChartError> {
        let session = Arc::new(SessionContext::new());
        let df = xy_dataframe(&session);
        let mark = Subplot::<HConcat>::new(Plot::<ZeroDCoord>::new());
        let compiled_mark = mark.compile_untransformed(&session).await?;
        let eval_ctx = eval_context(session);
        let scales = HashMap::new();

        let prepared = prepare_mark_data(MarkDataRequest {
            mark: compiled_mark.as_ref(),
            plot_data: None,
            provided_plot_df: Some(&df),
            eval_ctx: &eval_ctx,
            scales: &scales,
            plot_width: 100.0,
            plot_height: 100.0,
        })
        .await?
        .expect("prepared data");

        let data_batch = prepared.data_batch.expect("full data batch");
        assert_eq!(data_batch.num_rows(), 3);
        assert!(data_batch.column_by_name("x").is_some());
        assert!(data_batch.column_by_name("y").is_some());
        assert_eq!(prepared.scalar_batch.num_rows(), 1);
        Ok(())
    }
}
