use std::{collections::HashMap, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, DetailColumns, GeometrySpace,
    LegendRendererKind, LegendRendererSelection, Mark, MarkRuntimeContext, PointGeometry,
    RadiusExpression, RenderedMarkData, ScaleTypePreference, apply_opacity_to_color,
    coerce_bool_channel_with_renderer, coerce_color_channel_with_renderer,
    coerce_numeric_channel_with_renderer, coerce_opacity_channel_with_renderer,
    coerce_stroke_cap_channel_values_with_renderer, coerce_stroke_dash_channel,
    coerce_stroke_join_channel_values_with_renderer, default_scale_type_for_data_type,
    impl_mark_trait_common, is_continuous_scale, stroke_rendering,
};
use avenger_chart_marks::{Line, line_channel_defaults};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use avenger_scenegraph::marks::{line::SceneLineMark, mark::SceneMark};
use datafusion::{
    arrow::{array::RecordBatch, datatypes::DataType as ArrowDataType},
    common::ScalarValue,
    logical_expr::Expr,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::super::Polar;

const MAX_POLAR_THETA_STEP: f32 = std::f32::consts::PI / 90.0;
const MAX_POLAR_DISPLAY_STEP_PX: f32 = 6.0;
const MAX_POLAR_SUBDIVISIONS_PER_SEGMENT: usize = 256;

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<Polar> for Line<Polar> {
    impl_mark_trait_common!(Line);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        if !self.mark_effects().is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "Line<Polar> adjustments are not implemented yet".to_string(),
            ));
        }
        Ok(Arc::new(CompiledPolarLine {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledPolarLine {
    pub(crate) state: CompiledMarkState,
}

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
struct LineRenderPartitionKey {
    details: Vec<ScalarValue>,
    stroke: String,
    stroke_width_bits: u32,
    stroke_dash: String,
    opacity_bits: u32,
    stroke_cap: String,
    stroke_join: String,
}

impl CompiledMarkCore for CompiledPolarLine {
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
        "line"
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
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke_join",
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

    fn details_partition_continuous_geometry(&self) -> bool {
        true
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        line_channel_defaults(channel)
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
            _ => default_scale_type_for_data_type(data_type),
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        let is_continuous = is_continuous_scale(scale.scale_impl.as_ref());

        match channel {
            "stroke" if is_continuous => Some(LegendRendererSelection::BuiltIn(
                LegendRendererKind::Colorbar,
            )),
            "stroke" | "stroke_width" | "stroke_dash" | "opacity" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line))
            }
            "r" | "theta" | "defined" | "order" | "stroke_cap" | "stroke_join" => None,
            _ => Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line)),
        }
    }
}

#[typetag::serde]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl CompiledMark for CompiledPolarLine {
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
                "Line mark requires array data for r and theta positions".to_string(),
            )
        })?;
        let mark_context = context.core_view();
        let len = data.num_rows();

        let r = coerce_numeric_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "r",
            &mark_context,
            0.0,
        )?;
        let theta = coerce_numeric_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "theta",
            &mark_context,
            0.0,
        )?;

        let visual = self.coerce_line_visual_channels(Some(data), scalars, &mark_context, len)?;
        let detail_columns = DetailColumns::from_mark_data(self, data)?;
        let stroke_values = visual.stroke.as_vec(len, None);
        let stroke_strings = stroke_rendering::color_channel_strings(&visual.stroke, len);
        let stroke_width_values = visual.stroke_width.as_vec(len, None);
        let opacity_values = visual.opacity.as_vec(len, None);

        let mut partition_groups: IndexMap<LineRenderPartitionKey, Vec<usize>> = IndexMap::new();
        for i in 0..len {
            let key = LineRenderPartitionKey {
                details: detail_columns.key_for_row(i)?,
                stroke: stroke_strings[i].clone(),
                stroke_width_bits: stroke_width_values[i].to_bits(),
                stroke_dash: visual.stroke_dash_strings[i].clone(),
                opacity_bits: opacity_values[i].clamp(0.0, 1.0).to_bits(),
                stroke_cap: visual.stroke_cap_strings[i].clone(),
                stroke_join: visual.stroke_join_strings[i].clone(),
            };
            partition_groups.entry(key).or_default().push(i);
        }

        let geometry_space = self.state.geometry_space_or(GeometrySpace::Coordinate);
        let r_values = r.as_vec(len, None);
        let theta_values = theta.as_vec(len, None);
        let defined_values = visual.defined.as_vec(len, None);
        let display_geometry = if geometry_space == GeometrySpace::Display {
            Some(project_polar_arrays(
                &r_values,
                &theta_values,
                coord,
                context.plot_width(),
                context.plot_height(),
            )?)
        } else {
            None
        };

        let mut scene_marks = Vec::new();
        let mut source_row_indices = Vec::new();

        for (partition_key, indices) in partition_groups {
            if indices.is_empty() {
                continue;
            }
            let first_index = indices[0];

            let stroke_color = apply_opacity_to_color(
                &stroke_values[first_index],
                f32::from_bits(partition_key.opacity_bits),
            );
            let stroke_dash = stroke_rendering::stroke_dash_from_name(&partition_key.stroke_dash)?;
            let stroke_cap = stroke_rendering::coerce_stroke_cap_strings(
                std::slice::from_ref(&partition_key.stroke_cap),
                "stroke_cap",
            )?
            .first()
            .copied()
            .unwrap_or(StrokeCap::Round);
            let stroke_join = stroke_rendering::coerce_stroke_join_strings(
                std::slice::from_ref(&partition_key.stroke_join),
                "stroke_join",
            )?
            .first()
            .copied()
            .unwrap_or(StrokeJoin::Miter);

            let line_geometry = match geometry_space {
                GeometrySpace::Coordinate => sample_coordinate_space_line(
                    &r_values,
                    &theta_values,
                    &defined_values,
                    &indices,
                    coord,
                    context.plot_width(),
                    context.plot_height(),
                )?,
                GeometrySpace::Display => {
                    let display_geometry = display_geometry.as_ref().ok_or_else(|| {
                        AvengerChartError::InternalError(
                            "Missing projected polar display geometry".to_string(),
                        )
                    })?;
                    gather_display_space_line(
                        &display_geometry.x,
                        &display_geometry.y,
                        &defined_values,
                        len,
                        &indices,
                    )
                }
            };

            let line_mark = SceneLineMark {
                name: "line".to_string(),
                clip: true,
                len: line_geometry.len as u32,
                x: line_geometry.x,
                y: line_geometry.y,
                gradients: vec![],
                stroke: stroke_color,
                stroke_width: f32::from_bits(partition_key.stroke_width_bits),
                stroke_dash,
                stroke_cap,
                stroke_join,
                defined: line_geometry.defined,
                zindex: self.state.zindex,
                interactive: true,
            };

            scene_marks.push(SceneMark::Line(line_mark));
            source_row_indices.push(indices);
        }

        Ok(RenderedMarkData::with_source_row_indices(
            scene_marks,
            source_row_indices,
        ))
    }
}

impl CompiledPolarLine {
    fn coerce_line_visual_channels(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &avenger_chart_core::MarkRenderContext<'_>,
        len: usize,
    ) -> Result<LineVisualChannels, AvengerChartError> {
        let stroke = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke",
            context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let stroke_width = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke_width",
            context,
            2.0,
        )?;
        let opacity =
            coerce_opacity_channel_with_renderer(self, data, scalars, "opacity", context, 1.0)?;
        let defined =
            coerce_bool_channel_with_renderer(self, data, scalars, "defined", context, true)?;
        let stroke_dash = stroke_rendering::optional_stroke_dash(coerce_stroke_dash_channel(
            data,
            scalars,
            "stroke_dash",
        )?);
        let stroke_cap = coerce_stroke_cap_channel_values_with_renderer(
            self,
            data,
            scalars,
            "stroke_cap",
            context,
            StrokeCap::Round,
        )?;
        let stroke_join = coerce_stroke_join_channel_values_with_renderer(
            self,
            data,
            scalars,
            "stroke_join",
            context,
            StrokeJoin::Miter,
        )?;

        Ok(LineVisualChannels {
            stroke,
            stroke_width,
            stroke_dash_strings: stroke_rendering::stroke_dash_strings(&stroke_dash, len),
            stroke_cap_strings: stroke_rendering::stroke_cap_strings(&stroke_cap, len),
            stroke_join_strings: stroke_rendering::stroke_join_strings(&stroke_join, len),
            opacity,
            defined,
        })
    }
}

#[derive(Clone)]
struct LineVisualChannels {
    stroke: ScalarOrArray<ColorOrGradient>,
    stroke_width: ScalarOrArray<f32>,
    stroke_dash_strings: Vec<String>,
    stroke_cap_strings: Vec<String>,
    stroke_join_strings: Vec<String>,
    opacity: ScalarOrArray<f32>,
    defined: ScalarOrArray<bool>,
}

struct SampledLineGeometry {
    len: usize,
    x: ScalarOrArray<f32>,
    y: ScalarOrArray<f32>,
    defined: ScalarOrArray<bool>,
}

fn gather_display_space_line(
    x: &ScalarOrArray<f32>,
    y: &ScalarOrArray<f32>,
    defined_values: &[bool],
    len: usize,
    indices: &[usize],
) -> SampledLineGeometry {
    SampledLineGeometry {
        len: indices.len(),
        x: stroke_rendering::gather_by_indices(x, len, indices),
        y: stroke_rendering::gather_by_indices(y, len, indices),
        defined: ScalarOrArray::new_array(
            indices
                .iter()
                .filter_map(|index| defined_values.get(*index).copied())
                .collect(),
        ),
    }
}

fn sample_coordinate_space_line(
    r_values: &[f32],
    theta_values: &[f32],
    defined_values: &[bool],
    indices: &[usize],
    coord: &dyn CoordinateSystemTransformCore,
    plot_width: f32,
    plot_height: f32,
) -> Result<SampledLineGeometry, AvengerChartError> {
    let mut sampled_r = Vec::new();
    let mut sampled_theta = Vec::new();
    let mut sampled_defined = Vec::new();

    for (position, index) in indices.iter().copied().enumerate() {
        if position == 0 {
            push_sample(
                &mut sampled_r,
                &mut sampled_theta,
                &mut sampled_defined,
                r_values[index],
                theta_values[index],
                defined_values[index],
            );
            continue;
        }

        let previous_index = indices[position - 1];
        if defined_values[previous_index] && defined_values[index] {
            let subdivisions = polar_subdivision_count(
                r_values[previous_index],
                theta_values[previous_index],
                r_values[index],
                theta_values[index],
            );
            for step in 1..subdivisions {
                let t = step as f32 / subdivisions as f32;
                push_sample(
                    &mut sampled_r,
                    &mut sampled_theta,
                    &mut sampled_defined,
                    lerp(r_values[previous_index], r_values[index], t),
                    lerp(theta_values[previous_index], theta_values[index], t),
                    true,
                );
            }
        }

        push_sample(
            &mut sampled_r,
            &mut sampled_theta,
            &mut sampled_defined,
            r_values[index],
            theta_values[index],
            defined_values[index],
        );
    }

    let geometry =
        project_polar_arrays(&sampled_r, &sampled_theta, coord, plot_width, plot_height)?;
    Ok(SampledLineGeometry {
        len: sampled_r.len(),
        x: geometry.x,
        y: geometry.y,
        defined: ScalarOrArray::new_array(sampled_defined),
    })
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
                "Failed to downcast transformed polar line points to PointGeometry".to_string(),
            )
        })
}

fn push_sample(
    sampled_r: &mut Vec<f32>,
    sampled_theta: &mut Vec<f32>,
    sampled_defined: &mut Vec<bool>,
    r: f32,
    theta: f32,
    defined: bool,
) {
    sampled_r.push(r);
    sampled_theta.push(theta);
    sampled_defined.push(defined);
}

fn polar_subdivision_count(r0: f32, theta0: f32, r1: f32, theta1: f32) -> usize {
    let dtheta = (theta1 - theta0).abs();
    let dr = (r1 - r0).abs();
    let angular_distance = 0.5 * (r0.abs() + r1.abs()) * dtheta;
    let estimated_distance = dr.hypot(angular_distance);
    let theta_steps = (dtheta / MAX_POLAR_THETA_STEP).ceil() as usize;
    let display_steps = (estimated_distance / MAX_POLAR_DISPLAY_STEP_PX).ceil() as usize;
    theta_steps
        .max(display_steps)
        .clamp(1, MAX_POLAR_SUBDIVISIONS_PER_SEGMENT)
}

fn lerp(start: f32, end: f32, t: f32) -> f32 {
    start + (end - start) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinate_space_subdivides_arc() {
        let geometry = sample_coordinate_space_line(
            &[50.0, 50.0],
            &[0.0, std::f32::consts::FRAC_PI_2],
            &[true, true],
            &[0, 1],
            &Polar::new(),
            200.0,
            200.0,
        )
        .unwrap();

        assert!(geometry.len > 2);
        let x = geometry.x.as_vec(geometry.len, None);
        let y = geometry.y.as_vec(geometry.len, None);
        assert!((x[0] - 150.0).abs() < 1e-4);
        assert!((y[0] - 100.0).abs() < 1e-4);
        assert!((x[geometry.len - 1] - 100.0).abs() < 1e-4);
        assert!((y[geometry.len - 1] - 150.0).abs() < 1e-4);
    }

    #[test]
    fn coordinate_space_samples_radial_segment() {
        let geometry = sample_coordinate_space_line(
            &[10.0, 40.0],
            &[std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2],
            &[true, true],
            &[0, 1],
            &Polar::new(),
            200.0,
            200.0,
        )
        .unwrap();

        assert!(geometry.len > 2);
        let x = geometry.x.as_vec(geometry.len, None);
        let y = geometry.y.as_vec(geometry.len, None);
        assert!(x.iter().all(|value| (*value - 100.0).abs() < 1e-4));
        assert!((y[0] - 110.0).abs() < 1e-4);
        assert!((y[geometry.len - 1] - 140.0).abs() < 1e-4);
    }

    #[test]
    fn coordinate_space_samples_spiral_like_segment() {
        let geometry = sample_coordinate_space_line(
            &[10.0, 40.0],
            &[0.0, std::f32::consts::FRAC_PI_2],
            &[true, true],
            &[0, 1],
            &Polar::new(),
            200.0,
            200.0,
        )
        .unwrap();

        assert!(geometry.len > 2);
        let x = geometry.x.as_vec(geometry.len, None);
        let y = geometry.y.as_vec(geometry.len, None);
        assert!((x[0] - 110.0).abs() < 1e-4);
        assert!((y[0] - 100.0).abs() < 1e-4);
        assert!((x[geometry.len - 1] - 100.0).abs() < 1e-4);
        assert!((y[geometry.len - 1] - 140.0).abs() < 1e-4);
    }

    #[test]
    fn display_space_keeps_original_vertices() {
        let geometry = gather_display_space_line(
            &ScalarOrArray::new_array(vec![1.0, 2.0, 3.0]),
            &ScalarOrArray::new_array(vec![4.0, 5.0, 6.0]),
            &[true, false, true],
            3,
            &[0, 2],
        );

        assert_eq!(geometry.len, 2);
        assert_eq!(geometry.x.as_vec(2, None), vec![1.0, 3.0]);
        assert_eq!(geometry.y.as_vec(2, None), vec![4.0, 6.0]);
        assert_eq!(geometry.defined.as_vec(2, None), vec![true, true]);
    }

    #[test]
    fn undefined_segments_are_not_subdivided() {
        let geometry = sample_coordinate_space_line(
            &[50.0, 50.0],
            &[0.0, std::f32::consts::FRAC_PI_2],
            &[true, false],
            &[0, 1],
            &Polar::new(),
            200.0,
            200.0,
        )
        .unwrap();

        assert_eq!(geometry.len, 2);
        assert_eq!(geometry.defined.as_vec(2, None), vec![true, false]);
    }

    #[test]
    fn theta_wrap_is_interpolated_as_given() {
        let geometry = sample_coordinate_space_line(
            &[50.0, 50.0],
            &[std::f32::consts::TAU - 0.1, 0.1],
            &[true, true],
            &[0, 1],
            &Polar::new(),
            200.0,
            200.0,
        )
        .unwrap();

        let x = geometry.x.as_vec(geometry.len, None);
        let min_x = x.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(
            min_x < 60.0,
            "as-given interpolation should take the long route through the left side, got min x {min_x}"
        );
    }

    #[test]
    fn subdivision_count_is_capped() {
        let subdivisions =
            polar_subdivision_count(100_000.0, 0.0, 100_000.0, std::f32::consts::TAU);
        assert_eq!(subdivisions, MAX_POLAR_SUBDIVISIONS_PER_SEGMENT);

        let geometry = sample_coordinate_space_line(
            &[100_000.0, 100_000.0],
            &[0.0, std::f32::consts::TAU],
            &[true, true],
            &[0, 1],
            &Polar::new(),
            200.0,
            200.0,
        )
        .unwrap();
        assert_eq!(geometry.len, MAX_POLAR_SUBDIVISIONS_PER_SEGMENT + 1);
    }
}
