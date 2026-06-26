#![allow(dead_code)]

use std::sync::Arc;

use avenger_chart_core::{
    AdjustmentTransformContext, AdjustmentTransformRequirements, AvengerChartError,
    BasePlotAreaScene, CompiledMarkAdjustmentTransform, GeometryBounds,
    MarkAdjustmentCompileContext, MarkAdjustmentTransform, MarkEvaluationFrame,
    item_channel_column_name,
};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_text::{
    measurement::TextMeasurementConfig,
    types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline},
};
use datafusion::{
    arrow::array::{BooleanArray, Float32Array},
    logical_expr::Expr,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct FixedLabelPlacement {
    dx: f32,
    dy: f32,
}

impl FixedLabelPlacement {
    pub fn new(dx: f32, dy: f32) -> Self {
        Self { dx, dy }
    }
}

#[derive(Clone, Debug)]
pub struct FixedLabelPlacementOutput {
    x: Expr,
    y: Expr,
    defined: Expr,
}

impl FixedLabelPlacementOutput {
    pub fn x(&self) -> Expr {
        self.x.clone()
    }

    pub fn y(&self) -> Expr {
        self.y.clone()
    }

    pub fn defined(&self) -> Expr {
        self.defined.clone()
    }
}

impl MarkAdjustmentTransform for FixedLabelPlacement {
    type Output = FixedLabelPlacementOutput;

    fn compile(
        self,
        ctx: MarkAdjustmentCompileContext,
    ) -> Result<(Box<dyn CompiledMarkAdjustmentTransform>, Self::Output), AvengerChartError> {
        let x_column = ctx.output_column_name("x");
        let y_column = ctx.output_column_name("y");
        let defined_column = ctx.output_column_name("defined");
        let output = FixedLabelPlacementOutput {
            x: ctx.output_expr("x"),
            y: ctx.output_expr("y"),
            defined: ctx.output_expr("defined"),
        };
        Ok((
            Box::new(CompiledFixedLabelPlacement {
                dx: self.dx,
                dy: self.dy,
                x_column,
                y_column,
                defined_column,
            }),
            output,
        ))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CompiledFixedLabelPlacement {
    dx: f32,
    dy: f32,
    x_column: String,
    y_column: String,
    defined_column: String,
}

#[typetag::serde(name = "test_fixed_label_placement")]
impl CompiledMarkAdjustmentTransform for CompiledFixedLabelPlacement {
    fn clone_box(&self) -> Box<dyn CompiledMarkAdjustmentTransform> {
        Box::new(self.clone())
    }

    fn requirements(&self) -> AdjustmentTransformRequirements {
        AdjustmentTransformRequirements::default()
            .with_base_scene()
            .with_text_measurement()
    }

    fn apply(
        &self,
        frame: &mut MarkEvaluationFrame,
        context: &AdjustmentTransformContext<'_>,
    ) -> Result<(), AvengerChartError> {
        let base_scene = context.base_scene.ok_or_else(|| {
            AvengerChartError::InvalidArgument(
                "test FixedLabelPlacement requires a base plot-area scene".to_string(),
            )
        })?;
        let symbol_bounds = symbol_bounds(base_scene);
        let x = frame.f32_values(&item_channel_column_name("x"))?;
        let y = frame.f32_values(&item_channel_column_name("y"))?;
        let text = frame.string_values(&item_channel_column_name("text"))?;
        let font = frame.string_values(&item_channel_column_name("font"))?;
        let font_size = frame.f32_values(&item_channel_column_name("font_size"))?;
        let font_weight = frame.string_values(&item_channel_column_name("font_weight"))?;
        let font_style = frame.string_values(&item_channel_column_name("font_style"))?;
        let align = frame.string_values(&item_channel_column_name("align"))?;
        let baseline = frame.string_values(&item_channel_column_name("baseline"))?;

        let mut placed_x = Vec::with_capacity(frame.len());
        let mut placed_y = Vec::with_capacity(frame.len());
        let mut defined = Vec::with_capacity(frame.len());

        for index in 0..frame.len() {
            let candidate_x = x[index] + self.dx;
            let candidate_y = y[index] + self.dy;
            let weight = parse_font_weight(&font_weight[index])?;
            let style = parse_font_style(&font_style[index])?;
            let align = parse_text_align(&align[index])?;
            let baseline = parse_text_baseline(&baseline[index])?;
            let config = TextMeasurementConfig {
                text: &text[index],
                font: &font[index],
                font_size: font_size[index],
                font_weight: weight,
                font_style: style,
            };
            let text_bounds = context.measure_text_bounds(&config);
            let [left, top] =
                text_bounds.calculate_origin([candidate_x, candidate_y], &align, &baseline);
            let bounds = GeometryBounds {
                left,
                top,
                right: left + text_bounds.width,
                bottom: top + text_bounds.height,
            };
            placed_x.push(candidate_x);
            placed_y.push(candidate_y);
            defined.push(!symbol_bounds.iter().any(|symbol| symbol.overlaps(&bounds)));
        }

        frame.set_column(
            self.x_column.clone(),
            Arc::new(Float32Array::from(placed_x)),
        )?;
        frame.set_column(
            self.y_column.clone(),
            Arc::new(Float32Array::from(placed_y)),
        )?;
        frame.set_column(
            self.defined_column.clone(),
            Arc::new(BooleanArray::from(defined)),
        )?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct KeepUprightText;

impl KeepUprightText {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Clone, Debug)]
pub struct KeepUprightTextOutput {
    angle: Expr,
}

impl KeepUprightTextOutput {
    pub fn angle(&self) -> Expr {
        self.angle.clone()
    }
}

impl MarkAdjustmentTransform for KeepUprightText {
    type Output = KeepUprightTextOutput;

    fn compile(
        self,
        ctx: MarkAdjustmentCompileContext,
    ) -> Result<(Box<dyn CompiledMarkAdjustmentTransform>, Self::Output), AvengerChartError> {
        let angle_column = ctx.output_column_name("angle");
        let output = KeepUprightTextOutput {
            angle: ctx.output_expr("angle"),
        };
        Ok((Box::new(CompiledKeepUprightText { angle_column }), output))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CompiledKeepUprightText {
    angle_column: String,
}

#[typetag::serde(name = "test_keep_upright_text")]
impl CompiledMarkAdjustmentTransform for CompiledKeepUprightText {
    fn clone_box(&self) -> Box<dyn CompiledMarkAdjustmentTransform> {
        Box::new(self.clone())
    }

    fn apply(
        &self,
        frame: &mut MarkEvaluationFrame,
        _context: &AdjustmentTransformContext<'_>,
    ) -> Result<(), AvengerChartError> {
        let angles = frame.f32_values(&item_channel_column_name("angle"))?;
        let upright = angles
            .into_iter()
            .map(|angle| {
                let normalized = angle.rem_euclid(360.0);
                if normalized > 90.0 && normalized < 270.0 {
                    angle + 180.0
                } else {
                    angle
                }
            })
            .collect::<Vec<_>>();
        frame.set_column(
            self.angle_column.clone(),
            Arc::new(Float32Array::from(upright)),
        )?;
        Ok(())
    }
}

fn symbol_bounds(base_scene: &BasePlotAreaScene) -> Vec<GeometryBounds> {
    let mut bounds = Vec::new();
    collect_symbol_bounds(base_scene.marks(), [0.0, 0.0], &mut bounds);
    bounds
}

fn collect_symbol_bounds(marks: &[SceneMark], origin: [f32; 2], bounds: &mut Vec<GeometryBounds>) {
    for mark in marks {
        match mark {
            SceneMark::Group(group) => collect_symbol_bounds(
                &group.marks,
                [origin[0] + group.origin[0], origin[1] + group.origin[1]],
                bounds,
            ),
            SceneMark::Symbol(symbol) => {
                for ((x, y), size) in symbol.x_iter().zip(symbol.y_iter()).zip(symbol.size_iter()) {
                    let half = size.sqrt() * 0.5;
                    bounds.push(GeometryBounds {
                        left: x + origin[0] - half,
                        right: x + origin[0] + half,
                        top: y + origin[1] - half,
                        bottom: y + origin[1] + half,
                    });
                }
            }
            _ => {}
        }
    }
}

fn parse_text_align(value: &str) -> Result<TextAlign, AvengerChartError> {
    match value.to_ascii_lowercase().as_str() {
        "left" => Ok(TextAlign::Left),
        "center" => Ok(TextAlign::Center),
        "right" => Ok(TextAlign::Right),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "Invalid text align '{other}' in test FixedLabelPlacement"
        ))),
    }
}

fn parse_text_baseline(value: &str) -> Result<TextBaseline, AvengerChartError> {
    match value.to_ascii_lowercase().replace('_', "-").as_str() {
        "alphabetic" => Ok(TextBaseline::Alphabetic),
        "top" => Ok(TextBaseline::Top),
        "middle" => Ok(TextBaseline::Middle),
        "bottom" => Ok(TextBaseline::Bottom),
        "line-top" => Ok(TextBaseline::LineTop),
        "line-bottom" => Ok(TextBaseline::LineBottom),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "Invalid text baseline '{other}' in test FixedLabelPlacement"
        ))),
    }
}

fn parse_font_weight(value: &str) -> Result<FontWeight, AvengerChartError> {
    match value.to_ascii_lowercase().as_str() {
        "normal" => Ok(FontWeight::Name(FontWeightNameSpec::Normal)),
        "bold" => Ok(FontWeight::Name(FontWeightNameSpec::Bold)),
        other => {
            if let Ok(value) = other.parse::<f32>() {
                Ok(FontWeight::Number(value))
            } else {
                Err(AvengerChartError::InvalidArgument(format!(
                    "Invalid font weight '{other}' in test FixedLabelPlacement"
                )))
            }
        }
    }
}

fn parse_font_style(value: &str) -> Result<FontStyle, AvengerChartError> {
    match value.to_ascii_lowercase().as_str() {
        "normal" => Ok(FontStyle::Normal),
        "italic" => Ok(FontStyle::Italic),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "Invalid font style '{other}' in test FixedLabelPlacement"
        ))),
    }
}
