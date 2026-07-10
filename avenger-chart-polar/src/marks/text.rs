use std::{collections::HashMap, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, GeometrySpace, LegendRendererSelection, Mark,
    MarkRuntimeContext, PointGeometry, RadiusExpression, RenderedMarkData, ScaleTypePreference,
    coerce_numeric_channel_with_renderer, default_scale_type_for_data_type, impl_mark_trait_common,
    is_continuous_scale,
    text_rendering::{
        apply_text_adjustments, apply_text_syntax_and_params, build_scene_text_mark,
        build_scene_text_mark_with_angle,
    },
};
use avenger_chart_marks::{Text, text_channel_defaults};
use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_text::types::TextSyntaxMode;
use datafusion::{
    arrow::{array::RecordBatch, datatypes::DataType as ArrowDataType},
    common::ScalarValue,
    logical_expr::{Expr, lit},
};
use serde::{Deserialize, Serialize};

use super::super::Polar;

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<Polar> for Text<Polar> {
    impl_mark_trait_common!(Text);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledPolarText {
            state: compiled_state,
            effects: self.mark_effects().clone(),
            syntax_mode: self.text_syntax_mode(),
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledPolarText {
    pub(crate) state: CompiledMarkState,
    #[serde(default)]
    pub(crate) effects: avenger_chart_core::PrimitiveMarkEffects,
    #[serde(default)]
    pub(crate) syntax_mode: TextSyntaxMode,
}

impl CompiledMarkCore for CompiledPolarText {
    avenger_chart_core::impl_mark_with_data_context!();

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
        "text"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: "r",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "theta",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "text",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "align",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "baseline",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "angle",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "color",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "font",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "font_size",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "font_weight",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "font_style",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "limit",
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
                name: "leader",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_offset_x",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_offset_y",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_stroke",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_stroke_width",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_stroke_dash",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_stroke_cap",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_stroke_join",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_label_padding",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_target_radius",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_min_length",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_shape",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_arrow",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_arrow_length",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_arrow_width",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        text_channel_defaults(channel)
    }

    fn wants_full_data_batch(&self) -> bool {
        self.effects.requires_data_batch()
    }

    fn radius_expression(
        &self,
        _dimension: &str,
        _resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        None
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &ArrowDataType,
    ) -> Option<ScaleTypePreference> {
        match (channel, data_type) {
            (
                "r" | "theta",
                ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 | ArrowDataType::Utf8View,
            ) => Some(ScaleTypePreference::Point),
            ("color", ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 | ArrowDataType::Utf8View) => {
                Some(ScaleTypePreference::Ordinal)
            }
            (
                "leader_stroke",
                ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 | ArrowDataType::Utf8View,
            ) => Some(ScaleTypePreference::Ordinal),
            (
                "text"
                | "font"
                | "align"
                | "baseline"
                | "font_weight"
                | "font_style"
                | "defined"
                | "leader"
                | "leader_offset_x"
                | "leader_offset_y"
                | "leader_label_padding"
                | "leader_target_radius"
                | "leader_min_length"
                | "leader_shape"
                | "leader_arrow"
                | "leader_arrow_length"
                | "leader_arrow_width"
                | "leader_stroke_cap"
                | "leader_stroke_join",
                _,
            ) => None,
            _ => default_scale_type_for_data_type(data_type),
        }
    }

    fn preferred_legend_renderer(
        &self,
        _channel: &str,
        _scale: &ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        None
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        _data_type: &ArrowDataType,
    ) -> HashMap<String, Expr> {
        let mut options = HashMap::new();
        if matches!(channel, "color" | "leader_stroke") && is_continuous_scale(scale_impl) {
            options.insert("nice".to_string(), lit(true));
        }
        options
    }
}

#[typetag::serde]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl CompiledMark for CompiledPolarText {
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
        let mark_context = context.core_view();
        let len = data.map_or(1, |data| data.num_rows());

        let r = coerce_numeric_channel_with_renderer(self, data, scalars, "r", &mark_context, 0.0)?;
        let theta =
            coerce_numeric_channel_with_renderer(self, data, scalars, "theta", &mark_context, 0.0)?;
        let geometry = project_polar_arrays(
            &r.as_vec(len, None),
            &theta.as_vec(len, None),
            coord,
            context.plot_width(),
            context.plot_height(),
        )?;

        let mut mark = match self.state.geometry_space_or(GeometrySpace::Coordinate) {
            GeometrySpace::Display => build_scene_text_mark(
                self,
                data,
                scalars,
                &mark_context,
                geometry.x,
                geometry.y,
                len as u32,
                self.state.zindex,
                true,
            )?,
            GeometrySpace::Coordinate => {
                let angle = coerce_numeric_channel_with_renderer(
                    self,
                    data,
                    scalars,
                    "angle",
                    &mark_context,
                    0.0,
                )?;
                let display_angle = coordinate_space_text_angle(&theta, &angle, len);
                build_scene_text_mark_with_angle(
                    self,
                    data,
                    scalars,
                    &mark_context,
                    geometry.x,
                    geometry.y,
                    Some(display_angle),
                    len as u32,
                    self.state.zindex,
                    true,
                )?
            }
        };
        apply_text_syntax_and_params(&mut mark, self.syntax_mode, &mark_context);

        let mark = apply_text_adjustments(
            self,
            mark,
            data,
            None,
            context,
            &self.effects,
            self.state.zindex,
        )?;
        Ok(RenderedMarkData::new(vec![mark.into()]))
    }
}

fn coordinate_space_text_angle(
    theta: &ScalarOrArray<f32>,
    angle: &ScalarOrArray<f32>,
    len: usize,
) -> ScalarOrArray<f32> {
    let theta_values = theta.as_vec(len, None);
    let angle_values = angle.as_vec(len, None);
    ScalarOrArray::new_array(
        theta_values
            .into_iter()
            .zip(angle_values)
            .map(|(theta, angle)| theta.to_degrees() + angle)
            .collect(),
    )
    .to_scalar_if_len_one()
}

fn project_polar_arrays(
    r_values: &[f32],
    theta_values: &[f32],
    coord: &dyn CoordinateSystemTransformCore,
    plot_width: f32,
    plot_height: f32,
) -> Result<PointGeometry, AvengerChartError> {
    let mut position_channels = HashMap::new();
    position_channels.insert("r", ScalarOrArray::new_array(r_values.to_vec()));
    position_channels.insert("theta", ScalarOrArray::new_array(theta_values.to_vec()));

    let geometry = coord.transform(&position_channels, None, plot_width, plot_height)?;
    geometry
        .as_any()
        .downcast_ref::<PointGeometry>()
        .cloned()
        .ok_or_else(|| {
            AvengerChartError::CoordinateSystemError(
                "Failed to downcast transformed polar text points to PointGeometry".to_string(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinate_text_angle_matches_polar_radial_basis() {
        let theta = 1.2_f32;
        let r = 40.0_f32;
        let epsilon = 0.01_f32;

        let p0 = project_polar_arrays(&[r], &[theta], &Polar::new(), 200.0, 200.0).unwrap();
        let p1 =
            project_polar_arrays(&[r + epsilon], &[theta], &Polar::new(), 200.0, 200.0).unwrap();
        let x0 = p0.x.as_vec(1, None)[0];
        let y0 = p0.y.as_vec(1, None)[0];
        let x1 = p1.x.as_vec(1, None)[0];
        let y1 = p1.y.as_vec(1, None)[0];
        let finite_difference_angle = (y1 - y0).atan2(x1 - x0).to_degrees();
        let text_angle = coordinate_space_text_angle(
            &ScalarOrArray::new_scalar(theta),
            &ScalarOrArray::new_scalar(0.0),
            1,
        )
        .as_vec(1, None)[0];

        assert_angle_close(finite_difference_angle, text_angle);
    }

    #[test]
    fn coordinate_text_angle_composes_with_user_angle() {
        let text_angle = coordinate_space_text_angle(
            &ScalarOrArray::new_array(vec![0.0, std::f32::consts::FRAC_PI_2]),
            &ScalarOrArray::new_array(vec![0.0, 90.0]),
            2,
        );

        assert_eq!(text_angle.as_vec(2, None), vec![0.0, 180.0]);
    }

    fn assert_angle_close(actual: f32, expected: f32) {
        let delta = ((actual - expected + 180.0).rem_euclid(360.0) - 180.0).abs();
        assert!(
            delta < 1e-2,
            "expected angle {expected}, got {actual} with delta {delta}"
        );
    }
}
