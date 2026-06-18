use std::{marker::PhantomData, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererKind, LegendRendererSelection,
    Mark, MarkRuntimeContext, RenderedMarkData, apply_opacity_to_color,
    coerce_bool_channel_with_renderer, coerce_color_channel_with_renderer,
    coerce_numeric_channel_with_renderer, define_common_mark_channels, impl_mark_base,
    impl_mark_trait_common,
};
use avenger_chart_marks::line_channel_defaults;
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{StrokeCap, StrokeJoin},
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_scales::scales::coerce::Coercer;
use avenger_scenegraph::marks::{line::SceneLineMark, mark::SceneMark};
use datafusion::{
    arrow::{array::RecordBatch, datatypes::DataType},
    common::ScalarValue,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::Parallel;

pub struct ParallelLine<C = Parallel> {
    pub(crate) state: avenger_chart_core::MarkState,
    pub(crate) _phantom: PhantomData<C>,
}

impl_mark_base!(ParallelLine);

define_common_mark_channels! {
    ParallelLine {
        stroke: {
            allow_column: true,
            with_config: avenger_chart_core::ColorChannelConfig,
        },
        stroke_width: {
            allow_column: true,
            with_config: avenger_chart_core::StrokeWidthChannelConfig,
        },
        stroke_dash: {
            allow_column: true,
            with_config: avenger_chart_core::StrokeDashChannelConfig,
        },
        stroke_cap: {
            allow_column: false,
        },
        stroke_join: {
            allow_column: false,
        },
        opacity: {
            allow_column: true,
            with_config: avenger_chart_core::OpacityChannelConfig,
        },
        defined: {
            allow_column: true,
        },
    }
}

#[async_trait::async_trait]
impl Mark<Parallel> for ParallelLine<Parallel> {
    impl_mark_trait_common!(ParallelLine);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledParallelLine {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledParallelLine {
    pub(crate) state: CompiledMarkState,
}

impl CompiledMarkCore for CompiledParallelLine {
    fn state(&self) -> &CompiledMarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        &mut self.state
    }

    fn data_context(&self) -> &CompiledDataContext {
        &self.state.data
    }

    fn mark_type(&self) -> &str {
        "parallel_line"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: "stroke",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke_width",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke_dash",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "opacity",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke_cap",
                required: false,
                default_value: None,
                allow_column_ref: false,
            },
            ChannelDescriptor {
                name: "stroke_join",
                required: false,
                default_value: None,
                allow_column_ref: false,
            },
            ChannelDescriptor {
                name: "defined",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        line_channel_defaults(channel)
    }

    fn coordinate_channel_dependencies(
        &self,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> IndexMap<String, avenger_chart_core::ChannelValue> {
        coord.generated_position_channels()
    }

    fn preferred_scale_type(
        &self,
        _channel: &str,
        data_type: &DataType,
    ) -> Option<avenger_chart_core::ScaleTypePreference> {
        avenger_chart_core::default_scale_type_for_data_type(data_type)
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        let is_continuous = avenger_chart_core::is_continuous_scale(scale.scale_impl.as_ref());
        match channel {
            "stroke" if is_continuous => Some(LegendRendererSelection::BuiltIn(
                LegendRendererKind::Colorbar,
            )),
            "stroke" | "stroke_width" | "stroke_dash" | "opacity" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line))
            }
            _ => None,
        }
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledParallelLine {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        self.render_mark_data(data, scalars, context, coord)
            .await
            .map(|rendered| rendered.marks)
    }

    async fn render_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        let data = data.ok_or_else(|| {
            AvengerChartError::InternalError(
                "ParallelLine requires inherited or explicit row data".to_string(),
            )
        })?;
        let generated = coord.generated_position_channels();
        if generated.is_empty() {
            return Err(AvengerChartError::CoordinateSystemError(
                "ParallelLine requires a Parallel coordinate system with at least one dimension"
                    .to_string(),
            ));
        }

        let mark_context = context.core_view();
        let row_count = data.num_rows();
        let dimension_count = generated.len();
        let dimension_step = if dimension_count > 1 {
            context.plot_width() / (dimension_count.saturating_sub(1) as f32)
        } else {
            0.0
        };
        let x_positions = (0..dimension_count)
            .map(|index| {
                if dimension_count <= 1 {
                    context.plot_width() / 2.0
                } else {
                    index as f32 * dimension_step
                }
            })
            .collect::<Vec<_>>();

        let dimension_values = generated
            .keys()
            .map(|channel| {
                coerce_numeric_channel_with_renderer(
                    self,
                    Some(data),
                    scalars,
                    channel,
                    &mark_context,
                    f32::NAN,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let defined_values = coerce_bool_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "defined",
            &mark_context,
            true,
        )?;
        let strokes = coerce_color_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "stroke",
            &mark_context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let stroke_widths = coerce_numeric_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "stroke_width",
            &mark_context,
            2.0,
        )?;
        let opacities = coerce_numeric_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "opacity",
            &mark_context,
            1.0,
        )?;
        let stroke_cap = scalar_stroke_cap(self, &mark_context);
        let stroke_join = scalar_stroke_join(self, &mark_context);
        let stroke_dashes = stroke_dash_values(data, scalars)?;

        let mut marks = Vec::with_capacity(row_count);
        let mut source_row_indices = Vec::with_capacity(row_count);
        for row in 0..row_count {
            let row_defined = bool_at(&defined_values, row, true);
            let mut y_values = Vec::with_capacity(dimension_count);
            let mut vertex_defined = Vec::with_capacity(dimension_count);
            for values in &dimension_values {
                let value = f32_at(values, row, f32::NAN);
                y_values.push(value);
                vertex_defined.push(row_defined && value.is_finite());
            }

            let stroke = color_at(&strokes, row);
            let opacity = f32_at(&opacities, row, 1.0).clamp(0.0, 1.0);
            let stroke = apply_opacity_to_color(&stroke, opacity);
            let stroke_width = f32_at(&stroke_widths, row, 2.0);
            let stroke_dash = stroke_dashes
                .as_ref()
                .map(|values| stroke_dash_at(values, row))
                .filter(|dash| !dash.is_empty());

            marks.push(SceneMark::Line(SceneLineMark {
                name: "parallel_line".to_string(),
                clip: true,
                len: dimension_count as u32,
                x: ScalarOrArray::from(x_positions.clone()),
                y: ScalarOrArray::from(y_values),
                gradients: vec![],
                stroke,
                stroke_width,
                stroke_dash,
                stroke_cap,
                stroke_join,
                defined: ScalarOrArray::from(vertex_defined),
                zindex: self.state.zindex,
                interactive: true,
            }));
            source_row_indices.push(vec![row]);
        }

        Ok(RenderedMarkData::with_source_row_indices(
            marks,
            source_row_indices,
        ))
    }
}

fn f32_at(values: &ScalarOrArray<f32>, row: usize, default: f32) -> f32 {
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => *value,
        ScalarOrArrayValue::Array(values) => values.get(row).copied().unwrap_or(default),
    }
}

fn bool_at(values: &ScalarOrArray<bool>, row: usize, default: bool) -> bool {
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => *value,
        ScalarOrArrayValue::Array(values) => values.get(row).copied().unwrap_or(default),
    }
}

fn color_at(values: &ScalarOrArray<ColorOrGradient>, row: usize) -> ColorOrGradient {
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => value.clone(),
        ScalarOrArrayValue::Array(values) => values
            .get(row)
            .cloned()
            .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
    }
}

fn scalar_stroke_cap(
    mark: &CompiledParallelLine,
    context: &avenger_chart_core::MarkRenderContext<'_>,
) -> StrokeCap {
    mark.default_channel_value("stroke_cap", context)
        .and_then(|value| match value {
            ScalarValue::Utf8(Some(value)) => match value.as_str() {
                "butt" => Some(StrokeCap::Butt),
                "round" => Some(StrokeCap::Round),
                "square" => Some(StrokeCap::Square),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or(StrokeCap::Round)
}

fn scalar_stroke_join(
    mark: &CompiledParallelLine,
    context: &avenger_chart_core::MarkRenderContext<'_>,
) -> StrokeJoin {
    mark.default_channel_value("stroke_join", context)
        .and_then(|value| match value {
            ScalarValue::Utf8(Some(value)) => match value.as_str() {
                "miter" => Some(StrokeJoin::Miter),
                "round" => Some(StrokeJoin::Round),
                "bevel" => Some(StrokeJoin::Bevel),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or(StrokeJoin::Round)
}

fn stroke_dash_values(
    data: &RecordBatch,
    scalars: &RecordBatch,
) -> Result<Option<ScalarOrArray<Vec<f32>>>, AvengerChartError> {
    let coercer = Coercer::default();
    if let Some(array) = data.column_by_name("stroke_dash") {
        return Ok(Some(coercer.to_stroke_dash(array)?));
    }
    if let Some(array) = scalars.column_by_name("stroke_dash") {
        return Ok(Some(coercer.to_stroke_dash(array)?));
    }
    Ok(None)
}

fn stroke_dash_at(values: &ScalarOrArray<Vec<f32>>, row: usize) -> Vec<f32> {
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => value.clone(),
        ScalarOrArrayValue::Array(values) => values.get(row).cloned().unwrap_or_default(),
    }
}
