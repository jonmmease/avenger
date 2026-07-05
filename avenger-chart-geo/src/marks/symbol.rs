//! `Symbol<Geo>`: point marks positioned by projected coordinates.
//!
//! Adapted from `avenger-chart-webmercator/src/marks/symbol.rs` (the
//! compiled machinery is coordinate-agnostic; positions arrive as raw
//! projected units on the x/y channels and the coordinate transform
//! passes scaled pixels through).

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use avenger_chart_core::{
    AdjustmentTransformContext, AdjustmentTransformRequirements, AvengerChartError,
    ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore, CompiledMarkState,
    CompiledScalarExpressionProgram, CoordinateSystemTransformCore, DerivedPrimitiveMarkSpec,
    DerivedRuleMarkSpec, DerivedSymbolMarkSpec, DerivedTextMarkSpec, LegendRendererSelection, Mark,
    MarkEvaluationFrame, MarkRenderContext, MarkRuntimeContext, PhysicalScalarExpressionSpec,
    PhysicalScalarProgramOptions, PlotAreaInfo, PointGeometry, PrimitiveMarkEffects,
    RadiusExpression, RenderedMarkData, ScalarValueHelpers, ScaleRange, ScaleTypePreference,
    apply_opacity_to_color_channel, coerce_color_channel_with_renderer,
    coerce_numeric_channel_with_renderer, coerce_opacity_channel_with_renderer,
    coerce_pattern_channel_with_renderer, coerce_stroke_cap_channel_values_with_renderer,
    coerce_stroke_dash_channel, coerce_text_channel, default_scale_type_for_data_type,
    evaluate_item_assignments, impl_mark_trait_common, is_continuous_scale, item_bbox_column_name,
    item_channel_column_name, item_data_column_name,
    serialization::DefaultLogicalExprNodeExt,
    stroke_rendering,
    text_rendering::{apply_text_adjustments, build_scene_text_mark},
};
use avenger_chart_marks::{
    Symbol, symbol_channel_defaults, symbol_legend_renderer_kind, text_channel_defaults,
};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{StrokeCap, SymbolShape},
    value::ScalarOrArray,
};
use avenger_scales::scales::{ConfiguredScale, ScaleImpl, coerce::Coercer};
use avenger_scenegraph::marks::{
    mark::SceneMark, pattern::PatternFill, rule::SceneRuleMark, symbol::SceneSymbolMark,
};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float32Array, RecordBatch, StringArray},
        datatypes::{DataType, Field},
    },
    common::ScalarValue,
    functions::expr_fn::sqrt,
    logical_expr::{Expr, lit},
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};

use crate::Geo;

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<Geo> for Symbol<Geo> {
    impl_mark_trait_common!(Symbol);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledGeoSymbol {
            state: compiled_state,
            effects: self.mark_effects().clone(),
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledGeoSymbol {
    pub(crate) state: CompiledMarkState,
    #[serde(default)]
    pub(crate) effects: PrimitiveMarkEffects,
}

impl CompiledMarkCore for CompiledGeoSymbol {
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
        "symbol"
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
                name: "lon",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "lat",
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
                name: "fill",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "fill_pattern",
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
                allow_column_ref: false,
            },
            ChannelDescriptor {
                name: "shape",
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
                name: "opacity",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        symbol_channel_defaults(channel)
    }

    fn wants_full_data_batch(&self) -> bool {
        self.effects.requires_data_batch()
    }

    fn radius_expression(
        &self,
        dimension: &str,
        resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        match dimension {
            "x" | "y" => {
                let size_expr = resolve_channel("size");
                let stroke_width_expr = resolve_channel("stroke_width");
                let radius_expr =
                    sqrt(size_expr) * lit(0.5) + stroke_width_expr / lit(2.0) + lit(4.0);
                let radius_expr_node = LogicalExprNode::from_default_expr(radius_expr)
                    .expect("Failed to serialize Geo symbol radius expr");
                Some(RadiusExpression::Symmetric(radius_expr_node))
            }
            _ => None,
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        symbol_legend_renderer_kind(channel, scale, &["x", "y", "lon", "lat"])
            .map(LegendRendererSelection::BuiltIn)
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        match (channel, data_type) {
            ("fill_pattern", _) => Some(ScaleTypePreference::Ordinal),
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
            (
                "fill" | "stroke" | "color" | "shape",
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            ) => Some(ScaleTypePreference::Ordinal),
            ("stroke_width", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Ordinal)
            }
            _ => default_scale_type_for_data_type(data_type),
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        _data_type: &DataType,
    ) -> HashMap<String, Expr> {
        let mut options = HashMap::new();
        if channel == "size" && scale_impl.scale_type() == "pow" {
            options.insert("exponent".to_string(), lit(0.5f32));
        }
        if matches!(channel, "fill" | "stroke" | "color") && is_continuous_scale(scale_impl) {
            options.insert("nice".to_string(), lit(true));
        }
        options
    }

    fn default_channel_range(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        domain: &avenger_chart_core::ResolvedDomain,
        _data_type: &DataType,
        theme: &avenger_chart_core::Theme,
        params: &indexmap::IndexMap<String, ScalarValue>,
    ) -> Option<ScaleRange> {
        let range_kind = scale_impl.range_kind();
        let cardinality = match domain {
            avenger_chart_core::ResolvedDomain::Discrete(count) => Some(*count),
            avenger_chart_core::ResolvedDomain::Interval => None,
        };
        if let Some(theme_range) =
            theme.get_range_for_channel("symbol", channel, range_kind, cardinality, params)
        {
            return Some(theme_range);
        }
        match channel {
            "size" => match domain {
                avenger_chart_core::ResolvedDomain::Discrete(count) => {
                    if *count == 1 {
                        Some(ScaleRange::new_discrete(vec![ScalarValue::Float32(Some(
                            400.0,
                        ))]))
                    } else {
                        Some(ScaleRange::new_linspace_discrete(40.0, 400.0, *count))
                    }
                }
                avenger_chart_core::ResolvedDomain::Interval => {
                    Some(ScaleRange::new_interval(lit(0.0), lit(400.0)))
                }
            },
            "angle" => Some(domain.make_interval_or_linspaced_range(0.0, 360.0)),
            "opacity" => Some(domain.make_interval_or_linspaced_range(0.0, 1.0)),
            "stroke_width" => Some(domain.make_interval_or_linspaced_range(0.5, 5.0)),
            _ => None,
        }
    }
}

#[typetag::serde]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl CompiledMark for CompiledGeoSymbol {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let (mark, _) = self.render_symbol_scene(data, scalars, context, coord, false)?;
        Ok(vec![mark])
    }

    async fn render_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        let include_item_frame = self.effects.has_derived();
        let (base_mark, source_frame) =
            self.render_symbol_scene(data, scalars, context, coord, include_item_frame)?;
        let mut marks = vec![base_mark];
        if include_item_frame {
            let source_frame = source_frame.ok_or_else(|| {
                AvengerChartError::InternalError(
                    "Derived Geo symbol rendering expected a source item frame".to_string(),
                )
            })?;
            for derived in &self.effects.derived {
                match derived {
                    DerivedPrimitiveMarkSpec::Symbol(spec) => {
                        marks.push(self.render_derived_symbol(spec, &source_frame, context)?);
                    }
                    DerivedPrimitiveMarkSpec::Rule(spec) => {
                        marks.push(self.render_derived_rule(spec, &source_frame, context)?);
                    }
                    DerivedPrimitiveMarkSpec::Text(spec) => {
                        marks.push(self.render_derived_text(spec, &source_frame, context)?);
                    }
                    other => {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "Symbol<Geo> cannot derive {other:?} yet"
                        )));
                    }
                }
            }
        }
        Ok(RenderedMarkData::new(marks))
    }

    fn has_render_stage_derived(&self) -> bool {
        self.effects.has_derived()
    }

    fn derived_adjustment_requirements(&self) -> AdjustmentTransformRequirements {
        let mut requirements = AdjustmentTransformRequirements::default();
        for derived in &self.effects.derived {
            requirements.merge(derived.transform_requirements());
        }
        requirements
    }

    async fn render_base_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        let (base_mark, _) = self.render_symbol_scene(data, scalars, context, coord, false)?;
        Ok(RenderedMarkData::new(vec![base_mark]))
    }

    async fn render_derived_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        if !self.effects.has_derived() {
            return Ok(RenderedMarkData::new(Vec::new()));
        }
        let (_, source_frame) = self.render_symbol_scene(data, scalars, context, coord, true)?;
        let source_frame = source_frame.ok_or_else(|| {
            AvengerChartError::InternalError(
                "Derived Geo symbol rendering expected a source item frame".to_string(),
            )
        })?;
        let mut marks = Vec::new();
        for derived in &self.effects.derived {
            match derived {
                DerivedPrimitiveMarkSpec::Symbol(spec) => {
                    marks.push(self.render_derived_symbol(spec, &source_frame, context)?);
                }
                DerivedPrimitiveMarkSpec::Rule(spec) => {
                    marks.push(self.render_derived_rule(spec, &source_frame, context)?);
                }
                DerivedPrimitiveMarkSpec::Text(spec) => {
                    marks.push(self.render_derived_text(spec, &source_frame, context)?);
                }
                other => {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Symbol<Geo> cannot derive {other:?} yet"
                    )));
                }
            }
        }
        Ok(RenderedMarkData::new(marks))
    }
}

impl CompiledGeoSymbol {
    fn render_symbol_scene(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
        include_item_frame: bool,
    ) -> Result<(SceneMark, Option<MarkEvaluationFrame>), AvengerChartError> {
        let mark_context = context.core_view();

        let mut position_channels = HashMap::new();
        for channel_name in coord.required_channels() {
            let value = coerce_numeric_channel_with_renderer(
                self,
                data,
                scalars,
                channel_name,
                &mark_context,
                0.0,
            )?;
            position_channels.insert(*channel_name, value);
        }

        let geometry = coord.transform(
            &position_channels,
            None,
            context.plot_width(),
            context.plot_height(),
        )?;
        let geometry = geometry
            .as_any()
            .downcast_ref::<PointGeometry>()
            .ok_or_else(|| {
                AvengerChartError::CoordinateSystemError(
                    "Failed to downcast Geo symbol geometry to PointGeometry".to_string(),
                )
            })?;

        let mut x = geometry.x.clone();
        let mut y = geometry.y.clone();
        // Adaptive Mercator blend: authored-unit positions are only valid
        // at t = 0; re-project through the blended view projector using the
        // spherical lon/lat channels when present.
        if let Some(measurement) =
            crate::view::GeoCoordMeasurement::downcast(context.coord_measurement())
            && measurement.blend_t() > 0.0
            && data.is_some_and(|batch| {
                batch.column_by_name("lon").is_some() && batch.column_by_name("lat").is_some()
            })
        {
            let projector = measurement.view_projector();
            let lon = coerce_numeric_channel_with_renderer(
                self,
                data,
                scalars,
                "lon",
                &mark_context,
                0.0,
            )?;
            let lat = coerce_numeric_channel_with_renderer(
                self,
                data,
                scalars,
                "lat",
                &mark_context,
                0.0,
            )?;
            let rows = data.map(|batch| batch.num_rows()).unwrap_or(1);
            let lon_values = lon.as_vec(rows, None);
            let lat_values = lat.as_vec(rows, None);
            let mut xs = Vec::with_capacity(rows);
            let mut ys = Vec::with_capacity(rows);
            for i in 0..rows {
                match projector.project(f64::from(lon_values[i]), f64::from(lat_values[i])) {
                    Some((px, py)) => {
                        xs.push(px as f32);
                        ys.push(py as f32);
                    }
                    None => {
                        xs.push(f32::NAN);
                        ys.push(f32::NAN);
                    }
                }
            }
            x = ScalarOrArray::new_array(xs);
            y = ScalarOrArray::new_array(ys);
        }
        let size =
            coerce_numeric_channel_with_renderer(self, data, scalars, "size", &mark_context, 64.0)?;
        let angle =
            coerce_numeric_channel_with_renderer(self, data, scalars, "angle", &mark_context, 0.0)?;
        let visual = self.coerce_symbol_visual_channels(data, scalars, &mark_context, context)?;

        let len = data.map_or_else(
            || infer_symbol_item_len(&x, &y, &size, &angle),
            |data| data.num_rows(),
        );
        let (x, y, size, angle, visual) = self.apply_expression_adjustments(
            x,
            y,
            size,
            angle,
            visual,
            data,
            len,
            context,
            &mark_context,
        )?;
        let source_frame = if include_item_frame {
            Some(build_symbol_item_frame(
                &x, &y, &size, &angle, &visual, data, len,
            )?)
        } else {
            None
        };
        let symbol_mark =
            self.build_scene_symbol_mark(x, y, size, angle, visual, len, self.state.zindex)?;

        Ok((SceneMark::Symbol(symbol_mark), source_frame))
    }

    #[allow(clippy::too_many_arguments)]
    fn build_scene_symbol_mark(
        &self,
        x: ScalarOrArray<f32>,
        y: ScalarOrArray<f32>,
        size: ScalarOrArray<f32>,
        angle: ScalarOrArray<f32>,
        visual: SymbolVisualChannels,
        len: usize,
        zindex: Option<i32>,
    ) -> Result<SceneSymbolMark, AvengerChartError> {
        let fill = apply_opacity_to_color_channel(visual.fill, &visual.opacity, len);
        let stroke = apply_opacity_to_color_channel(visual.stroke, &visual.opacity, len);

        Ok(SceneSymbolMark {
            name: "symbol".to_string(),
            clip: true,
            len: len as u32,
            gradients: vec![],
            shapes: visual.shapes,
            stroke_width: visual.stroke_width,
            shape_index: visual.shape_index,
            x,
            y,
            fill,
            fill_pattern: visual.fill_pattern,
            size,
            stroke,
            angle,
            indices: None,
            zindex,
            x_adjustment: None,
            y_adjustment: None,
            interactive: true,
        })
    }

    fn coerce_symbol_visual_channels(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        mark_context: &MarkRenderContext<'_>,
        runtime_context: &dyn MarkRuntimeContext,
    ) -> Result<SymbolVisualChannels, AvengerChartError> {
        let opacity = coerce_opacity_channel_with_renderer(
            self,
            data,
            scalars,
            "opacity",
            mark_context,
            1.0,
        )?;
        let fill = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "fill",
            mark_context,
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0],
        )?;
        let stroke = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke",
            mark_context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let fill_pattern = coerce_pattern_channel_with_renderer(
            self,
            data,
            scalars,
            "fill_pattern",
            runtime_context,
        )?;

        let coercer = Coercer::default();
        let shape_default = self
            .default_channel_value("shape", mark_context)
            .and_then(|scalar| match scalar {
                ScalarValue::Utf8(Some(s)) => SymbolShape::from_vega_str(&s).ok(),
                _ => None,
            })
            .unwrap_or(SymbolShape::Circle);
        let shape_names = coerce_text_channel(
            data,
            scalars,
            "shape",
            default_symbol_shape_name(&shape_default),
        )?;
        let (shapes, shape_index) =
            coerce_symbol_shape_names(&shape_names, Some(shape_default.clone()))?;

        let stroke_width_default = self
            .default_channel_value("stroke_width", mark_context)
            .and_then(|scalar| ScalarValueHelpers::as_f32(&scalar).ok())
            .unwrap_or(1.0);
        let stroke_width = if let Some(width_scalar) = scalars.column_by_name("stroke_width") {
            let val = *coercer
                .to_numeric(width_scalar, Some(stroke_width_default))?
                .first()
                .unwrap();
            Some(val)
        } else {
            Some(stroke_width_default)
        };

        Ok(SymbolVisualChannels {
            fill,
            fill_pattern,
            stroke,
            opacity,
            shape_names,
            shapes,
            stroke_width,
            shape_index,
        })
    }

    fn render_derived_symbol(
        &self,
        spec: &DerivedSymbolMarkSpec,
        source_frame: &MarkEvaluationFrame,
        context: &dyn MarkRuntimeContext,
    ) -> Result<SceneMark, AvengerChartError> {
        let item_batch = source_frame.record_batch()?;
        let mark_context = context.core_view();
        let derived_scalars = evaluate_item_assignments(
            spec.assignments.iter(),
            &item_batch,
            mark_context.session_context().as_ref(),
        )?;
        let len = source_frame.len();
        let x = coerce_numeric_channel_with_renderer(
            self,
            None,
            &derived_scalars,
            "x",
            &mark_context,
            0.0,
        )?;
        let y = coerce_numeric_channel_with_renderer(
            self,
            None,
            &derived_scalars,
            "y",
            &mark_context,
            0.0,
        )?;
        let size = coerce_numeric_channel_with_renderer(
            self,
            None,
            &derived_scalars,
            "size",
            &mark_context,
            64.0,
        )?;
        let angle = coerce_numeric_channel_with_renderer(
            self,
            None,
            &derived_scalars,
            "angle",
            &mark_context,
            0.0,
        )?;
        let visual =
            self.coerce_symbol_visual_channels(None, &derived_scalars, &mark_context, context)?;
        Ok(SceneMark::Symbol(self.build_scene_symbol_mark(
            x,
            y,
            size,
            angle,
            visual,
            len,
            spec.zindex,
        )?))
    }

    fn render_derived_rule(
        &self,
        spec: &DerivedRuleMarkSpec,
        source_frame: &MarkEvaluationFrame,
        context: &dyn MarkRuntimeContext,
    ) -> Result<SceneMark, AvengerChartError> {
        let item_batch = source_frame.record_batch()?;
        let mark_context = context.core_view();
        let derived_scalars = evaluate_item_assignments(
            spec.assignments.iter(),
            &item_batch,
            mark_context.session_context().as_ref(),
        )?;
        let len = source_frame.len();
        let x = coerce_numeric_channel_with_renderer(
            self,
            None,
            &derived_scalars,
            "x",
            &mark_context,
            0.0,
        )?;
        let y = coerce_numeric_channel_with_renderer(
            self,
            None,
            &derived_scalars,
            "y",
            &mark_context,
            0.0,
        )?;
        let x2 = coerce_numeric_channel_with_renderer(
            self,
            None,
            &derived_scalars,
            "x2",
            &mark_context,
            0.0,
        )?;
        let y2 = coerce_numeric_channel_with_renderer(
            self,
            None,
            &derived_scalars,
            "y2",
            &mark_context,
            0.0,
        )?;
        let stroke = coerce_color_channel_with_renderer(
            self,
            None,
            &derived_scalars,
            "stroke",
            &mark_context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let stroke_width = coerce_numeric_channel_with_renderer(
            self,
            None,
            &derived_scalars,
            "stroke_width",
            &mark_context,
            1.0,
        )?;
        let stroke_dash = stroke_rendering::optional_stroke_dash(coerce_stroke_dash_channel(
            None,
            &derived_scalars,
            "stroke_dash",
        )?);
        let stroke_cap = coerce_stroke_cap_channel_values_with_renderer(
            self,
            None,
            &derived_scalars,
            "stroke_cap",
            &mark_context,
            StrokeCap::Butt,
        )?;
        let opacity = coerce_opacity_channel_with_renderer(
            self,
            None,
            &derived_scalars,
            "opacity",
            &mark_context,
            1.0,
        )?;
        let stroke = apply_opacity_to_color_channel(stroke, &opacity, len);

        Ok(SceneMark::Rule(SceneRuleMark {
            name: "rule".to_string(),
            clip: true,
            len: len as u32,
            gradients: vec![],
            stroke_dash,
            x,
            y,
            x2,
            y2,
            stroke,
            stroke_width,
            stroke_cap,
            indices: None,
            zindex: spec.zindex,
            interactive: true,
        }))
    }

    fn render_derived_text(
        &self,
        spec: &DerivedTextMarkSpec,
        source_frame: &MarkEvaluationFrame,
        context: &dyn MarkRuntimeContext,
    ) -> Result<SceneMark, AvengerChartError> {
        let item_batch = source_frame.record_batch()?;
        let mark_context = context.core_view();
        let derived_scalars = evaluate_item_assignments(
            spec.assignments.iter(),
            &item_batch,
            mark_context.session_context().as_ref(),
        )?;
        let len = source_frame.len();
        let text_defaults = DerivedGeoTextDefaults {
            state: self.state.clone(),
            effects: spec.effects.clone(),
        };
        let x = coerce_numeric_channel_with_renderer(
            &text_defaults,
            None,
            &derived_scalars,
            "x",
            &mark_context,
            0.0,
        )?;
        let y = coerce_numeric_channel_with_renderer(
            &text_defaults,
            None,
            &derived_scalars,
            "y",
            &mark_context,
            0.0,
        )?;
        let text_mark = build_scene_text_mark(
            &text_defaults,
            None,
            &derived_scalars,
            &mark_context,
            x,
            y,
            len as u32,
            spec.zindex,
            true,
        )?;
        Ok(apply_text_adjustments(
            &text_defaults,
            text_mark,
            None,
            Some(source_frame),
            context,
            &spec.effects,
            spec.zindex,
        )?
        .into())
    }

    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn apply_expression_adjustments(
        &self,
        mut x: ScalarOrArray<f32>,
        mut y: ScalarOrArray<f32>,
        mut size: ScalarOrArray<f32>,
        mut angle: ScalarOrArray<f32>,
        mut visual: SymbolVisualChannels,
        data: Option<&RecordBatch>,
        len: usize,
        runtime_context: &dyn MarkRuntimeContext,
        context: &MarkRenderContext<'_>,
    ) -> Result<
        (
            ScalarOrArray<f32>,
            ScalarOrArray<f32>,
            ScalarOrArray<f32>,
            ScalarOrArray<f32>,
            SymbolVisualChannels,
        ),
        AvengerChartError,
    > {
        if self.effects.adjustments.is_empty() {
            return Ok((x, y, size, angle, visual));
        }

        for adjustment in &self.effects.adjustments {
            let mut frame = build_symbol_item_frame(&x, &y, &size, &angle, &visual, data, len)?;
            let assignments = match adjustment {
                avenger_chart_core::MarkAdjustmentSpec::Expr(spec) => &spec.assignments,
                avenger_chart_core::MarkAdjustmentSpec::Transform(spec) => {
                    let adjustment_context =
                        AdjustmentTransformContext::new().with_plot_area(PlotAreaInfo {
                            facet_path: runtime_context.facet_path(),
                            width: runtime_context.plot_width(),
                            height: runtime_context.plot_height(),
                            origin: runtime_context.plot_area_origin(),
                            clip: runtime_context.plot_area_clip(),
                        });
                    spec.transform.apply(&mut frame, &adjustment_context)?;
                    &spec.assignments
                }
            };
            if assignments.is_empty() {
                continue;
            }

            let item_batch = frame.record_batch()?;
            let allowed_columns = item_batch
                .schema()
                .fields()
                .iter()
                .map(|field| field.name().clone())
                .collect::<HashSet<_>>();
            let mut output_channels = Vec::with_capacity(assignments.len());
            let mut expression_specs = Vec::with_capacity(assignments.len());
            for (index, assignment) in assignments.iter().enumerate() {
                validate_symbol_adjustment_channel(&assignment.channel)?;
                output_channels.push(assignment.channel.clone());
                let spec = PhysicalScalarExpressionSpec::new(
                    format!("__avenger_adjust_{index}_{}", assignment.channel),
                    assignment.expr(context.session_context().as_ref())?,
                )
                .with_expected_type(symbol_adjustment_channel_type(&assignment.channel)?)
                .with_nullable_cast();
                expression_specs.push(spec);
            }

            let program = CompiledScalarExpressionProgram::compile(
                context.session_context().as_ref(),
                item_batch.schema(),
                expression_specs,
                PhysicalScalarProgramOptions::default().with_allowed_columns(allowed_columns),
            )?;
            let output_batch = program.evaluate_batch(&item_batch)?;

            for (index, channel) in output_channels.iter().enumerate() {
                let array = output_batch.column(index);
                frame.set_column(item_channel_column_name(channel), array.clone())?;
                match channel.as_str() {
                    "x" => {
                        x = ScalarOrArray::new_array(
                            frame.f32_values(&item_channel_column_name("x"))?,
                        )
                        .to_scalar_if_len_one()
                    }
                    "y" => {
                        y = ScalarOrArray::new_array(
                            frame.f32_values(&item_channel_column_name("y"))?,
                        )
                        .to_scalar_if_len_one()
                    }
                    "size" => {
                        size = ScalarOrArray::new_array(
                            frame.f32_values(&item_channel_column_name("size"))?,
                        )
                        .to_scalar_if_len_one()
                    }
                    "angle" => {
                        angle = ScalarOrArray::new_array(
                            frame.f32_values(&item_channel_column_name("angle"))?,
                        )
                        .to_scalar_if_len_one()
                    }
                    "fill" => {
                        visual.fill = stroke_rendering::coerce_color_strings(
                            &frame.string_values(&item_channel_column_name("fill"))?,
                            "fill",
                        )?;
                    }
                    "stroke" => {
                        visual.stroke = stroke_rendering::coerce_color_strings(
                            &frame.string_values(&item_channel_column_name("stroke"))?,
                            "stroke",
                        )?;
                    }
                    "opacity" => {
                        visual.opacity = ScalarOrArray::new_array(
                            frame.f32_values(&item_channel_column_name("opacity"))?,
                        )
                        .to_scalar_if_len_one();
                    }
                    "shape" => {
                        visual.shape_names = ScalarOrArray::new_array(
                            frame.string_values(&item_channel_column_name("shape"))?,
                        )
                        .to_scalar_if_len_one();
                        let (shapes, shape_index) =
                            coerce_symbol_shape_names(&visual.shape_names, None)?;
                        visual.shapes = shapes;
                        visual.shape_index = shape_index;
                    }
                    "stroke_width" => {
                        visual.stroke_width = adjusted_scalar_stroke_width(&frame)?;
                    }
                    _ => unreachable!("validated Geo symbol adjustment channel"),
                }
            }
        }

        Ok((x, y, size, angle, visual))
    }
}

#[derive(Clone)]
struct SymbolVisualChannels {
    fill: ScalarOrArray<ColorOrGradient>,
    fill_pattern: ScalarOrArray<Option<PatternFill>>,
    stroke: ScalarOrArray<ColorOrGradient>,
    opacity: ScalarOrArray<f32>,
    shape_names: ScalarOrArray<String>,
    shapes: Vec<SymbolShape>,
    shape_index: ScalarOrArray<usize>,
    stroke_width: Option<f32>,
}

#[derive(Clone, Serialize, Deserialize)]
struct DerivedGeoTextDefaults {
    state: CompiledMarkState,
    #[serde(default)]
    effects: PrimitiveMarkEffects,
}

impl CompiledMarkCore for DerivedGeoTextDefaults {
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
        Vec::new()
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        text_channel_defaults(channel)
    }

    fn wants_full_data_batch(&self) -> bool {
        self.effects.requires_data_batch()
    }
}

fn validate_symbol_adjustment_channel(channel: &str) -> Result<(), AvengerChartError> {
    match channel {
        "x" | "y" | "size" | "angle" | "fill" | "stroke" | "opacity" | "shape" | "stroke_width" => {
            Ok(())
        }
        other => Err(AvengerChartError::InvalidArgument(format!(
            "Symbol expression adjustment cannot assign unsupported channel '{other}' yet"
        ))),
    }
}

fn symbol_adjustment_channel_type(channel: &str) -> Result<DataType, AvengerChartError> {
    validate_symbol_adjustment_channel(channel)?;
    Ok(match channel {
        "fill" | "stroke" | "shape" => DataType::Utf8,
        _ => DataType::Float32,
    })
}

fn coerce_symbol_shape_names(
    values: &ScalarOrArray<String>,
    default: Option<SymbolShape>,
) -> Result<(Vec<SymbolShape>, ScalarOrArray<usize>), AvengerChartError> {
    let array = Arc::new(StringArray::from(values.as_vec(values.len(), None))) as ArrayRef;
    Coercer::default()
        .to_symbol_shape(&array, default)
        .map(|(shapes, shape_index)| (shapes, shape_index.to_scalar_if_len_one()))
        .map_err(|error| {
            AvengerChartError::InvalidArgument(format!(
                "Error coercing adjusted symbol shape channel: {error}"
            ))
        })
}

fn default_symbol_shape_name(value: &SymbolShape) -> String {
    match value {
        SymbolShape::Circle => "circle",
        SymbolShape::Path(_) => "circle",
    }
    .to_string()
}

#[allow(clippy::type_complexity)]
fn build_symbol_item_frame(
    x: &ScalarOrArray<f32>,
    y: &ScalarOrArray<f32>,
    size: &ScalarOrArray<f32>,
    angle: &ScalarOrArray<f32>,
    visual: &SymbolVisualChannels,
    data: Option<&RecordBatch>,
    len: usize,
) -> Result<MarkEvaluationFrame, AvengerChartError> {
    let x_values = x.as_vec(len, None);
    let y_values = y.as_vec(len, None);
    let size_values = size.as_vec(len, None);
    let angle_values = angle.as_vec(len, None);
    let fill_values = stroke_rendering::color_channel_strings(&visual.fill, len);
    let stroke_values = stroke_rendering::color_channel_strings(&visual.stroke, len);
    let opacity_values = visual.opacity.as_vec(len, None);
    let shape_values = visual.shape_names.as_vec(len, None);
    let stroke_width_values = vec![visual.stroke_width.unwrap_or(1.0); len];
    let bbox = symbol_bbox_values(&x_values, &y_values, &size_values);
    let mut columns = vec![
        (
            Field::new(item_channel_column_name("x"), DataType::Float32, true),
            Arc::new(Float32Array::from(x_values)) as ArrayRef,
        ),
        (
            Field::new(item_channel_column_name("y"), DataType::Float32, true),
            Arc::new(Float32Array::from(y_values)) as ArrayRef,
        ),
        (
            Field::new(item_channel_column_name("size"), DataType::Float32, true),
            Arc::new(Float32Array::from(size_values)) as ArrayRef,
        ),
        (
            Field::new(item_channel_column_name("angle"), DataType::Float32, true),
            Arc::new(Float32Array::from(angle_values)) as ArrayRef,
        ),
        (
            Field::new(item_channel_column_name("fill"), DataType::Utf8, true),
            Arc::new(StringArray::from(fill_values)) as ArrayRef,
        ),
        (
            Field::new(item_channel_column_name("stroke"), DataType::Utf8, true),
            Arc::new(StringArray::from(stroke_values)) as ArrayRef,
        ),
        (
            Field::new(item_channel_column_name("opacity"), DataType::Float32, true),
            Arc::new(Float32Array::from(opacity_values)) as ArrayRef,
        ),
        (
            Field::new(item_channel_column_name("shape"), DataType::Utf8, true),
            Arc::new(StringArray::from(shape_values)) as ArrayRef,
        ),
        (
            Field::new(
                item_channel_column_name("stroke_width"),
                DataType::Float32,
                true,
            ),
            Arc::new(Float32Array::from(stroke_width_values)) as ArrayRef,
        ),
        (
            Field::new(item_bbox_column_name("left"), DataType::Float32, true),
            Arc::new(Float32Array::from(bbox.left)) as ArrayRef,
        ),
        (
            Field::new(item_bbox_column_name("right"), DataType::Float32, true),
            Arc::new(Float32Array::from(bbox.right)) as ArrayRef,
        ),
        (
            Field::new(item_bbox_column_name("top"), DataType::Float32, true),
            Arc::new(Float32Array::from(bbox.top)) as ArrayRef,
        ),
        (
            Field::new(item_bbox_column_name("bottom"), DataType::Float32, true),
            Arc::new(Float32Array::from(bbox.bottom)) as ArrayRef,
        ),
    ];
    if let Some(data) = data {
        if data.num_rows() != len {
            return Err(AvengerChartError::InternalError(format!(
                "Geo symbol adjustment data row count {} did not match item count {len}",
                data.num_rows()
            )));
        }
        for (index, field) in data.schema().fields().iter().enumerate() {
            columns.push((
                Field::new(
                    item_data_column_name(field.name()),
                    field.data_type().clone(),
                    field.is_nullable(),
                ),
                data.column(index).clone(),
            ));
        }
    }
    Ok(MarkEvaluationFrame::new(len, columns))
}

fn adjusted_scalar_stroke_width(
    frame: &MarkEvaluationFrame,
) -> Result<Option<f32>, AvengerChartError> {
    let values = frame.f32_values(&item_channel_column_name("stroke_width"))?;
    let Some(first) = values.first().copied() else {
        return Ok(None);
    };
    if values.iter().all(|value| *value == first) {
        Ok(Some(first))
    } else {
        Err(AvengerChartError::InvalidArgument(
            "Symbol stroke_width adjustment must evaluate to one constant value because SceneSymbolMark stroke_width is scalar".to_string(),
        ))
    }
}

fn infer_symbol_item_len(
    x: &ScalarOrArray<f32>,
    y: &ScalarOrArray<f32>,
    size: &ScalarOrArray<f32>,
    angle: &ScalarOrArray<f32>,
) -> usize {
    x.len().max(y.len()).max(size.len()).max(angle.len()).max(1)
}

struct SymbolBboxValues {
    left: Vec<f32>,
    right: Vec<f32>,
    top: Vec<f32>,
    bottom: Vec<f32>,
}

fn symbol_bbox_values(x: &[f32], y: &[f32], size: &[f32]) -> SymbolBboxValues {
    let mut left = Vec::with_capacity(x.len());
    let mut right = Vec::with_capacity(x.len());
    let mut top = Vec::with_capacity(x.len());
    let mut bottom = Vec::with_capacity(x.len());
    for ((x, y), size) in x.iter().zip(y).zip(size) {
        let half = size.sqrt() * 0.5;
        left.push(*x - half);
        right.push(*x + half);
        top.push(*y - half);
        bottom.push(*y + half);
    }
    SymbolBboxValues {
        left,
        right,
        top,
        bottom,
    }
}
