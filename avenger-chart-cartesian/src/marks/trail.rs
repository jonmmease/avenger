use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererKind, LegendRendererSelection,
    Mark, MarkRuntimeContext, ResolvedDomain, ScaleRange, ScaleTypePreference, Theme,
    apply_opacity_to_color, coerce_bool_channel_with_renderer, coerce_color_channel_with_renderer,
    coerce_numeric_channel_with_renderer, coerce_opacity_channel_with_renderer,
    default_scale_type_for_data_type, impl_mark_trait_common, is_continuous_scale,
};
use avenger_chart_marks::{
    Trail, TrailPartitionKey, ensure_dictionary_array, trail_channel_defaults,
};
use avenger_common::{types::ColorOrGradient, value::ScalarOrArrayValue};
use avenger_scales::scales::{ConfiguredScale, ScaleImpl, coerce::Coercer};
use avenger_scenegraph::marks::{mark::SceneMark, trail::SceneTrailMark};
use datafusion::{
    arrow::array::{AsArray, RecordBatch},
    arrow::datatypes::DataType,
    common::ScalarValue,
    logical_expr::lit,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{Cartesian, marks::util};

#[async_trait::async_trait]
impl Mark<Cartesian> for Trail<Cartesian> {
    impl_mark_trait_common!(Trail);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianTrail {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianTrail {
    pub(crate) state: CompiledMarkState,
}

impl CompiledMarkCore for CompiledCartesianTrail {
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
        "trail"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: "x",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "y",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "size",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke",
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
                name: "defined",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "order",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn supports_order(&self) -> bool {
        true
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        trail_channel_defaults(channel)
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        match (channel, data_type) {
            ("x" | "y", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Point)
            }
            (
                "size",
                DataType::Float32
                | DataType::Float64
                | DataType::Int8
                | DataType::Int16
                | DataType::Int32
                | DataType::Int64
                | DataType::UInt8
                | DataType::UInt16
                | DataType::UInt32
                | DataType::UInt64,
            ) => Some(ScaleTypePreference::Sqrt),
            ("size", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Ordinal)
            }
            ("stroke", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Ordinal)
            }
            _ => default_scale_type_for_data_type(data_type),
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        match channel {
            "stroke" if is_continuous_scale(scale.scale_impl.as_ref()) => Some(
                LegendRendererSelection::BuiltIn(LegendRendererKind::Colorbar),
            ),
            "stroke" => Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line)),
            "size" | "opacity" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Symbol))
            }
            "x" | "y" | "defined" | "order" => None,
            _ => Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line)),
        }
    }

    fn default_channel_range(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        domain: &ResolvedDomain,
        _data_type: &DataType,
        theme: &Theme,
        params: &indexmap::IndexMap<String, ScalarValue>,
    ) -> Option<ScaleRange> {
        let range_kind = scale_impl.range_kind();
        let cardinality = match domain {
            ResolvedDomain::Discrete(count) => Some(*count),
            ResolvedDomain::Interval => None,
        };

        if let Some(theme_range) =
            theme.get_range_for_channel("trail", channel, range_kind, cardinality, params)
        {
            return Some(theme_range);
        }

        match channel {
            "size" => match domain {
                ResolvedDomain::Discrete(count) => {
                    if *count == 1 {
                        Some(ScaleRange::new_discrete(vec![ScalarValue::Float32(Some(
                            8.0,
                        ))]))
                    } else {
                        Some(ScaleRange::new_linspace_discrete(2.0, 18.0, *count))
                    }
                }
                ResolvedDomain::Interval => Some(ScaleRange::new_interval(lit(2.0), lit(18.0))),
            },
            "opacity" => Some(domain.make_interval_or_linspaced_range(0.0, 1.0)),
            _ => None,
        }
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianTrail {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let data = data.ok_or_else(|| {
            AvengerChartError::InternalError(
                "Trail mark requires array data for x and y positions".to_string(),
            )
        })?;
        let mark_context = context.core_view();
        let len = data.num_rows();
        let coercer = Coercer::default();

        let position = util::transform_cartesian_point_channels(
            self,
            Some(data),
            scalars,
            context,
            coord,
            "x",
            "y",
        )?;
        let size = coerce_numeric_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "size",
            &mark_context,
            1.0,
        )?;
        let defined = coerce_bool_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "defined",
            &mark_context,
            true,
        )?;

        let stroke_array = data.column_by_name("stroke");
        let opacity_array = data.column_by_name("opacity");

        if stroke_array.is_none() && opacity_array.is_none() {
            let stroke = first_color(coerce_color_channel_with_renderer(
                self,
                None,
                scalars,
                "stroke",
                &mark_context,
                [0.0, 0.0, 0.0, 1.0],
            )?);
            let opacity = coerce_opacity_channel_with_renderer(
                self,
                None,
                scalars,
                "opacity",
                &mark_context,
                1.0,
            )?
            .first()
            .cloned()
            .unwrap_or(1.0);
            return Ok(vec![
                SceneTrailMark {
                    name: "trail".to_string(),
                    clip: true,
                    len: len as u32,
                    gradients: vec![],
                    stroke: apply_opacity_to_color(&stroke, opacity),
                    x: position.x,
                    y: position.y,
                    size,
                    defined,
                    zindex: self.state.zindex,
                    interactive: true,
                }
                .into(),
            ]);
        }

        let stroke_keys = stroke_array
            .map(ensure_dictionary_array)
            .transpose()?
            .map(|d| {
                let dict = d.as_any_dictionary();
                (d.clone(), dict.normalized_keys())
            });
        let opacity_keys = opacity_array
            .map(ensure_dictionary_array)
            .transpose()?
            .map(|d| {
                let dict = d.as_any_dictionary();
                (d.clone(), dict.normalized_keys())
            });

        let stroke_values = if let Some((dict, _)) = &stroke_keys {
            Some(coercer.to_color(
                dict.as_any_dictionary().values(),
                Some(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
            )?)
        } else {
            None
        };
        let opacity_values = if let Some((dict, _)) = &opacity_keys {
            Some(
                coercer
                    .to_numeric(dict.as_any_dictionary().values(), Some(1.0))?
                    .map(|v| v.clamp(0.0, 1.0)),
            )
        } else {
            None
        };

        let stroke_default = first_color(coerce_color_channel_with_renderer(
            self,
            None,
            scalars,
            "stroke",
            &mark_context,
            [0.0, 0.0, 0.0, 1.0],
        )?);
        let opacity_default = coerce_opacity_channel_with_renderer(
            self,
            None,
            scalars,
            "opacity",
            &mark_context,
            1.0,
        )?
        .first()
        .cloned()
        .unwrap_or(1.0);

        let mut groups: IndexMap<TrailPartitionKey, Vec<usize>> = IndexMap::new();
        for i in 0..len {
            let key = TrailPartitionKey {
                stroke: dictionary_key(&stroke_keys, i),
                opacity: dictionary_key(&opacity_keys, i),
            };
            groups.entry(key).or_default().push(i);
        }

        let mut marks = Vec::new();
        for (key, indices) in groups {
            let stroke = key
                .stroke
                .and_then(|key| {
                    stroke_values
                        .as_ref()
                        .map(|values| values.as_vec(values.len(), None)[key].clone())
                })
                .unwrap_or_else(|| stroke_default.clone());
            let opacity = key
                .opacity
                .and_then(|key| {
                    opacity_values
                        .as_ref()
                        .map(|values| values.as_vec(values.len(), None)[key])
                })
                .unwrap_or(opacity_default);

            marks.push(SceneMark::Trail(SceneTrailMark {
                name: "trail".to_string(),
                clip: true,
                len: indices.len() as u32,
                gradients: vec![],
                stroke: apply_opacity_to_color(&stroke, opacity),
                x: util::gather_by_indices(&position.x, len, &indices),
                y: util::gather_by_indices(&position.y, len, &indices),
                size: util::gather_by_indices(&size, len, &indices),
                defined: util::gather_by_indices(&defined, len, &indices),
                zindex: self.state.zindex,
                interactive: true,
            }));
        }

        Ok(marks)
    }
}

fn first_color(colors: avenger_common::value::ScalarOrArray<ColorOrGradient>) -> ColorOrGradient {
    match colors.value() {
        ScalarOrArrayValue::Scalar(color) => color.clone(),
        ScalarOrArrayValue::Array(colors) => colors
            .first()
            .cloned()
            .unwrap_or_else(ColorOrGradient::transparent),
    }
}

fn dictionary_key(
    keys: &Option<(datafusion::arrow::array::ArrayRef, Vec<usize>)>,
    index: usize,
) -> Option<usize> {
    keys.as_ref().and_then(|(array, keys)| {
        let dict = array.as_any_dictionary();
        if dict.is_null(index) {
            None
        } else {
            Some(keys[index])
        }
    })
}
