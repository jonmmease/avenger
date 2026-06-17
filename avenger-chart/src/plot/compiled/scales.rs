//! Facade compatibility for scale building owned by `avenger-chart-scales`.

#[cfg(test)]
use std::collections::HashMap;
use std::sync::Arc;

use datafusion::{
    arrow::datatypes::DataType as ArrowDataType, common::ScalarValue, dataframe::DataFrame,
};
#[cfg(test)]
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;

#[cfg(test)]
use avenger_chart_core::CoordinateSystemTransform;
use avenger_chart_core::{
    AvengerChartError, CompiledMark, EvaluationContext as CoreEvaluationContext, ResolvedDomain,
    ScaleRange, Theme,
};
#[cfg(test)]
use avenger_chart_scales::PlotScaleSpec;
use avenger_chart_scales::{PreparedScaleMark, ScaleBuilder};
use avenger_scales::scales::ScaleImpl;

use crate::facet::evaluated_facet_tree::EvaluatedFacetTree;

use super::{
    CompiledPlot, LogicalMarkDataRequest, prepare_logical_mark_data,
    session::{ScaleDomainCacheScope, scale_domain_cache_key_for_parts_with_scope},
};

async fn prepare_scale_mark_for_plot(
    plot: &CompiledPlot,
    mark: &Arc<dyn CompiledMark>,
    df_override: Option<&DataFrame>,
    eval_ctx: &crate::render::EvaluationContext,
    facet_path: &[ScalarValue],
) -> Result<PreparedScaleMark, AvengerChartError> {
    let prepared_base = match plot.mark_group_index_for_mark(mark.state().mark_index()) {
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
    Ok(PreparedScaleMark::new_with_domain_source(
        mark.clone(),
        prepared.dataframe,
        prepared.channels,
        prepared.domain_dataframe,
        prepared.domain_channels,
        prepared.derived_scalars,
    ))
}

#[cfg(test)]
pub(crate) async fn build_scale_builder_from_marks(
    compiled_marks: &[Arc<dyn CompiledMark>],
    scale_specs: &HashMap<String, PlotScaleSpec>,
    coord_transform: &Box<dyn CoordinateSystemTransform>,
    data: &Option<LogicalPlanNode>,
    df_override: Option<DataFrame>,
    eval_ctx: &CoreEvaluationContext,
    theme: &Theme,
) -> Result<ScaleBuilder, AvengerChartError> {
    let render_eval_ctx = crate::render::EvaluationContext::new(
        eval_ctx.theme().clone(),
        eval_ctx.session_context().clone(),
        eval_ctx.params().clone(),
        Arc::new(EvaluatedFacetTree::empty()),
    )
    .with_time_context(eval_ctx.time_context().clone());
    let mut prepared_marks = Vec::with_capacity(compiled_marks.len());
    for mark in compiled_marks {
        let prepared = Box::pin(prepare_logical_mark_data(LogicalMarkDataRequest {
            mark: mark.as_ref(),
            plot_data: data.as_ref(),
            provided_plot_df: df_override.as_ref(),
            facet_data_scope: None,
            prepared_base: None,
            eval_ctx: &render_eval_ctx,
        }))
        .await?;
        prepared_marks.push(PreparedScaleMark::new_with_domain_source(
            mark.clone(),
            prepared.dataframe,
            prepared.channels,
            prepared.domain_dataframe,
            prepared.domain_channels,
            prepared.derived_scalars,
        ));
    }

    Box::pin(
        avenger_chart_scales::build_scale_builder_from_prepared_marks(
            &prepared_marks,
            scale_specs,
            coord_transform.as_ref(),
            eval_ctx,
            theme,
        ),
    )
    .await
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
    use super::build_scale_builder_from_marks;
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

        let builder = build_scale_builder_from_marks(
            &compiled.marks,
            &compiled.scale_specs,
            &compiled.coord_transform,
            &compiled.data,
            None,
            &avenger_chart_core::EvaluationContext::new(
                compiled.get_theme(),
                Arc::new(ctx.clone()),
                params.clone(),
            )
            .with_time_context(compiled.time_context.clone()),
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
