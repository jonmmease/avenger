//! Facade compatibility for scale building owned by `avenger-chart-scales`.

use std::{collections::HashMap, sync::Arc};

use datafusion::{
    arrow::datatypes::DataType as ArrowDataType, common::ScalarValue, dataframe::DataFrame,
};
use indexmap::IndexMap;

use avenger_chart_core::{
    AvengerChartError, ChannelValue, CompiledMark, CompiledViewScope, CompiledViewSpec,
    EvaluationContext as CoreEvaluationContext, MarkScaleDomainSource, ResolvedDomain, ScaleRange,
    Theme, resolve_all_channel_refs,
};
use avenger_chart_scales::{ConfiguredScaleWithSpec, PreparedScaleMark, ScaleBuilder};
use avenger_scales::scales::ScaleImpl;

use crate::{facet::evaluated_facet_tree::EvaluatedFacetTree, serialization::LogicalExprNodeExt};

use super::{
    CompiledPlot, LogicalMarkDataRequest, MarkDataRequest,
    mark_data_runtime::PreparedLogicalMarkData,
    mark_data_runtime::{
        ViewMaterializationHandling, eval_ctx_with_view_params, prepare_view_logical_mark_data,
    },
    prepare_logical_mark_data,
    session::{ScaleDomainCacheScope, scale_domain_cache_key_for_parts_with_scope},
};

fn view_domain_channel_value(
    channel: &str,
    domain_expr: datafusion::logical_expr::Expr,
    prepared: &PreparedLogicalMarkData,
) -> Result<ChannelValue, AvengerChartError> {
    let domain_node = datafusion_proto::protobuf::LogicalExprNode::from_expr(domain_expr.clone())?;
    Ok(prepared
        .channels
        .get(channel)
        .cloned()
        .map(|value| value.with_expr(domain_node.clone()))
        .unwrap_or_else(|| ChannelValue::from(domain_expr)))
}

fn view_domain_sources(
    view_scope: &CompiledViewScope,
    prepared: &PreparedLogicalMarkData,
    base_prepared: &PreparedLogicalMarkData,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<Vec<MarkScaleDomainSource>, AvengerChartError> {
    let Some(dataframe) = base_prepared.domain_dataframe.clone() else {
        return Ok(Vec::new());
    };

    match &view_scope.spec {
        CompiledViewSpec::Cartesian(spec) => {
            let x_expr = spec.x_domain.to_expr(ctx)?;
            let y_expr = spec.y_domain.to_expr(ctx)?;
            Ok(vec![
                MarkScaleDomainSource {
                    channel: "x".to_string(),
                    channel_value: view_domain_channel_value("x", x_expr.clone(), prepared)?,
                    dataframe: dataframe.clone(),
                    exprs: vec![x_expr],
                },
                MarkScaleDomainSource {
                    channel: "y".to_string(),
                    channel_value: view_domain_channel_value("y", y_expr.clone(), prepared)?,
                    dataframe,
                    exprs: vec![y_expr],
                },
            ])
        }
    }
}

fn remove_view_positional_domain_channels(domain_channels: &mut IndexMap<String, ChannelValue>) {
    for channel in ["x", "x2", "y", "y2"] {
        domain_channels.shift_remove(channel);
    }
}

async fn prepare_scale_mark_for_plot(
    plot: &CompiledPlot,
    mark: &Arc<dyn CompiledMark>,
    df_override: Option<&DataFrame>,
    eval_ctx: &crate::render::EvaluationContext,
    facet_path: &[ScalarValue],
) -> Result<PreparedScaleMark, AvengerChartError> {
    let prepared_base = match plot.data_group_index_for_mark(mark.state().mark_index()) {
        Some(group_index) => Some(
            Box::pin(plot.prepare_mark_group_base_data(
                group_index,
                eval_ctx,
                df_override,
                facet_path,
            ))
            .await?,
        ),
        None => None,
    };
    let prepared = Box::pin(prepare_logical_mark_data(LogicalMarkDataRequest {
        mark: mark.as_ref(),
        plot_data: plot.data.as_ref(),
        provided_plot_df: df_override,
        facet_data_scope: Some(crate::facet::data_scope::FacetDataScopeContext::new(
            eval_ctx.facet_tree.as_ref(),
            eval_ctx.facet_data_root(),
            facet_path,
        )),
        prepared_base: prepared_base.as_deref(),
        eval_ctx,
    }))
    .await?;
    let mut channels = prepared.channels.clone();
    let mut domain_channels = prepared.domain_channels.clone();
    let mut extra_domain_sources = Vec::new();
    if let Some(view_scope) = mark.state().view.as_ref() {
        let view_channels = resolve_all_channel_refs(
            view_scope.data.channels(),
            eval_ctx.session_context.as_ref(),
        )?;
        channels.extend(view_channels);
        remove_view_positional_domain_channels(&mut domain_channels);
        extra_domain_sources = view_domain_sources(
            view_scope,
            &PreparedLogicalMarkData {
                dataframe: prepared.dataframe.clone(),
                channels: channels.clone(),
                domain_dataframe: prepared.domain_dataframe.clone(),
                domain_channels: domain_channels.clone(),
                derived_scalars: prepared.derived_scalars.clone(),
            },
            &prepared,
            eval_ctx.session_context.as_ref(),
        )?;
    }

    Ok(PreparedScaleMark::new_with_domain_source(
        mark.clone(),
        prepared.dataframe,
        channels,
        prepared.domain_dataframe,
        domain_channels,
        prepared.derived_scalars,
    )
    .with_extra_domain_sources(extra_domain_sources)
    .with_scale_inference_hints(plot.scale_inference_hints_for_mark(mark.state().mark_index())?))
}

async fn prepare_view_materialized_scale_mark_for_plot(
    plot: &CompiledPlot,
    mark: &Arc<dyn CompiledMark>,
    df_override: Option<&DataFrame>,
    eval_ctx: &crate::render::EvaluationContext,
    base_scales: &HashMap<String, ConfiguredScaleWithSpec>,
    plot_width: f32,
    plot_height: f32,
    facet_path: &[ScalarValue],
) -> Result<PreparedScaleMark, AvengerChartError> {
    let prepared_base = match plot.data_group_index_for_mark(mark.state().mark_index()) {
        Some(group_index) => Some(
            Box::pin(plot.prepare_mark_group_base_data(
                group_index,
                eval_ctx,
                df_override,
                facet_path,
            ))
            .await?,
        ),
        None => None,
    };
    let base_prepared = Box::pin(prepare_logical_mark_data(LogicalMarkDataRequest {
        mark: mark.as_ref(),
        plot_data: plot.data.as_ref(),
        provided_plot_df: df_override,
        facet_data_scope: Some(crate::facet::data_scope::FacetDataScopeContext::new(
            eval_ctx.facet_tree.as_ref(),
            eval_ctx.facet_data_root(),
            facet_path,
        )),
        prepared_base: prepared_base.as_deref(),
        eval_ctx,
    }))
    .await?;

    let (prepared, domain_channels, extra_domain_sources) = if let Some(view_scope) =
        mark.state().view.as_ref()
    {
        let view_eval_ctx =
            eval_ctx_with_view_params(eval_ctx, view_scope, base_scales, plot_width, plot_height)?;
        let request = MarkDataRequest {
            mark: mark.as_ref(),
            coord_transform: Some(plot.coord_transform.as_ref()),
            plot_data: plot.data.as_ref(),
            provided_plot_df: df_override,
            facet_data_scope: Some(crate::facet::data_scope::FacetDataScopeContext::new(
                eval_ctx.facet_tree.as_ref(),
                eval_ctx.facet_data_root(),
                facet_path,
            )),
            prepared_logical: Some(&base_prepared),
            prepared_base: prepared_base.as_deref(),
            eval_ctx,
            evaluation_metrics: eval_ctx.evaluation_metrics.clone(),
            scales: base_scales,
            plot_width,
            plot_height,
        };
        let view_prepared = Box::pin(prepare_view_logical_mark_data(
            mark.as_ref(),
            view_scope,
            &base_prepared,
            &request,
            &view_eval_ctx,
            ViewMaterializationHandling::ScaleInferenceReadOnly,
        ))
        .await?;
        let mut domain_channels = view_prepared.domain_channels.clone();
        remove_view_positional_domain_channels(&mut domain_channels);
        let extra_domain_sources = view_domain_sources(
            view_scope,
            &view_prepared,
            &base_prepared,
            eval_ctx.session_context.as_ref(),
        )?;
        (view_prepared, domain_channels, extra_domain_sources)
    } else {
        (
            base_prepared.clone(),
            base_prepared.domain_channels.clone(),
            Vec::new(),
        )
    };

    Ok(PreparedScaleMark::new_with_domain_source(
        mark.clone(),
        prepared.dataframe,
        prepared.channels,
        prepared.domain_dataframe,
        domain_channels,
        prepared.derived_scalars,
    )
    .with_extra_domain_sources(extra_domain_sources)
    .with_scale_inference_hints(plot.scale_inference_hints_for_mark(mark.state().mark_index())?))
}

pub(crate) async fn build_scale_builder_from_compiled_plot(
    plot: &CompiledPlot,
    df_override: Option<DataFrame>,
    eval_ctx: &CoreEvaluationContext,
    theme: &Theme,
) -> Result<ScaleBuilder, AvengerChartError> {
    let mut render_eval_ctx = crate::render::EvaluationContext::new(
        eval_ctx.theme().clone(),
        eval_ctx.session_context().clone(),
        eval_ctx.params().clone(),
        Arc::new(EvaluatedFacetTree::empty()),
    );
    render_eval_ctx.core = eval_ctx.clone();
    build_scale_builder_from_compiled_plot_with_render_context(
        plot,
        df_override,
        &render_eval_ctx,
        theme,
    )
    .await
}

pub(crate) async fn build_scale_builder_from_compiled_plot_with_render_context(
    plot: &CompiledPlot,
    df_override: Option<DataFrame>,
    eval_ctx: &crate::render::EvaluationContext,
    theme: &Theme,
) -> Result<ScaleBuilder, AvengerChartError> {
    let mut prepared_marks = Vec::with_capacity(plot.marks.len());
    for mark in &plot.marks {
        prepared_marks.push(
            Box::pin(prepare_scale_mark_for_plot(
                plot,
                mark,
                df_override.as_ref(),
                eval_ctx,
                &[],
            ))
            .await?,
        );
    }

    Box::pin(
        avenger_chart_scales::build_scale_builder_from_prepared_marks(
            &prepared_marks,
            &plot.scale_specs,
            plot.coord_transform.as_ref(),
            eval_ctx,
            theme,
        ),
    )
    .await
}

pub(crate) async fn build_scale_builder_from_compiled_plot_with_view_materialized_data(
    plot: &CompiledPlot,
    df_override: Option<DataFrame>,
    eval_ctx: &crate::render::EvaluationContext,
    base_scales: &HashMap<String, ConfiguredScaleWithSpec>,
    plot_width: f32,
    plot_height: f32,
    facet_path: &[ScalarValue],
    theme: &Theme,
) -> Result<ScaleBuilder, AvengerChartError> {
    let mut prepared_marks = Vec::with_capacity(plot.marks.len());
    for mark in &plot.marks {
        prepared_marks.push(
            Box::pin(prepare_view_materialized_scale_mark_for_plot(
                plot,
                mark,
                df_override.as_ref(),
                eval_ctx,
                base_scales,
                plot_width,
                plot_height,
                facet_path,
            ))
            .await?,
        );
    }

    Box::pin(
        avenger_chart_scales::build_scale_builder_from_prepared_marks(
            &prepared_marks,
            &plot.scale_specs,
            plot.coord_transform.as_ref(),
            eval_ctx,
            theme,
        ),
    )
    .await
}

pub(crate) async fn build_scale_builder_from_compiled_plot_with_facet_scope(
    plot: &CompiledPlot,
    df_override: Option<DataFrame>,
    eval_ctx: &crate::render::EvaluationContext,
    facet_path: &[ScalarValue],
    theme: &Theme,
) -> Result<ScaleBuilder, AvengerChartError> {
    let cache_lookup = eval_ctx.scale_domain_cache().map(|cache| {
        let scope = ScaleDomainCacheScope::FacetPath(
            facet_path
                .iter()
                .map(|value| format!("{value:?}"))
                .collect(),
        );
        let key = scale_domain_cache_key_for_parts_with_scope(
            &plot.marks,
            &plot.scale_specs,
            &plot.data,
            df_override.as_ref(),
            eval_ctx.session_context.as_ref(),
            eval_ctx.params(),
            scope,
        );
        (cache.clone(), key)
    });
    if let Some((cache, key)) = &cache_lookup {
        let cached_builder = {
            cache
                .lock()
                .expect("scale-domain cache lock poisoned")
                .get(key)
        };
        if let Some(builder) = cached_builder {
            eval_ctx.record_scale_domain_cache_hit();
            return Ok((*builder).clone());
        }
        eval_ctx.record_scale_domain_cache_miss();
    }
    eval_ctx.record_scale_builder_build();
    let mut prepared_marks = Vec::with_capacity(plot.marks.len());
    for mark in &plot.marks {
        prepared_marks.push(
            Box::pin(prepare_scale_mark_for_plot(
                plot,
                mark,
                df_override.as_ref(),
                eval_ctx,
                facet_path,
            ))
            .await?,
        );
    }

    let scale_builder = Box::pin(
        avenger_chart_scales::build_scale_builder_from_prepared_marks(
            &prepared_marks,
            &plot.scale_specs,
            plot.coord_transform.as_ref(),
            eval_ctx,
            theme,
        ),
    )
    .await?;
    if let Some((cache, key)) = cache_lookup {
        cache
            .lock()
            .expect("scale-domain cache lock poisoned")
            .insert(key, scale_builder.clone());
    }
    Ok(scale_builder)
}

pub(super) fn default_range_for_compiled_marks<'a>(
    compiled_marks: &'a [Arc<dyn CompiledMark>],
) -> impl Fn(
    &str,
    &dyn ScaleImpl,
    &ResolvedDomain,
    &ArrowDataType,
    &Theme,
    &IndexMap<String, ScalarValue>,
) -> Option<ScaleRange>
+ 'a {
    move |channel_name, scale_impl, resolved_domain, data_type, theme, params| {
        for mark in compiled_marks {
            if let Some(mark_range) = mark.default_channel_range(
                channel_name,
                scale_impl,
                resolved_domain,
                data_type,
                theme,
                params,
            ) {
                return Some(mark_range);
            }
        }

        None
    }
}

impl CompiledPlot {
    /// Check if a data type is numeric
    pub(super) fn is_numeric_type(dtype: &ArrowDataType) -> bool {
        match dtype {
            // Standard numeric types
            ArrowDataType::Int8
            | ArrowDataType::Int16
            | ArrowDataType::Int32
            | ArrowDataType::Int64
            | ArrowDataType::UInt8
            | ArrowDataType::UInt16
            | ArrowDataType::UInt32
            | ArrowDataType::UInt64
            | ArrowDataType::Float16
            | ArrowDataType::Float32
            | ArrowDataType::Float64 => true,
            // Dictionary types are allowed if their value type is numeric
            // (This happens when categorical data goes through an ordinal scale)
            ArrowDataType::Dictionary(_, value_type) => Self::is_numeric_type(value_type),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;
    use crate::render::RenderContext;
    use avenger_chart_core::{EmptyCoordMeasurement, channel::strip_trailing_numbers};
    use avenger_chart_scales::ConfiguredScaleWithSpec;
    use datafusion::arrow::array::{Float64Array, TimestampMillisecondArray};
    use datafusion::arrow::datatypes::{DataType, Field, Schema, TimeUnit as ArrowTimeUnit};
    use datafusion::arrow::record_batch::RecordBatch;
    use datafusion::prelude::SessionContext;
    use indexmap::IndexMap;
    use std::sync::Arc;

    async fn two_phase_build_scales(
        compiled: &super::CompiledPlot,
        width: f32,
        height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<std::collections::HashMap<String, ConfiguredScaleWithSpec>, super::AvengerChartError>
    {
        use std::collections::HashMap;

        let builder = super::build_scale_builder_from_compiled_plot(
            compiled,
            None,
            &avenger_chart_core::EvaluationContext::new(
                compiled.get_theme(),
                Arc::new(ctx.clone()),
                params.clone(),
            )
            .with_time_context(compiled.time_context.clone())
            .with_formatting_context(compiled.formatting_context.clone()),
            compiled.get_theme().as_ref(),
        )
        .await?;

        let mut coord_system_range_bindings = HashMap::new();
        for ch in builder.channel_builders().keys() {
            let base = compiled
                .scale_to_coord_channel
                .get(ch)
                .map(String::as_str)
                .unwrap_or_else(|| strip_trailing_numbers(ch));
            if let Some(binding) = compiled.coord_transform.default_range_binding(base) {
                coord_system_range_bindings.insert(ch.clone(), binding);
            }
        }

        let theme = compiled.get_theme();
        let default_range_resolver = super::default_range_for_compiled_marks(&compiled.marks);
        builder
            .build_scales(
                width,
                height,
                &coord_system_range_bindings,
                &compiled.scale_specs,
                &default_range_resolver,
                theme.as_ref(),
                ctx,
                params,
            )
            .await
    }

    #[tokio::test]
    async fn view_xy_domains_come_from_view_spec_not_view_render_channels() {
        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES \
                 (0.0, 0.0, 100.0, 500.0), \
                 (10.0, 5.0, 200.0, 600.0) \
                 ) AS t(x, y, zoom_x, zoom_y)",
            )
            .await
            .expect("view scale data");
        let compiled = Plot::<Cartesian>::new()
            .data(df)
            .mark(
                Line::new().view(
                    View::cartesian()
                        .id("viewport")
                        .x_domain(col("x"))
                        .y_domain(col("y")),
                    |mark, _view| {
                        mark.x_with(col("zoom_x"), |x| {
                            x.scale_with::<Linear>(|s| s.nice(false).zero(false))
                        })
                        .y_with(col("zoom_y"), |y| {
                            y.scale_with::<Linear>(|s| s.nice(false).zero(false))
                        })
                    },
                ),
            )
            .compile(&ctx)
            .await
            .expect("compile view plot");

        let scales = two_phase_build_scales(&compiled, 400.0, 300.0, &ctx, &IndexMap::new())
            .await
            .expect("build view scales");

        let x_domain = scales
            .get("x")
            .expect("x scale")
            .configured()
            .numeric_interval_domain()
            .unwrap();
        let y_domain = scales
            .get("y")
            .expect("y scale")
            .configured()
            .numeric_interval_domain()
            .unwrap();
        assert!(
            x_domain.0 < 1.0 && x_domain.1 < 20.0,
            "x domain should come from view x_domain, not zoom_x render channel: {x_domain:?}"
        );
        assert!(
            y_domain.0 < 1.0 && y_domain.1 < 10.0,
            "y domain should come from view y_domain, not zoom_y render channel: {y_domain:?}"
        );
    }

    #[tokio::test]
    async fn view_xy_domains_ignore_view_derivative_render_channels() {
        let ctx = SessionContext::new();
        let df = ctx
            .sql("SELECT * FROM (VALUES (0.0, 0.0), (10.0, 8.0)) AS t(x, y)")
            .await
            .expect("view scale data");
        let compiled = Plot::<Cartesian>::new()
            .data(df)
            .mark(
                Rect::new().view(
                    View::cartesian()
                        .id("domain_box")
                        .x_domain(col("x"))
                        .y_domain(col("y")),
                    |mark, view| {
                        mark.x_with(view.x().domain_start(), |x| {
                            x.scale_with::<Linear>(|s| s.nice(false).zero(false))
                        })
                        .x2(view.x().domain_end())
                        .y_with(view.y().domain_start(), |y| {
                            y.scale_with::<Linear>(|s| s.nice(false).zero(false))
                        })
                        .y2(view.y().domain_end())
                    },
                ),
            )
            .compile(&ctx)
            .await
            .expect("compile view rect plot");

        let scales = two_phase_build_scales(&compiled, 400.0, 300.0, &ctx, &IndexMap::new())
            .await
            .expect("build view scales");

        let x_domain = scales
            .get("x")
            .expect("x scale")
            .configured()
            .numeric_interval_domain()
            .unwrap();
        let y_domain = scales
            .get("y")
            .expect("y scale")
            .configured()
            .numeric_interval_domain()
            .unwrap();
        assert_eq!(x_domain, (0.0, 10.0));
        assert_eq!(y_domain, (0.0, 8.0));
    }

    #[tokio::test]
    async fn mark_owned_parallel_dimension_contributes_scale_domain() {
        let ctx = SessionContext::new();
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Float64,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Float64Array::from(vec![2.0, 4.0, 8.0]))],
        )
        .unwrap();
        let df = ctx.read_batch(batch).unwrap();

        let compiled = Plot::<Parallel>::new()
            .data(df)
            .mark(ParallelLine::new().dimension("value_axis", col("value")))
            .compile(&ctx)
            .await
            .expect("compile parallel plot");

        let builder = super::build_scale_builder_from_compiled_plot(
            &compiled,
            None,
            &avenger_chart_core::EvaluationContext::new(
                compiled.get_theme(),
                Arc::new(ctx.clone()),
                IndexMap::new(),
            )
            .with_time_context(compiled.time_context.clone())
            .with_formatting_context(compiled.formatting_context.clone()),
            compiled.get_theme().as_ref(),
        )
        .await
        .expect("build scales");

        assert!(
            builder.channel_builders().contains_key("value_axis"),
            "mark-owned parallel dimension should create a scale-domain builder"
        );
    }

    #[tokio::test]
    async fn parallel_dimension_scales_infer_linear_point_and_vertical_range() {
        let ctx = SessionContext::new();
        let df = ctx
            .sql("SELECT * FROM (VALUES (21.0, 'usa'), (28.0, 'japan')) AS t(mpg, origin)")
            .await
            .expect("parallel data");
        let compiled = Plot::<Parallel>::new()
            .data(df)
            .mark(
                ParallelLine::new()
                    .dimension("mpg", col("mpg"))
                    .dimension("origin", col("origin")),
            )
            .compile(&ctx)
            .await
            .expect("compile parallel plot");

        let scales = two_phase_build_scales(&compiled, 400.0, 300.0, &ctx, &IndexMap::new())
            .await
            .expect("build parallel scales");

        let mpg = scales.get("mpg").expect("mpg scale").configured();
        assert_eq!(mpg.scale_impl.scale_type(), "linear");
        assert_eq!(mpg.numeric_interval_range().unwrap(), (300.0, 0.0));
        assert!(
            mpg.config.options.contains_key("nice"),
            "parallel numeric dimensions should inherit nice=true by default"
        );
        assert!(
            mpg.config.options.contains_key("round"),
            "parallel numeric dimensions should inherit round=true by default"
        );
        assert!(
            !mpg.config.options.contains_key("zero"),
            "parallel dimensions should not force a zero baseline by default"
        );

        let origin = scales.get("origin").expect("origin scale").configured();
        assert_eq!(origin.scale_impl.scale_type(), "point");
        assert_eq!(origin.numeric_interval_range().unwrap(), (300.0, 0.0));
    }

    #[tokio::test]
    async fn parallel_timestamp_dimension_infers_time_scale() {
        let ctx = SessionContext::new();
        let schema = Arc::new(Schema::new(vec![Field::new(
            "observed_at",
            DataType::Timestamp(ArrowTimeUnit::Millisecond, None),
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(TimestampMillisecondArray::from(vec![
                1_704_067_200_000,
                1_704_153_600_000,
            ]))],
        )
        .expect("timestamp batch");
        let df = ctx.read_batch(batch).expect("timestamp dataframe");
        let compiled = Plot::<Parallel>::new()
            .data(df)
            .mark(ParallelLine::new().dimension("observed_at", col("observed_at")))
            .compile(&ctx)
            .await
            .expect("compile parallel timestamp plot");

        let scales = two_phase_build_scales(&compiled, 400.0, 300.0, &ctx, &IndexMap::new())
            .await
            .expect("build parallel timestamp scales");

        let observed_at = scales
            .get("observed_at")
            .expect("observed_at scale")
            .configured();
        assert_eq!(observed_at.scale_impl.scale_type(), "time");
        assert_eq!(observed_at.numeric_interval_range().unwrap(), (300.0, 0.0));
    }

    #[tokio::test]
    async fn symbol_constant_size_expands_domain() {
        let ctx = SessionContext::new();

        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));

        let x_values = Float64Array::from(vec![0.0, 2.0, 10.0]);
        let y_values = Float64Array::from(vec![1.0, 3.0, 5.0]);

        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)]).unwrap();
        let df = ctx.read_batch(batch).unwrap();

        let plot = Plot::<Cartesian>::new().data(df).mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(400.0)
                .fill("#4682b4"),
        );

        let compiled = plot.compile(&ctx).await.expect("compile plot");

        let params = IndexMap::new();
        let positional_scales = two_phase_build_scales(&compiled, 400.0, 300.0, &ctx, &params)
            .await
            .expect("build scales two-phase");

        let (domain_min, domain_max) = positional_scales
            .get("x")
            .expect("x scale")
            .configured()
            .numeric_interval_domain()
            .expect("numeric domain");

        assert!(
            domain_min < 0.0,
            "domain_min should be less than data minimum (0.0), got {}",
            domain_min
        );
        assert!(
            domain_max > 10.0,
            "domain_max should be greater than data maximum (10.0), got {}",
            domain_max
        );

        let (y_min, y_max) = positional_scales
            .get("y")
            .expect("y scale")
            .configured()
            .numeric_interval_domain()
            .expect("numeric domain");

        assert!(
            y_min < 1.0,
            "y_min should be less than data minimum (1.0), got {}",
            y_min
        );
        assert!(
            y_max > 5.0,
            "y_max should be greater than data maximum (5.0), got {}",
            y_max
        );
    }

    #[tokio::test]
    async fn plot_time_context_sets_temporal_scale_timezone_default() {
        let ctx = SessionContext::new();

        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "timestamp",
                DataType::Timestamp(ArrowTimeUnit::Millisecond, None),
                false,
            ),
            Field::new("value", DataType::Float64, false),
        ]));

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(TimestampMillisecondArray::from(vec![
                    1_704_067_200_000,
                    1_704_153_600_000,
                ])),
                Arc::new(Float64Array::from(vec![1.0, 2.0])),
            ],
        )
        .unwrap();
        let df = ctx.read_batch(batch).unwrap();

        let plot = Plot::<Cartesian>::new()
            .time_context(TimeContext::new().timezone("America/New_York"))
            .data(df)
            .mark(Symbol::new().x(col("timestamp")).y(col("value")));

        let compiled = plot.compile(&ctx).await.expect("compile plot");
        let scales = two_phase_build_scales(&compiled, 400.0, 300.0, &ctx, &IndexMap::new())
            .await
            .expect("build scales two-phase");

        let x_scale = scales.get("x").expect("x scale").configured();
        assert_eq!(
            x_scale.option_string("timezone", "missing"),
            "America/New_York"
        );
    }

    #[tokio::test]
    async fn legend_titles_radius_padding_matches_data() {
        use avenger_chart_core::ScalarValueHelpers;
        use datafusion::arrow::array::Float64Array;
        use datafusion::prelude::*;

        let ctx = SessionContext::new();
        let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
        let df = ctx
            .read_parquet(iris_path, ParquetReadOptions::default())
            .await
            .expect("load iris dataset");

        let projected = df
            .clone()
            .select(vec![col("sepal_length"), col("sepal_width")])
            .expect("project columns")
            .collect()
            .await
            .expect("collect samples");

        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;

        for batch in &projected {
            let x_array = batch
                .column(0)
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("x array");
            let y_array = batch
                .column(1)
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("y array");

            for value in x_array.iter().flatten() {
                min_x = min_x.min(value);
                max_x = max_x.max(value);
            }
            for value in y_array.iter().flatten() {
                min_y = min_y.min(value);
                max_y = max_y.max(value);
            }
        }

        let min_x = min_x as f32;
        let max_x = max_x as f32;
        let min_y = min_y as f32;
        let max_y = max_y as f32;

        let plot_df = df;

        let plot = Plot::<Cartesian>::new()
            .data(plot_df)
            .title("Custom Legend Titles")
            .mark(
                Symbol::new()
                    .x(col("sepal_length"))
                    .y(col("sepal_width"))
                    .size(150.0)
                    .fill_with(col("species"), |c| {
                        c.scale_with::<Ordinal>(|s| s)
                            .legend(|l| l.title("Iris Species"))
                    }),
            );

        let compiled = plot.compile(&ctx).await.expect("compile plot");

        let params = IndexMap::new();
        let final_scales = two_phase_build_scales(&compiled, 220.0, 300.0, &ctx, &params)
            .await
            .expect("build scales two-phase");

        let (x_domain_min, x_domain_max) = final_scales
            .get("x")
            .expect("x scale")
            .configured()
            .numeric_interval_domain()
            .expect("numeric domain");
        let y_scale = final_scales.get("y").expect("y scale").configured();
        let (y_domain_min, y_domain_max) = y_scale.numeric_interval_domain().expect("numeric");

        assert!(
            x_domain_min < min_x,
            "x domain minimum ({x_domain_min}) should be less than data minimum ({min_x})"
        );
        assert!(
            x_domain_max > max_x,
            "x domain maximum ({x_domain_max}) should be greater than data maximum ({max_x})"
        );
        assert!(
            y_domain_min < min_y,
            "y domain minimum ({y_domain_min}) should be less than data minimum ({min_y})"
        );
        assert!(
            y_domain_max > max_y,
            "y domain maximum ({y_domain_max}) should be greater than data maximum ({max_y})"
        );

        let theme = compiled.get_theme();
        let eval_ctx = crate::render::EvaluationContext::new(
            theme.clone(),
            Arc::new(ctx.clone()),
            IndexMap::new(),
            Arc::new(crate::facet::evaluated_facet_tree::EvaluatedFacetTree::empty()),
        );
        let render_state =
            crate::render::RenderState::new(220.0, 300.0, std::collections::HashMap::new());
        let final_context =
            RenderContext::new(&eval_ctx, &render_state, &[], &EmptyCoordMeasurement);
        let final_mark_context = final_context.core_view();
        let first_mark = compiled.marks().first().expect("compiled mark");
        let stroke_width = first_mark
            .default_channel_value("stroke_width", &final_mark_context)
            .and_then(|scalar| scalar.as_f32().ok())
            .unwrap_or(1.0);
        let radius_px = 150.0_f32.sqrt() * 0.5 + stroke_width / 2.0;

        let x_scale_span = x_domain_max - x_domain_min;
        let y_scale_span = y_domain_max - y_domain_min;
        let padding_left = (min_x - x_domain_min) * final_context.plot_width() / x_scale_span;
        let padding_right = (x_domain_max - max_x) * final_context.plot_width() / x_scale_span;
        let padding_bottom = (min_y - y_domain_min) * final_context.plot_height() / y_scale_span;
        let padding_top = (y_domain_max - max_y) * final_context.plot_height() / y_scale_span;

        let tolerance = 0.5;

        assert!(
            padding_left + tolerance >= radius_px,
            "x left padding {padding_left} smaller than radius {radius_px}"
        );
        assert!(
            padding_right + tolerance >= radius_px,
            "x right padding {padding_right} smaller than radius {radius_px}"
        );
        assert!(
            padding_bottom + tolerance >= radius_px,
            "y bottom padding {padding_bottom} smaller than radius {radius_px}"
        );
        assert!(
            padding_top + tolerance >= radius_px,
            "y top padding {padding_top} smaller than radius {radius_px}"
        );
    }
}
