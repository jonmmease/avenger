use avenger_chart::{
    plot::CompiledPlot,
    prelude::*,
    render::{EvaluatedEventDatumRows, EvaluationOptions},
};
use avenger_chart_core::AvengerChartError;
use avenger_chart_marks_statistical::BoxPlot;
use avenger_color::ColorOrGradient;
use avenger_common::types::{
    AreaOrientation, ImageAlign, ImageBaseline, SceneTextLeaderArrow, SceneTextLeaderShape,
    StrokeCap, StrokeJoin, SymbolShape,
};
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_scenegraph::marks::{
    area::SceneAreaMark, image::SceneImageMark, line::SceneLineMark, mark::SceneMark,
    path::ScenePathMark, rect::SceneRectMark, rule::SceneRuleMark, symbol::SceneSymbolMark,
    text::SceneTextMark, trail::SceneTrailMark,
};
use avenger_text::types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline};
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, BooleanArray, Float32Array, Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::{SessionContext, col, lit},
};
use std::sync::Arc;

mod mark_effects_support;
use mark_effects_support::FixedLabelPlacement;

const TINY_PNG_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAG0lEQVR4nGO4o6b2XzX59X8GscVe/3+dEf0PAE8fCXZKLiUkAAAAAElFTkSuQmCC";

#[derive(Clone)]
struct NoopAdjustment;

#[derive(Clone, Debug)]
struct NoopOutput {
    x: Expr,
    y: Expr,
}

impl NoopOutput {
    fn x(&self) -> Expr {
        self.x.clone()
    }

    fn y(&self) -> Expr {
        self.y.clone()
    }
}

impl MarkAdjustmentTransform for NoopAdjustment {
    type Output = NoopOutput;

    fn compile(
        self,
        ctx: MarkAdjustmentCompileContext,
    ) -> Result<(Box<dyn CompiledMarkAdjustmentTransform>, Self::Output), AvengerChartError> {
        let x_column = ctx.output_column_name("x");
        let y_column = ctx.output_column_name("y");
        let output = NoopOutput {
            x: ctx.output_expr("x"),
            y: ctx.output_expr("y"),
        };
        Ok((
            Box::new(CompiledNoopAdjustment { x_column, y_column }),
            output,
        ))
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CompiledNoopAdjustment {
    x_column: String,
    y_column: String,
}

#[typetag::serde(name = "test_noop_adjustment")]
impl CompiledMarkAdjustmentTransform for CompiledNoopAdjustment {
    fn clone_box(&self) -> Box<dyn CompiledMarkAdjustmentTransform> {
        Box::new(self.clone())
    }

    fn apply(
        &self,
        frame: &mut MarkEvaluationFrame,
        _context: &AdjustmentTransformContext<'_>,
    ) -> Result<(), AvengerChartError> {
        let x = frame.f32_values(&avenger_chart_core::item_channel_column_name("x"))?;
        let y = frame.f32_values(&avenger_chart_core::item_channel_column_name("y"))?;
        frame.set_column(
            self.x_column.clone(),
            Arc::new(Float32Array::from(x)) as ArrayRef,
        )?;
        frame.set_column(
            self.y_column.clone(),
            Arc::new(Float32Array::from(y)) as ArrayRef,
        )?;
        Ok(())
    }
}

#[derive(Clone)]
struct GeometryEchoAdjustment;

#[derive(Clone, Debug)]
struct GeometryEchoOutput {
    x: Expr,
    y: Expr,
    x2: Expr,
    y2: Expr,
    width: Expr,
    height: Expr,
    stroke_width: Expr,
}

impl GeometryEchoOutput {
    fn x(&self) -> Expr {
        self.x.clone()
    }

    fn y(&self) -> Expr {
        self.y.clone()
    }

    fn x2(&self) -> Expr {
        self.x2.clone()
    }

    fn y2(&self) -> Expr {
        self.y2.clone()
    }

    fn width(&self) -> Expr {
        self.width.clone()
    }

    fn height(&self) -> Expr {
        self.height.clone()
    }

    fn stroke_width(&self) -> Expr {
        self.stroke_width.clone()
    }
}

impl MarkAdjustmentTransform for GeometryEchoAdjustment {
    type Output = GeometryEchoOutput;

    fn compile(
        self,
        ctx: MarkAdjustmentCompileContext,
    ) -> Result<(Box<dyn CompiledMarkAdjustmentTransform>, Self::Output), AvengerChartError> {
        let output_columns = ["x", "y", "x2", "y2", "width", "height", "stroke_width"]
            .into_iter()
            .map(|channel| (channel.to_string(), ctx.output_column_name(channel)))
            .collect::<Vec<_>>();
        let output = GeometryEchoOutput {
            x: ctx.output_expr("x"),
            y: ctx.output_expr("y"),
            x2: ctx.output_expr("x2"),
            y2: ctx.output_expr("y2"),
            width: ctx.output_expr("width"),
            height: ctx.output_expr("height"),
            stroke_width: ctx.output_expr("stroke_width"),
        };
        Ok((
            Box::new(CompiledGeometryEchoAdjustment { output_columns }),
            output,
        ))
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CompiledGeometryEchoAdjustment {
    output_columns: Vec<(String, String)>,
}

#[typetag::serde(name = "test_geometry_echo_adjustment")]
impl CompiledMarkAdjustmentTransform for CompiledGeometryEchoAdjustment {
    fn clone_box(&self) -> Box<dyn CompiledMarkAdjustmentTransform> {
        Box::new(self.clone())
    }

    fn apply(
        &self,
        frame: &mut MarkEvaluationFrame,
        _context: &AdjustmentTransformContext<'_>,
    ) -> Result<(), AvengerChartError> {
        for (channel, output_column) in &self.output_columns {
            let item_column = avenger_chart_core::item_channel_column_name(channel);
            if frame.column(&item_column).is_none() {
                continue;
            }
            let values = frame.f32_values(&item_column)?;
            frame.set_column(
                output_column.clone(),
                Arc::new(Float32Array::from(values)) as ArrayRef,
            )?;
        }
        Ok(())
    }
}

#[derive(Clone)]
struct PlotAreaProbe {
    expected_facet_depth: usize,
}

impl PlotAreaProbe {
    fn new(expected_facet_depth: usize) -> Self {
        Self {
            expected_facet_depth,
        }
    }
}

#[derive(Clone, Debug)]
struct PlotAreaProbeOutput {
    x: Expr,
    y: Expr,
}

impl PlotAreaProbeOutput {
    fn x(&self) -> Expr {
        self.x.clone()
    }

    fn y(&self) -> Expr {
        self.y.clone()
    }
}

impl MarkAdjustmentTransform for PlotAreaProbe {
    type Output = PlotAreaProbeOutput;

    fn compile(
        self,
        ctx: MarkAdjustmentCompileContext,
    ) -> Result<(Box<dyn CompiledMarkAdjustmentTransform>, Self::Output), AvengerChartError> {
        let x_column = ctx.output_column_name("x");
        let y_column = ctx.output_column_name("y");
        let output = PlotAreaProbeOutput {
            x: ctx.output_expr("x"),
            y: ctx.output_expr("y"),
        };
        Ok((
            Box::new(CompiledPlotAreaProbe {
                x_column,
                y_column,
                expected_facet_depth: self.expected_facet_depth,
            }),
            output,
        ))
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CompiledPlotAreaProbe {
    x_column: String,
    y_column: String,
    expected_facet_depth: usize,
}

#[typetag::serde(name = "test_plot_area_probe")]
impl CompiledMarkAdjustmentTransform for CompiledPlotAreaProbe {
    fn clone_box(&self) -> Box<dyn CompiledMarkAdjustmentTransform> {
        Box::new(self.clone())
    }

    fn apply(
        &self,
        frame: &mut MarkEvaluationFrame,
        context: &AdjustmentTransformContext<'_>,
    ) -> Result<(), AvengerChartError> {
        let plot_area = context.plot_area.ok_or_else(|| {
            AvengerChartError::InvalidArgument("PlotAreaProbe expected plot-area info".to_string())
        })?;
        if plot_area.facet_path.len() != self.expected_facet_depth {
            return Err(AvengerChartError::InvalidArgument(format!(
                "PlotAreaProbe expected facet depth {}, got {}",
                self.expected_facet_depth,
                plot_area.facet_path.len()
            )));
        }
        if plot_area.width <= 0.0 || plot_area.height <= 0.0 {
            return Err(AvengerChartError::InvalidArgument(format!(
                "PlotAreaProbe expected positive plot-area size, got {}x{}",
                plot_area.width, plot_area.height
            )));
        }
        if plot_area.clip.is_none() {
            return Err(AvengerChartError::InvalidArgument(
                "PlotAreaProbe expected plot-area clip".to_string(),
            ));
        }
        if plot_area.origin != [0.0, 0.0] {
            return Err(AvengerChartError::InvalidArgument(format!(
                "PlotAreaProbe expected local origin [0, 0], got {:?}",
                plot_area.origin
            )));
        }

        let x = frame.f32_values(&avenger_chart_core::item_channel_column_name("x"))?;
        let y = frame.f32_values(&avenger_chart_core::item_channel_column_name("y"))?;
        frame.set_column(
            self.x_column.clone(),
            Arc::new(Float32Array::from(x)) as ArrayRef,
        )?;
        frame.set_column(
            self.y_column.clone(),
            Arc::new(Float32Array::from(y)) as ArrayRef,
        )?;
        Ok(())
    }
}

#[derive(Clone)]
struct ContextAvailabilityProbe {
    requirements: AdjustmentTransformRequirements,
    expected_source: bool,
    expected_base_scene: bool,
    expected_text_measurement: bool,
}

impl ContextAvailabilityProbe {
    fn new(
        requirements: AdjustmentTransformRequirements,
        expected_source: bool,
        expected_base_scene: bool,
        expected_text_measurement: bool,
    ) -> Self {
        Self {
            requirements,
            expected_source,
            expected_base_scene,
            expected_text_measurement,
        }
    }
}

impl MarkAdjustmentTransform for ContextAvailabilityProbe {
    type Output = NoopOutput;

    fn compile(
        self,
        ctx: MarkAdjustmentCompileContext,
    ) -> Result<(Box<dyn CompiledMarkAdjustmentTransform>, Self::Output), AvengerChartError> {
        let x_column = ctx.output_column_name("x");
        let y_column = ctx.output_column_name("y");
        let output = NoopOutput {
            x: ctx.output_expr("x"),
            y: ctx.output_expr("y"),
        };
        Ok((
            Box::new(CompiledContextAvailabilityProbe {
                requirements: self.requirements,
                expected_source: self.expected_source,
                expected_base_scene: self.expected_base_scene,
                expected_text_measurement: self.expected_text_measurement,
                x_column,
                y_column,
            }),
            output,
        ))
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CompiledContextAvailabilityProbe {
    requirements: AdjustmentTransformRequirements,
    expected_source: bool,
    expected_base_scene: bool,
    expected_text_measurement: bool,
    x_column: String,
    y_column: String,
}

#[typetag::serde(name = "test_context_availability_probe")]
impl CompiledMarkAdjustmentTransform for CompiledContextAvailabilityProbe {
    fn clone_box(&self) -> Box<dyn CompiledMarkAdjustmentTransform> {
        Box::new(self.clone())
    }

    fn requirements(&self) -> AdjustmentTransformRequirements {
        self.requirements.clone()
    }

    fn apply(
        &self,
        frame: &mut MarkEvaluationFrame,
        context: &AdjustmentTransformContext<'_>,
    ) -> Result<(), AvengerChartError> {
        if context.source.is_some() != self.expected_source {
            return Err(AvengerChartError::InvalidArgument(format!(
                "ContextAvailabilityProbe expected source {}, got {}",
                self.expected_source,
                context.source.is_some()
            )));
        }
        if context.base_scene.is_some() != self.expected_base_scene {
            return Err(AvengerChartError::InvalidArgument(format!(
                "ContextAvailabilityProbe expected base_scene {}, got {}",
                self.expected_base_scene,
                context.base_scene.is_some()
            )));
        }
        if context.text_measurement.is_some() != self.expected_text_measurement {
            return Err(AvengerChartError::InvalidArgument(format!(
                "ContextAvailabilityProbe expected text_measurement {}, got {}",
                self.expected_text_measurement,
                context.text_measurement.is_some()
            )));
        }

        let x = frame.f32_values(&avenger_chart_core::item_channel_column_name("x"))?;
        let y = frame.f32_values(&avenger_chart_core::item_channel_column_name("y"))?;
        frame.set_column(
            self.x_column.clone(),
            Arc::new(Float32Array::from(x)) as ArrayRef,
        )?;
        frame.set_column(
            self.y_column.clone(),
            Arc::new(Float32Array::from(y)) as ArrayRef,
        )?;
        Ok(())
    }
}

fn collect_symbols<'a>(mark: &'a SceneMark, symbols: &mut Vec<&'a SceneSymbolMark>) {
    match mark {
        SceneMark::Symbol(symbol) => symbols.push(symbol),
        SceneMark::Group(group) => {
            for child in &group.marks {
                collect_symbols(child, symbols);
            }
        }
        _ => {}
    }
}

fn collect_rules<'a>(mark: &'a SceneMark, rules: &mut Vec<&'a SceneRuleMark>) {
    match mark {
        SceneMark::Rule(rule) => rules.push(rule),
        SceneMark::Group(group) => {
            for child in &group.marks {
                collect_rules(child, rules);
            }
        }
        _ => {}
    }
}

fn collect_rects<'a>(mark: &'a SceneMark, rects: &mut Vec<&'a SceneRectMark>) {
    match mark {
        SceneMark::Rect(rect) if rect.name == "rect" => rects.push(rect),
        SceneMark::Group(group) => {
            for child in &group.marks {
                collect_rects(child, rects);
            }
        }
        _ => {}
    }
}

fn collect_texts<'a>(mark: &'a SceneMark, texts: &mut Vec<&'a SceneTextMark>) {
    match mark {
        SceneMark::Text(text) if text.name == "text" => texts.push(text),
        SceneMark::Group(group) => {
            for child in &group.marks {
                collect_texts(child, texts);
            }
        }
        _ => {}
    }
}

fn collect_images<'a>(mark: &'a SceneMark, images: &mut Vec<&'a SceneImageMark>) {
    match mark {
        SceneMark::Image(image) => images.push(image),
        SceneMark::Group(group) => {
            for child in &group.marks {
                collect_images(child, images);
            }
        }
        _ => {}
    }
}

fn collect_paths<'a>(mark: &'a SceneMark, paths: &mut Vec<&'a ScenePathMark>) {
    match mark {
        SceneMark::Path(path) if path.name == "path" => paths.push(path),
        SceneMark::Group(group) => {
            for child in &group.marks {
                collect_paths(child, paths);
            }
        }
        _ => {}
    }
}

fn collect_lines<'a>(mark: &'a SceneMark, lines: &mut Vec<&'a SceneLineMark>) {
    match mark {
        SceneMark::Line(line) if line.name == "line" => lines.push(line),
        SceneMark::Group(group) => {
            for child in &group.marks {
                collect_lines(child, lines);
            }
        }
        _ => {}
    }
}

fn collect_trails<'a>(mark: &'a SceneMark, trails: &mut Vec<&'a SceneTrailMark>) {
    match mark {
        SceneMark::Trail(trail) if trail.name == "trail" => trails.push(trail),
        SceneMark::Group(group) => {
            for child in &group.marks {
                collect_trails(child, trails);
            }
        }
        _ => {}
    }
}

fn collect_areas<'a>(mark: &'a SceneMark, areas: &mut Vec<&'a SceneAreaMark>) {
    match mark {
        SceneMark::Area(area) if area.name == "area" => areas.push(area),
        SceneMark::Group(group) => {
            for child in &group.marks {
                collect_areas(child, areas);
            }
        }
        _ => {}
    }
}

fn flattened_event_ids(rows: &[EvaluatedEventDatumRows]) -> Vec<String> {
    let mut ids = Vec::new();
    for event_rows in rows {
        let Some(values) = event_rows.rows.column_by_name("id") else {
            continue;
        };
        let values = values
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("string id datum");
        ids.extend((0..values.len()).map(|index| values.value(index).to_string()));
    }
    ids.sort();
    ids
}

fn event_datum_rows_with_id(rows: &[EvaluatedEventDatumRows]) -> usize {
    rows.iter()
        .filter(|rows| rows.rows.column_by_name("id").is_some())
        .count()
}

static PANIC_HOOK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn capture_panic_message(f: impl FnOnce() + std::panic::UnwindSafe) -> String {
    let _guard = PANIC_HOOK_LOCK.lock().expect("panic hook lock");
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let panic = std::panic::catch_unwind(f);
    std::panic::set_hook(previous_hook);
    let panic = panic.expect_err("expected panic");
    if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = panic.downcast_ref::<&str>() {
        message.to_string()
    } else {
        "<non-string panic>".to_string()
    }
}

fn compound_box_plot_outlier_batch() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("group", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec![
                "Alpha", "Alpha", "Alpha", "Alpha", "Alpha", "Alpha", "Alpha", "Beta", "Beta",
                "Beta", "Beta", "Beta", "Beta", "Beta",
            ])),
            Arc::new(Float64Array::from(vec![
                4.0, 5.0, 5.5, 6.0, 6.5, 7.0, 24.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 31.0,
            ])),
        ],
    )
    .expect("compound box plot outlier data")
}

fn roundtrip_compiled_plot(compiled: &CompiledPlot) -> Result<CompiledPlot, AvengerChartError> {
    let json = serde_json::to_string(compiled)
        .map_err(|err| AvengerChartError::SerializationError(err.to_string()))?;
    let decoded_json: CompiledPlot = serde_json::from_str(&json)
        .map_err(|err| AvengerChartError::DeserializationError(err.to_string()))?;
    let bytes = bincode::serialize(&decoded_json)
        .map_err(|err| AvengerChartError::SerializationError(err.to_string()))?;
    bincode::deserialize(&bytes)
        .map_err(|err| AvengerChartError::DeserializationError(err.to_string()))
}

fn assert_color_close(actual: &ColorOrGradient, expected: [f32; 4]) {
    let ColorOrGradient::Color(actual) = actual else {
        panic!("expected solid color, got {actual:?}");
    };
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(
            (*actual - expected).abs() < 0.001,
            "color component {actual} did not match {expected}"
        );
    }
}

async fn symbol_xy_values(
    plot: Plot<Cartesian>,
    ctx: &SessionContext,
) -> Result<(Vec<f32>, Vec<f32>), AvengerChartError> {
    let evaluated = plot.compile(ctx).await?.evaluate(ctx, None).await?;
    let mut symbols = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_symbols(mark, &mut symbols);
    }

    assert_eq!(symbols.len(), 1);
    let symbol = symbols[0];
    Ok((
        symbol.x.as_vec(symbol.len as usize, None),
        symbol.y.as_vec(symbol.len as usize, None),
    ))
}

#[tokio::test]
async fn symbol_expression_adjustment_uses_post_scale_channels() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new().unit_data().x(20.0).y(30.0).adjust(|point| {
            point
                .x(point.channel("x") + lit(7.0))
                .y(point.channel("y") - lit(4.0))
        }),
    );

    let compiled = plot.compile(&ctx).await?;
    let encoded = serde_json::to_string(&compiled)
        .map_err(|err| AvengerChartError::SerializationError(err.to_string()))?;
    let decoded: CompiledPlot = serde_json::from_str(&encoded)
        .map_err(|err| AvengerChartError::DeserializationError(err.to_string()))?;

    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut symbols = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_symbols(mark, &mut symbols);
    }

    assert_eq!(symbols.len(), 1);
    let symbol = symbols[0];
    assert_eq!(symbol.x.as_vec(symbol.len as usize, None), vec![27.0]);
    assert_eq!(symbol.y.as_vec(symbol.len as usize, None), vec![26.0]);
    Ok(())
}

#[tokio::test]
async fn symbol_expression_adjustment_can_read_size_channel() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .unit_data()
            .x(5.0)
            .y(8.0)
            .size(40.0)
            .adjust(|point| point.x(point.channel("x") + point.channel("size") / lit(2.0))),
    );

    let compiled = plot.compile(&ctx).await?;
    let encoded = serde_json::to_string(&compiled)
        .map_err(|err| AvengerChartError::SerializationError(err.to_string()))?;
    let decoded: CompiledPlot = serde_json::from_str(&encoded)
        .map_err(|err| AvengerChartError::DeserializationError(err.to_string()))?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut symbols = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_symbols(mark, &mut symbols);
    }

    assert_eq!(symbols.len(), 1);
    let symbol = symbols[0];
    assert_eq!(symbol.x.as_vec(symbol.len as usize, None), vec![25.0]);
    assert_eq!(symbol.y.as_vec(symbol.len as usize, None), vec![8.0]);
    Ok(())
}

#[tokio::test]
async fn symbol_expression_adjustment_updates_style_channels() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .unit_data()
            .x(5.0)
            .y(8.0)
            .fill("#ff0000")
            .stroke("#0000ff")
            .stroke_width(1.0)
            .shape("circle")
            .opacity(1.0)
            .adjust(|point| {
                point
                    .fill(point.channel("stroke"))
                    .stroke(point.channel("fill"))
                    .stroke_width(point.channel("stroke_width") + lit(2.0))
                    .shape(lit("square"))
                    .opacity(lit(0.25))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut symbols = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_symbols(mark, &mut symbols);
    }

    assert_eq!(symbols.len(), 1);
    let symbol = symbols[0];
    assert_eq!(symbol.stroke_width, Some(3.0));
    assert_eq!(
        symbol.shape_index.as_vec(symbol.len as usize, None),
        vec![0]
    );
    assert_eq!(symbol.shapes.len(), 1);
    assert_ne!(symbol.shapes[0], SymbolShape::Circle);
    let fill = symbol.fill.as_vec(symbol.len as usize, None);
    let stroke = symbol.stroke.as_vec(symbol.len as usize, None);
    assert_color_close(&fill[0], [0.0, 0.0, 1.0, 0.25]);
    assert_color_close(&stroke[0], [1.0, 0.0, 0.0, 0.25]);
    Ok(())
}

#[tokio::test]
async fn symbol_expression_adjustment_rejects_varying_stroke_width() -> Result<(), AvengerChartError>
{
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("width", DataType::Float32, false),
        ])),
        vec![
            Arc::new(Float64Array::from(vec![1.0, 2.0])),
            Arc::new(Float64Array::from(vec![1.0, 2.0])),
            Arc::new(Float32Array::from(vec![1.0, 3.0])),
        ],
    )?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .data(ctx.read_batch(batch)?)
            .x(col("x"))
            .y(col("y"))
            .stroke_width(1.0)
            .adjust(|point| point.stroke_width(point.data("width"))),
    );

    let compiled = plot.compile(&ctx).await?;
    let err = match compiled.evaluate(&ctx, None).await {
        Ok(_) => panic!("varying symbol stroke_width adjustment should fail"),
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("Symbol stroke_width adjustment must evaluate to one constant value"),
        "{err}"
    );
    Ok(())
}

#[tokio::test]
async fn symbol_expression_adjustment_can_read_source_data() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("dx", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["a", "b"])),
            Arc::new(Float64Array::from(vec![1.0, 2.0])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new()
        .plot_size(100.0, 100.0)
        .mark(
            Symbol::new()
                .data(df)
                .x(5.0)
                .y(8.0)
                .adjust(|point| point.x(point.channel("x") + point.data("dx"))),
        )
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(avenger_chart::event::datum("id").is_not_null()),
        );

    let compiled = plot.compile(&ctx).await?;
    assert_eq!(
        compiled.event_datum_types().get("id"),
        Some(&DataType::Utf8)
    );
    let evaluated = compiled.evaluate(&ctx, None).await?;
    let mut symbols = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_symbols(mark, &mut symbols);
    }

    assert_eq!(symbols.len(), 1);
    let symbol = symbols[0];
    assert_eq!(symbol.x.as_vec(symbol.len as usize, None), vec![6.0, 7.0]);
    assert_eq!(symbol.y.as_vec(symbol.len as usize, None), vec![8.0, 8.0]);

    assert_eq!(evaluated.event_datums.rows.len(), 1);
    assert_eq!(evaluated.event_datums.rows[0].rows.num_rows(), 2);
    let ids = evaluated.event_datums.rows[0]
        .rows
        .column_by_name("id")
        .expect("id datum")
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("string id datum");
    assert_eq!(ids.value(0), "a");
    assert_eq!(ids.value(1), "b");
    Ok(())
}

#[tokio::test]
async fn symbol_expression_adjustment_can_read_bbox_fields() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .unit_data()
            .x(10.0)
            .y(20.0)
            .size(36.0)
            .adjust(|point| {
                point
                    .x(point.bbox().right() + lit(2.0))
                    .y(point.bbox().top() - lit(1.0))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let encoded = serde_json::to_string(&compiled)
        .map_err(|err| AvengerChartError::SerializationError(err.to_string()))?;
    let decoded: CompiledPlot = serde_json::from_str(&encoded)
        .map_err(|err| AvengerChartError::DeserializationError(err.to_string()))?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut symbols = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_symbols(mark, &mut symbols);
    }

    assert_eq!(symbols.len(), 1);
    let symbol = symbols[0];
    assert_eq!(symbol.x.as_vec(symbol.len as usize, None), vec![15.0]);
    assert_eq!(symbol.y.as_vec(symbol.len as usize, None), vec![16.0]);
    Ok(())
}

#[tokio::test]
async fn symbol_transform_nudge_uses_output_handles() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .unit_data()
            .x(20.0)
            .y(30.0)
            .adjust_transform(Nudge::new(7.0, -4.0), |mark, nudge| {
                mark.x(nudge.x()).y(nudge.y())
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let encoded = serde_json::to_string(&compiled)
        .map_err(|err| AvengerChartError::SerializationError(err.to_string()))?;
    let decoded: CompiledPlot = serde_json::from_str(&encoded)
        .map_err(|err| AvengerChartError::DeserializationError(err.to_string()))?;

    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut symbols = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_symbols(mark, &mut symbols);
    }

    assert_eq!(symbols.len(), 1);
    let symbol = symbols[0];
    assert_eq!(symbol.x.as_vec(symbol.len as usize, None), vec![27.0]);
    assert_eq!(symbol.y.as_vec(symbol.len as usize, None), vec![26.0]);
    Ok(())
}

#[tokio::test]
async fn symbol_transform_accepts_custom_noop_with_default_requirements()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .unit_data()
            .x(20.0)
            .y(30.0)
            .adjust_transform(NoopAdjustment, |mark, noop| mark.x(noop.x()).y(noop.y())),
    );

    let compiled = plot.compile(&ctx).await?;
    let encoded = serde_json::to_string(&compiled)
        .map_err(|err| AvengerChartError::SerializationError(err.to_string()))?;
    let decoded: CompiledPlot = serde_json::from_str(&encoded)
        .map_err(|err| AvengerChartError::DeserializationError(err.to_string()))?;

    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut symbols = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_symbols(mark, &mut symbols);
    }

    assert_eq!(symbols.len(), 1);
    let symbol = symbols[0];
    assert_eq!(symbol.x.as_vec(symbol.len as usize, None), vec![20.0]);
    assert_eq!(symbol.y.as_vec(symbol.len as usize, None), vec![30.0]);
    Ok(())
}

#[tokio::test]
async fn symbol_transform_receives_plot_area_info() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(120.0, 90.0).mark(
        Symbol::new()
            .x(20.0)
            .y(30.0)
            .adjust_transform(PlotAreaProbe::new(0), |mark, probe| {
                mark.x(probe.x()).y(probe.y())
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut symbols = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_symbols(mark, &mut symbols);
    }

    assert_eq!(symbols.len(), 1);
    assert_eq!(
        symbols[0].x.as_vec(symbols[0].len as usize, None),
        vec![20.0]
    );
    assert_eq!(
        symbols[0].y.as_vec(symbols[0].len as usize, None),
        vec![30.0]
    );
    Ok(())
}

#[tokio::test]
async fn base_symbol_transform_receives_no_source_or_services_by_default()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(120.0, 90.0).mark(
        Symbol::new().x(20.0).y(30.0).adjust_transform(
            ContextAvailabilityProbe::new(
                AdjustmentTransformRequirements::default(),
                false,
                false,
                false,
            ),
            |mark, probe| mark.x(probe.x()).y(probe.y()),
        ),
    );

    plot.compile(&ctx).await?.evaluate(&ctx, None).await?;
    Ok(())
}

#[tokio::test]
async fn symbol_transform_receives_facet_plot_area_info() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("facet", DataType::Utf8, false),
            Field::new("x", DataType::Float32, false),
            Field::new("y", DataType::Float32, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["A"])),
            Arc::new(Float32Array::from(vec![20.0])),
            Arc::new(Float32Array::from(vec![30.0])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<FacetColumn>::new().data(df).mark(
        Subplot::new(
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x(col("x"))
                    .y(col("y"))
                    .adjust_transform(PlotAreaProbe::new(1), |mark, probe| {
                        mark.x(probe.x()).y(probe.y())
                    }),
            ),
        )
        .column(col("facet")),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut symbols = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_symbols(mark, &mut symbols);
    }

    assert_eq!(symbols.len(), 1);
    Ok(())
}

#[tokio::test]
async fn symbol_transform_jitter_is_seed_stable() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "row",
            DataType::Float32,
            false,
        )])),
        vec![Arc::new(Float32Array::from(vec![0.0, 1.0, 2.0, 3.0]))],
    )?;

    let build_plot = |seed| -> Result<Plot<Cartesian>, AvengerChartError> {
        let df = ctx.read_batch(batch.clone())?;
        Ok(Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
            Symbol::new()
                .data(df)
                .x_with(col("row"), |x| {
                    x.scale_with::<Linear>(|scale| scale.domain((0.0, 3.0)).nice(false))
                })
                .y(20.0)
                .adjust_transform(Jitter::x().width_px(10.0).seed(seed), |mark, jitter| {
                    mark.x(jitter.x())
                }),
        ))
    };
    let base = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .data(ctx.read_batch(batch.clone())?)
            .x_with(col("row"), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 3.0)).nice(false))
            })
            .y(20.0),
    );

    let (base_x, _) = symbol_xy_values(base, &ctx).await?;
    let (seed_7_x, seed_7_y) = symbol_xy_values(build_plot(7)?, &ctx).await?;
    let (seed_7_again_x, _) = symbol_xy_values(build_plot(7)?, &ctx).await?;
    let (seed_9_x, _) = symbol_xy_values(build_plot(9)?, &ctx).await?;

    assert_eq!(seed_7_x, seed_7_again_x);
    assert_ne!(seed_7_x, seed_9_x);
    assert_eq!(seed_7_y, vec![20.0, 20.0, 20.0, 20.0]);
    for (base, jittered) in base_x.iter().zip(seed_7_x) {
        let delta = jittered - base;
        assert!((-5.0..=5.0).contains(&delta), "jitter delta: {delta}");
    }
    Ok(())
}

#[tokio::test]
async fn symbol_transform_dodge_groups_by_anchor() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float32, false),
            Field::new("series", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(Float32Array::from(vec![10.0, 10.0, 30.0, 30.0])),
            Arc::new(StringArray::from(vec!["a", "b", "a", "b"])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .data(df)
            .x(col("x"))
            .y(20.0)
            .adjust_transform(Dodge::x().by("series").step_px(12.0), |mark, dodge| {
                mark.x(dodge.x())
            }),
    );

    let (x, y) = symbol_xy_values(plot, &ctx).await?;
    assert_eq!(y, vec![20.0, 20.0, 20.0, 20.0]);
    assert!((x[1] - x[0] - 12.0).abs() < 0.001);
    assert!((x[3] - x[2] - 12.0).abs() < 0.001);
    assert!((x[2] - x[0]).abs() > 12.0);
    Ok(())
}

#[tokio::test]
async fn symbol_adjustment_chaining_reads_current_frame() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float32, false),
            Field::new("series", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(Float32Array::from(vec![10.0, 10.0, 30.0, 30.0])),
            Arc::new(StringArray::from(vec!["a", "b", "a", "b"])),
        ],
    )?;

    let dodge_only = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .data(ctx.read_batch(batch.clone())?)
            .x(col("x"))
            .y(20.0)
            .adjust_transform(Dodge::x().by("series").step_px(12.0), |mark, dodge| {
                mark.x(dodge.x())
            }),
    );
    let chained = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .data(ctx.read_batch(batch)?)
            .x(col("x"))
            .y(20.0)
            .adjust_transform(Dodge::x().by("series").step_px(12.0), |mark, dodge| {
                mark.x(dodge.x())
            })
            .adjust(|point| {
                point
                    .x(point.channel("x") + lit(2.0))
                    .y(point.channel("y") - lit(3.0))
            }),
    );

    let (dodge_x, dodge_y) = symbol_xy_values(dodge_only, &ctx).await?;
    let (chained_x, chained_y) = symbol_xy_values(chained, &ctx).await?;
    assert_eq!(dodge_y, vec![20.0, 20.0, 20.0, 20.0]);
    assert_eq!(chained_y, vec![17.0, 17.0, 17.0, 17.0]);
    for (dodge, chained) in dodge_x.iter().zip(chained_x) {
        assert!((chained - dodge - 2.0).abs() < 0.001);
    }
    Ok(())
}

#[tokio::test]
async fn symbol_derive_symbol_uses_source_item_channels() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .unit_data()
            .x(20.0)
            .y(30.0)
            .size(36.0)
            .angle(10.0)
            .adjust(|point| {
                point
                    .size(point.channel("size") + lit(12.0))
                    .angle(point.channel("angle") + lit(5.0))
            })
            .derive(|point| {
                Symbol::new()
                    .x(point.channel("x"))
                    .y(point.channel("y"))
                    .size(point.channel("size") + lit(160.0))
                    .angle(point.channel("angle"))
                    .fill("#2f6fed")
                    .opacity(0.25)
                    .zindex(-1)
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let encoded = serde_json::to_string(&compiled)
        .map_err(|err| AvengerChartError::SerializationError(err.to_string()))?;
    let decoded: CompiledPlot = serde_json::from_str(&encoded)
        .map_err(|err| AvengerChartError::DeserializationError(err.to_string()))?;

    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut symbols = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_symbols(mark, &mut symbols);
    }

    assert_eq!(symbols.len(), 2);
    let base = symbols[0];
    let halo = symbols[1];
    assert_eq!(base.x.as_vec(base.len as usize, None), vec![20.0]);
    assert_eq!(base.y.as_vec(base.len as usize, None), vec![30.0]);
    assert_eq!(base.size.as_vec(base.len as usize, None), vec![48.0]);
    assert_eq!(base.angle.as_vec(base.len as usize, None), vec![15.0]);
    assert_eq!(halo.x.as_vec(halo.len as usize, None), vec![20.0]);
    assert_eq!(halo.y.as_vec(halo.len as usize, None), vec![30.0]);
    assert_eq!(halo.size.as_vec(halo.len as usize, None), vec![208.0]);
    assert_eq!(halo.angle.as_vec(halo.len as usize, None), vec![15.0]);
    assert_eq!(halo.zindex, Some(-1));
    Ok(())
}

#[tokio::test]
async fn symbol_derive_symbol_inherits_source_data_and_event_datums()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("x", DataType::Float32, false),
            Field::new("halo", DataType::Float32, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["a", "b"])),
            Arc::new(Float32Array::from(vec![10.0, 30.0])),
            Arc::new(Float32Array::from(vec![90.0, 120.0])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new()
        .plot_size(100.0, 100.0)
        .mark(Symbol::new().data(df).x(col("x")).y(20.0).derive(|point| {
            Symbol::new()
                .x(point.channel("x"))
                .y(point.channel("y"))
                .size(point.data("halo"))
        }))
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(avenger_chart::event::datum("id").is_not_null()),
        );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut symbols = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_symbols(mark, &mut symbols);
    }

    assert_eq!(symbols.len(), 2);
    let halo = symbols[1];
    assert_eq!(halo.size.as_vec(halo.len as usize, None), vec![90.0, 120.0]);
    assert_eq!(evaluated.event_datums.rows.len(), 2);
    for rows in &evaluated.event_datums.rows {
        assert_eq!(rows.rows.num_rows(), 2);
        let ids = rows
            .rows
            .column_by_name("id")
            .expect("id datum")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("string id datum");
        assert_eq!(ids.value(0), "a");
        assert_eq!(ids.value(1), "b");
    }
    Ok(())
}

#[tokio::test]
async fn rule_expression_adjustment_uses_post_scale_endpoints_source_data_and_bbox()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "dx",
            DataType::Float32,
            false,
        )])),
        vec![Arc::new(Float32Array::from(vec![5.0]))],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Rule::new()
            .data(df)
            .x(10.0)
            .y(20.0)
            .x2(30.0)
            .y2(40.0)
            .adjust(|rule| {
                rule.x(rule.channel("x") + rule.data("dx"))
                    .x2(rule.channel("x2") + rule.data("dx"))
                    .y2(rule.channel("y2") + rule.bbox().bottom() - rule.bbox().top())
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut rules = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_rules(mark, &mut rules);
    }

    assert_eq!(rules.len(), 1);
    let rule = rules[0];
    assert_eq!(rule.x.as_vec(rule.len as usize, None), vec![15.0]);
    assert_eq!(rule.y.as_vec(rule.len as usize, None), vec![20.0]);
    assert_eq!(rule.x2.as_vec(rule.len as usize, None), vec![35.0]);
    assert_eq!(rule.y2.as_vec(rule.len as usize, None), vec![60.0]);
    Ok(())
}

#[tokio::test]
async fn rule_transform_adjustment_updates_endpoint_channels() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Rule::new()
            .unit_data()
            .x(10.0)
            .y(20.0)
            .x2(30.0)
            .y2(40.0)
            .stroke_width(2.0)
            .adjust_transform(GeometryEchoAdjustment, |rule, echo| {
                rule.x(echo.x() + lit(1.0))
                    .y(echo.y() + lit(2.0))
                    .x2(echo.x2() + lit(3.0))
                    .y2(echo.y2() + lit(4.0))
                    .stroke_width(echo.stroke_width() + lit(1.5))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut rules = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_rules(mark, &mut rules);
    }

    assert_eq!(rules.len(), 1);
    let rule = rules[0];
    assert_eq!(rule.x.as_vec(rule.len as usize, None), vec![11.0]);
    assert_eq!(rule.y.as_vec(rule.len as usize, None), vec![22.0]);
    assert_eq!(rule.x2.as_vec(rule.len as usize, None), vec![33.0]);
    assert_eq!(rule.y2.as_vec(rule.len as usize, None), vec![44.0]);
    assert_eq!(rule.stroke_width.as_vec(rule.len as usize, None), vec![3.5]);
    Ok(())
}

#[tokio::test]
async fn rule_expression_adjustment_updates_stroke_width() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Rule::new()
            .unit_data()
            .x(10.0)
            .y(20.0)
            .x2(30.0)
            .y2(40.0)
            .stroke_width(1.5)
            .adjust(|rule| rule.stroke_width(rule.channel("stroke_width") + lit(2.0))),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut rules = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_rules(mark, &mut rules);
    }

    assert_eq!(rules.len(), 1);
    let rule = rules[0];
    assert_eq!(rule.stroke_width.as_vec(rule.len as usize, None), vec![3.5]);
    Ok(())
}

#[tokio::test]
async fn rule_expression_adjustment_updates_style_channels() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Rule::new()
            .unit_data()
            .x(10.0)
            .y(20.0)
            .x2(30.0)
            .y2(40.0)
            .stroke("#0000ff")
            .stroke_width(1.5)
            .stroke_dash("solid")
            .stroke_cap("butt")
            .opacity(1.0)
            .adjust(|rule| {
                rule.stroke(rule.channel("stroke"))
                    .stroke_width(rule.channel("stroke_width") + lit(2.0))
                    .stroke_dash(lit("dashed"))
                    .stroke_cap(lit("round"))
                    .opacity(lit(0.5))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut rules = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_rules(mark, &mut rules);
    }

    assert_eq!(rules.len(), 1);
    let rule = rules[0];
    assert_eq!(rule.stroke_width.as_vec(rule.len as usize, None), vec![3.5]);
    assert_eq!(
        rule.stroke_cap.as_vec(rule.len as usize, None),
        vec![StrokeCap::Round]
    );
    let stroke_dash = rule
        .stroke_dash
        .as_ref()
        .expect("adjusted rule stroke dash");
    assert_eq!(
        stroke_dash.as_vec(rule.len as usize, None),
        vec![vec![8.0, 4.0]]
    );
    let stroke = rule.stroke.as_vec(rule.len as usize, None);
    assert_color_close(&stroke[0], [0.0, 0.0, 1.0, 0.5]);
    Ok(())
}

#[tokio::test]
async fn symbol_derive_rule_consumes_adjusted_source_position() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .unit_data()
            .x(20.0)
            .y(30.0)
            .adjust(|point| point.x(point.channel("x") + lit(4.0)))
            .derive(|point| {
                Rule::new()
                    .x(point.channel("x"))
                    .x2(point.channel("x"))
                    .y(point.channel("y"))
                    .y2(point.channel("y") + lit(16.0))
                    .stroke("#4b5563")
                    .stroke_width(2.0)
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let encoded = serde_json::to_string(&compiled)
        .map_err(|err| AvengerChartError::SerializationError(err.to_string()))?;
    let decoded: CompiledPlot = serde_json::from_str(&encoded)
        .map_err(|err| AvengerChartError::DeserializationError(err.to_string()))?;

    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut symbols = Vec::new();
    let mut rules = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_symbols(mark, &mut symbols);
        collect_rules(mark, &mut rules);
    }

    assert_eq!(symbols.len(), 1);
    assert_eq!(rules.len(), 1);
    let symbol = symbols[0];
    let rule = rules[0];
    assert_eq!(symbol.x.as_vec(symbol.len as usize, None), vec![24.0]);
    assert_eq!(symbol.y.as_vec(symbol.len as usize, None), vec![30.0]);
    assert_eq!(rule.x.as_vec(rule.len as usize, None), vec![24.0]);
    assert_eq!(rule.x2.as_vec(rule.len as usize, None), vec![24.0]);
    assert_eq!(rule.y.as_vec(rule.len as usize, None), vec![30.0]);
    assert_eq!(rule.y2.as_vec(rule.len as usize, None), vec![46.0]);
    Ok(())
}

#[tokio::test]
async fn symbol_derive_text_uses_post_scale_channels_and_source_data()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("x", DataType::Float32, false),
            Field::new("label", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["a", "b"])),
            Arc::new(Float32Array::from(vec![25.0, 50.0])),
            Arc::new(StringArray::from(vec!["Alpha", "Beta"])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new()
        .plot_size(100.0, 100.0)
        .mark(
            Symbol::new()
                .data(df)
                .x_with(col("x"), |x| {
                    x.scale_with::<Linear>(|scale| scale.domain((0.0, 50.0)).nice(false))
                })
                .y(20.0)
                .derive(|point| {
                    Text::new()
                        .x(point.channel("x"))
                        .y(point.channel("y"))
                        .text(point.data("label"))
                        .align("center")
                        .baseline("middle")
                }),
        )
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(avenger_chart::event::datum("id").is_not_null()),
        );

    let compiled = plot.compile(&ctx).await?;
    let encoded = serde_json::to_string(&compiled)
        .map_err(|err| AvengerChartError::SerializationError(err.to_string()))?;
    let decoded: CompiledPlot = serde_json::from_str(&encoded)
        .map_err(|err| AvengerChartError::DeserializationError(err.to_string()))?;

    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut symbols = Vec::new();
    let mut texts = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_symbols(mark, &mut symbols);
        collect_texts(mark, &mut texts);
    }

    assert_eq!(symbols.len(), 1);
    assert_eq!(texts.len(), 1);
    let symbol = symbols[0];
    let text = texts[0];
    let symbol_x = symbol.x.as_vec(symbol.len as usize, None);
    let text_x = text.x.as_vec(text.len as usize, None);
    assert_eq!(text_x, symbol_x);
    assert_ne!(text_x[0], 25.0, "derived text x was scaled a second time");
    assert_eq!(text.y.as_vec(text.len as usize, None), vec![20.0, 20.0]);
    assert_eq!(
        text.text.as_vec(text.len as usize, None),
        vec!["Alpha".to_string(), "Beta".to_string()]
    );
    assert_eq!(evaluated.event_datums.rows.len(), 2);
    for rows in &evaluated.event_datums.rows {
        assert_eq!(rows.rows.num_rows(), 2);
        let ids = rows
            .rows
            .column_by_name("id")
            .expect("id datum")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("string id datum");
        assert_eq!(ids.value(0), "a");
        assert_eq!(ids.value(1), "b");
    }
    Ok(())
}

#[tokio::test]
async fn symbol_derived_text_fixed_label_placement_hides_point_overlap()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("x", DataType::Float32, false),
            Field::new("label", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["a", "b"])),
            Arc::new(Float32Array::from(vec![20.0, 50.0])),
            Arc::new(StringArray::from(vec!["AlphaLabel", "Beta"])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new()
        .plot_size(100.0, 80.0)
        .mark(
            Symbol::new()
                .data(df)
                .x_with(col("x"), |x| {
                    x.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
                })
                .y(40.0)
                .size(64.0)
                .derive(|point| {
                    Text::new()
                        .x(point.channel("x"))
                        .y(point.channel("y"))
                        .text(point.data("label"))
                        .align("center")
                        .baseline("middle")
                        .font_size(10.0)
                        .adjust_transform(FixedLabelPlacement::new(20.0, 0.0), |text, placed| {
                            text.x(placed.x()).y(placed.y()).defined(placed.defined())
                        })
                }),
        )
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(avenger_chart::event::datum("id").is_not_null()),
        );

    let compiled = plot.compile(&ctx).await?;
    let encoded = serde_json::to_string(&compiled)
        .map_err(|err| AvengerChartError::SerializationError(err.to_string()))?;
    let decoded: CompiledPlot = serde_json::from_str(&encoded)
        .map_err(|err| AvengerChartError::DeserializationError(err.to_string()))?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut symbols = Vec::new();
    let mut texts = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_symbols(mark, &mut symbols);
        collect_texts(mark, &mut texts);
    }

    assert_eq!(symbols.len(), 1);
    assert_eq!(texts.len(), 1);
    let text = texts[0];
    assert_eq!(text.x.as_vec(text.len as usize, None), vec![40.0, 70.0]);
    assert_eq!(text.y.as_vec(text.len as usize, None), vec![40.0, 40.0]);
    assert_eq!(
        text.defined.as_vec(text.len as usize, None),
        vec![false, true]
    );
    let text_geometries = text.geometry_iter(vec![1], [0.0, 0.0]).collect::<Vec<_>>();
    assert_eq!(text_geometries.len(), 1);
    assert_eq!(text_geometries[0].mark_instance.instance_index, Some(1));
    assert_eq!(evaluated.event_datums.rows.len(), 2);
    let text_event_rows = evaluated
        .event_datums
        .rows
        .iter()
        .find(|rows| rows.mark_path == vec![0, 1, 1])
        .expect("derived text event datum rows");
    assert_eq!(text_event_rows.rows.num_rows(), 2);
    let ids = text_event_rows
        .rows
        .column_by_name("id")
        .expect("id datum")
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("string id datum");
    assert_eq!(ids.value(0), "a");
    assert_eq!(ids.value(1), "b");
    Ok(())
}

#[tokio::test]
async fn fixed_label_placement_reuses_text_measurement_cache() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new("x", DataType::Float32, false)])),
        vec![Arc::new(Float32Array::from(vec![20.0, 60.0]))],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 80.0).mark(
        Symbol::new()
            .data(df)
            .x_with(col("x"), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .y(40.0)
            .size(16.0)
            .derive(|point| {
                Text::new()
                    .x(point.channel("x"))
                    .y(point.channel("y"))
                    .text("Repeated")
                    .font_size(10.0)
                    .adjust_transform(FixedLabelPlacement::new(18.0, 0.0), |text, placed| {
                        text.x(placed.x()).y(placed.y()).defined(placed.defined())
                    })
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let (_evaluated, metrics) = compiled
        .evaluate_with_options_and_metrics(&ctx, None, EvaluationOptions::default())
        .await?;
    assert!(
        metrics.pipeline.text_measurement_cache_misses > 0,
        "initial fixed-label measurement should populate the text cache"
    );
    assert!(
        metrics.pipeline.text_measurement_cache_hits > 0,
        "repeated fixed-label text should reuse the text cache"
    );
    Ok(())
}

#[tokio::test]
async fn derived_text_transform_receives_only_requested_context() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .plot_size(100.0, 80.0)
        .mark(
            Symbol::new()
                .unit_data()
                .x(20.0)
                .y(40.0)
                .size(36.0)
                .derive(|point| {
                    Text::new()
                        .x(point.channel("x"))
                        .y(point.channel("y"))
                        .text("fixed")
                        .font_size(10.0)
                        .adjust_transform(FixedLabelPlacement::new(18.0, 0.0), |text, placed| {
                            text.x(placed.x()).y(placed.y()).defined(placed.defined())
                        })
                }),
        )
        .mark(
            Symbol::new()
                .unit_data()
                .x(70.0)
                .y(40.0)
                .size(36.0)
                .derive(|point| {
                    Text::new()
                        .x(point.channel("x"))
                        .y(point.channel("y"))
                        .text("probe")
                        .font_size(10.0)
                        .adjust_transform(
                            ContextAvailabilityProbe::new(
                                AdjustmentTransformRequirements::default(),
                                false,
                                false,
                                false,
                            ),
                            |text, probe| text.x(probe.x()).y(probe.y()),
                        )
                }),
        );

    plot.compile(&ctx).await?.evaluate(&ctx, None).await?;
    Ok(())
}

#[tokio::test]
async fn derived_text_transform_receives_requested_source_frame() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 80.0).mark(
        Symbol::new().unit_data().x(20.0).y(40.0).derive(|point| {
            Text::new()
                .x(point.channel("x"))
                .y(point.channel("y"))
                .text("probe")
                .font_size(10.0)
                .adjust_transform(
                    ContextAvailabilityProbe::new(
                        AdjustmentTransformRequirements::default().with_source_frame(),
                        true,
                        false,
                        false,
                    ),
                    |text, probe| text.x(probe.x()).y(probe.y()),
                )
        }),
    );

    plot.compile(&ctx).await?.evaluate(&ctx, None).await?;
    Ok(())
}

#[tokio::test]
async fn symbol_derived_text_fixed_label_placement_uses_facet_local_base_scene()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("facet", DataType::Utf8, false),
            Field::new("x", DataType::Float32, false),
            Field::new("y", DataType::Float32, false),
            Field::new("label", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["A", "B"])),
            Arc::new(Float32Array::from(vec![20.0, 40.0])),
            Arc::new(Float32Array::from(vec![55.0, 55.0])),
            Arc::new(StringArray::from(vec!["A", "B"])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;
    let child = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(col("x"), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .y_with(col("y"), |y| {
                y.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .size(72.0)
            .derive(|point| {
                Text::new()
                    .x(point.channel("x"))
                    .y(point.channel("y"))
                    .text(point.data("label"))
                    .align("center")
                    .baseline("middle")
                    .font_size(10.0)
                    .adjust_transform(FixedLabelPlacement::new(34.0, 0.0), |text, placed| {
                        text.x(placed.x()).y(placed.y()).defined(placed.defined())
                    })
            }),
    );
    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .plot_size(220.0, 150.0)
        .mark(Subplot::new(child).column(col("facet")));

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut texts = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_texts(mark, &mut texts);
    }

    let defined = texts
        .iter()
        .flat_map(|text| text.defined.as_vec(text.len as usize, None))
        .collect::<Vec<_>>();
    assert_eq!(defined, vec![true, true]);
    Ok(())
}

#[tokio::test]
async fn derived_marks_do_not_affect_source_scale_domain() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float32, false),
            Field::new("y", DataType::Float32, false),
        ])),
        vec![
            Arc::new(Float32Array::from(vec![0.0, 10.0])),
            Arc::new(Float32Array::from(vec![20.0, 80.0])),
        ],
    )?;

    let base_plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .data(ctx.read_batch(batch.clone())?)
            .x(col("x"))
            .y(col("y")),
    );
    let derived_plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .data(ctx.read_batch(batch)?)
            .x(col("x"))
            .y(col("y"))
            .derive(|point| {
                Symbol::new()
                    .x(point.channel("x") + lit(10_000.0))
                    .y(point.channel("y"))
                    .size(12.0)
            }),
    );

    let (base_x, base_y) = symbol_xy_values(base_plot, &ctx).await?;
    let evaluated = derived_plot
        .compile(&ctx)
        .await?
        .evaluate(&ctx, None)
        .await?;
    let mut symbols = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_symbols(mark, &mut symbols);
    }

    assert_eq!(symbols.len(), 2);
    assert_eq!(symbols[0].x.as_vec(symbols[0].len as usize, None), base_x);
    assert_eq!(symbols[0].y.as_vec(symbols[0].len as usize, None), base_y);
    assert_eq!(
        symbols[1].x.as_vec(symbols[1].len as usize, None),
        base_x
            .iter()
            .map(|value| value + 10_000.0)
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[tokio::test]
async fn compound_box_plot_outlier_halo_inherits_identity_and_excludes_scale_domain()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = compound_box_plot_outlier_batch();
    let base_plot = Plot::<Cartesian>::new().plot_size(240.0, 160.0).mark(
        BoxPlot::new()
            .data(ctx.read_batch(batch.clone())?)
            .x(col("value"))
            .y(col("group")),
    );
    let halo_plot = Plot::<Cartesian>::new()
        .plot_size(240.0, 160.0)
        .mark(
            BoxPlot::new()
                .id("compound")
                .data(ctx.read_batch(batch)?)
                .x(col("value"))
                .y(col("group"))
                .outliers(|outliers| {
                    outliers
                        .adjust(|point| point.x(point.channel("x") + lit(3.0)))
                        .adjust_transform(Nudge::new(2.0, 0.0), |point, nudge| {
                            point.x(nudge.x()).y(nudge.y())
                        })
                        .derive(|point| {
                            Symbol::new()
                                .x(point.channel("x") + lit(10_000.0))
                                .y(point.channel("y"))
                                .size(point.channel("size") * lit(2.0))
                                .fill("#facc15")
                        })
                }),
        )
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(avenger_chart::event::datum("value").is_not_null()),
        );

    let base_evaluated = base_plot.compile(&ctx).await?.evaluate(&ctx, None).await?;
    let mut base_symbols = Vec::new();
    for mark in base_evaluated.scene_graph.children() {
        collect_symbols(mark, &mut base_symbols);
    }
    assert_eq!(base_symbols.len(), 1);
    let base_outlier_x = base_symbols[0].x.as_vec(base_symbols[0].len as usize, None);

    let halo_compiled = halo_plot.compile(&ctx).await?;
    let halo_decoded = roundtrip_compiled_plot(&halo_compiled)?;
    let halo_evaluated = halo_decoded.evaluate(&ctx, None).await?;
    let mut halo_symbols = Vec::new();
    for mark in halo_evaluated.scene_graph.children() {
        collect_symbols(mark, &mut halo_symbols);
    }
    assert_eq!(halo_symbols.len(), 2);
    assert_eq!(
        halo_symbols[0].x.as_vec(halo_symbols[0].len as usize, None),
        base_outlier_x
            .iter()
            .map(|value| value + 5.0)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        halo_symbols[1].x.as_vec(halo_symbols[1].len as usize, None),
        base_outlier_x
            .iter()
            .map(|value| value + 10_005.0)
            .collect::<Vec<_>>()
    );

    let outlier_event_rows = halo_evaluated
        .event_datums
        .rows
        .iter()
        .filter(|rows| rows.rows.column_by_name("value").is_some())
        .collect::<Vec<_>>();
    assert_eq!(outlier_event_rows.len(), 2);
    for rows in outlier_event_rows {
        assert_eq!(rows.rows.num_rows(), 2);
        let values = rows
            .rows
            .column_by_name("value")
            .expect("outlier values")
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("float64 outlier values");
        let mut values = (0..values.len())
            .map(|index| values.value(index))
            .collect::<Vec<_>>();
        values.sort_by(|a, b| a.partial_cmp(b).expect("finite outlier values"));
        assert_eq!(values, vec![24.0, 31.0]);
    }

    Ok(())
}

#[tokio::test]
async fn compound_box_plot_generated_children_keep_public_target_paths()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = compound_box_plot_outlier_batch();
    let plot = Plot::<Cartesian>::new().plot_size(240.0, 160.0).mark(
        BoxPlot::new()
            .id("box_plot")
            .data(ctx.read_batch(batch)?)
            .x(col("value"))
            .y(col("group")),
    );

    let compiled = plot.compile(&ctx).await?;
    let mut paths = compiled
        .marks()
        .iter()
        .filter_map(|mark| mark.state().public_target_path.clone())
        .collect::<Vec<_>>();
    paths.sort();

    assert_eq!(
        paths,
        vec![
            "box_plot.box",
            "box_plot.lower_cap",
            "box_plot.median",
            "box_plot.outliers",
            "box_plot.upper_cap",
            "box_plot.whiskers",
        ]
    );

    Ok(())
}

#[test]
fn derived_text_rejects_ordinary_data_refs() {
    let message = capture_panic_message(|| {
        let _ = Symbol::<Cartesian>::new()
            .unit_data()
            .y(10.0)
            .derive(|point| Text::new().x(col("x")).y(point.channel("y")).text("bad"));
    });
    assert!(
        message.contains("Derived Text channel 'x' referenced ordinary data column 'x'"),
        "unexpected panic: {message}"
    );
}

#[test]
fn derived_mark_rejects_mark_local_data() {
    let message = capture_panic_message(|| {
        let ctx = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("x", DataType::Float32, false)])),
            vec![Arc::new(Float32Array::from(vec![1.0]))],
        )
        .expect("derived output local data batch");
        let df = ctx.read_batch(batch).expect("derived output local data");
        let _ = Symbol::<Cartesian>::new()
            .unit_data()
            .x(10.0)
            .derive(|point| Symbol::new().data(df).x(point.channel("x")));
    });
    assert!(
        message
            .contains("Derived Symbol marks cannot declare mark-local data or data transforms yet"),
        "unexpected panic: {message}"
    );
}

#[tokio::test]
async fn adjustment_unknown_item_channel_errors_clearly() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .unit_data()
            .x(10.0)
            .y(20.0)
            .adjust(|point| point.x(point.channel("missing") + lit(1.0))),
    );

    let err = match plot.compile(&ctx).await?.evaluate(&ctx, None).await {
        Ok(_) => {
            return Err(AvengerChartError::InternalError(
                "unknown adjustment item channel unexpectedly succeeded".to_string(),
            ));
        }
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("Unknown item-frame expression column"),
        "unexpected error: {err}"
    );
    assert!(
        err.to_string().contains("item.channel(\"missing\")"),
        "unexpected error: {err}"
    );
    Ok(())
}

#[tokio::test]
async fn derived_unknown_item_channel_errors_clearly() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new().unit_data().x(10.0).y(20.0).derive(|point| {
            Symbol::new()
                .x(point.channel("missing"))
                .y(point.channel("y"))
        }),
    );

    let err = match plot.compile(&ctx).await?.evaluate(&ctx, None).await {
        Ok(_) => {
            return Err(AvengerChartError::InternalError(
                "unknown derived item channel unexpectedly succeeded".to_string(),
            ));
        }
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("Unknown item-frame expression column"),
        "unexpected error: {err}"
    );
    assert!(
        err.to_string().contains("item.channel(\"missing\")"),
        "unexpected error: {err}"
    );
    Ok(())
}

#[test]
fn recursive_derived_symbol_rejected_with_clear_message() {
    let message = capture_panic_message(|| {
        let _ = Symbol::<Cartesian>::new()
            .unit_data()
            .x(10.0)
            .derive(|point| {
                Symbol::new()
                    .x(point.channel("x"))
                    .derive(|nested| Symbol::new().x(nested.channel("x")))
            });
    });
    assert!(
        message.contains("Nested derived Symbol effects are not implemented yet"),
        "unexpected panic: {message}"
    );
}

#[test]
fn recursive_derived_rect_rejected_with_clear_message() {
    let message = capture_panic_message(|| {
        let _ = Rect::<Cartesian>::new()
            .unit_data()
            .x(10.0)
            .x2(20.0)
            .y(10.0)
            .y2(20.0)
            .derive(|rect| {
                Rect::new()
                    .x(rect.channel("x"))
                    .x2(rect.channel("x2"))
                    .y(rect.channel("y"))
                    .y2(rect.channel("y2"))
                    .derive(|nested| Rect::new().x(nested.channel("x")))
            });
    });
    assert!(
        message.contains("Nested derived Rect effects are not implemented yet"),
        "unexpected panic: {message}"
    );
}

#[tokio::test]
async fn rect_expression_adjustment_uses_post_scale_corners_source_data_and_bbox()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "dx",
            DataType::Float32,
            false,
        )])),
        vec![Arc::new(Float32Array::from(vec![5.0]))],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Rect::new()
            .data(df)
            .x(10.0)
            .x2(30.0)
            .y(20.0)
            .y2(50.0)
            .adjust(|rect| {
                rect.x(rect.channel("x") + rect.data("dx"))
                    .x2(rect.channel("x2") + rect.data("dx"))
                    .y2(rect.channel("y2") + rect.bbox().bottom() - rect.bbox().top())
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut rects = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_rects(mark, &mut rects);
    }

    assert_eq!(rects.len(), 1);
    let rect = rects[0];
    assert_eq!(rect.x.as_vec(rect.len as usize, None), vec![15.0]);
    assert_eq!(
        rect.x2.as_ref().unwrap().as_vec(rect.len as usize, None),
        vec![35.0]
    );
    assert_eq!(rect.y.as_vec(rect.len as usize, None), vec![20.0]);
    assert_eq!(
        rect.y2.as_ref().unwrap().as_vec(rect.len as usize, None),
        vec![80.0]
    );
    Ok(())
}

#[tokio::test]
async fn rect_transform_adjustment_updates_corner_channels() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Rect::new()
            .unit_data()
            .x(10.0)
            .x2(30.0)
            .y(20.0)
            .y2(50.0)
            .adjust_transform(GeometryEchoAdjustment, |rect, echo| {
                rect.x(echo.x() + lit(1.0))
                    .y(echo.y() + lit(2.0))
                    .x2(echo.x2() + lit(3.0))
                    .y2(echo.y2() + lit(4.0))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut rects = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_rects(mark, &mut rects);
    }

    assert_eq!(rects.len(), 1);
    let rect = rects[0];
    assert_eq!(rect.x.as_vec(rect.len as usize, None), vec![11.0]);
    assert_eq!(rect.y.as_vec(rect.len as usize, None), vec![22.0]);
    assert_eq!(
        rect.x2.as_ref().unwrap().as_vec(rect.len as usize, None),
        vec![33.0]
    );
    assert_eq!(
        rect.y2.as_ref().unwrap().as_vec(rect.len as usize, None),
        vec![54.0]
    );
    Ok(())
}

#[tokio::test]
async fn rect_derive_consumes_adjusted_source_geometry() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Rect::new()
            .unit_data()
            .x(10.0)
            .x2(30.0)
            .y(20.0)
            .y2(50.0)
            .adjust(|rect| {
                rect.x(rect.channel("x") + lit(5.0))
                    .x2(rect.channel("x2") + lit(5.0))
            })
            .derive(|rect| {
                Rect::new()
                    .x(rect.bbox().left() - lit(2.0))
                    .x2(rect.bbox().right() + lit(2.0))
                    .y(rect.bbox().top() - lit(2.0))
                    .y2(rect.bbox().bottom() + lit(2.0))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut rects = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_rects(mark, &mut rects);
    }

    assert_eq!(rects.len(), 2);
    let outline = rects[1];
    assert_eq!(outline.x.as_vec(outline.len as usize, None), vec![13.0]);
    assert_eq!(
        outline
            .x2
            .as_ref()
            .unwrap()
            .as_vec(outline.len as usize, None),
        vec![37.0]
    );
    Ok(())
}

#[tokio::test]
async fn rect_expression_adjustment_updates_corner_radius() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Rect::new()
            .unit_data()
            .x(10.0)
            .x2(30.0)
            .y(20.0)
            .y2(50.0)
            .corner_radius(2.0)
            .adjust(|rect| rect.corner_radius(rect.channel("corner_radius") + lit(4.0))),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut rects = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_rects(mark, &mut rects);
    }

    assert_eq!(rects.len(), 1);
    let rect = rects[0];
    assert_eq!(
        rect.corner_radius.as_vec(rect.len as usize, None),
        vec![6.0]
    );
    Ok(())
}

#[tokio::test]
async fn rect_expression_adjustment_updates_style_channels() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Rect::new()
            .unit_data()
            .x(10.0)
            .x2(30.0)
            .y(20.0)
            .y2(50.0)
            .fill("#ff0000")
            .stroke("#0000ff")
            .stroke_width(1.5)
            .opacity(1.0)
            .adjust(|rect| {
                rect.fill(rect.channel("stroke"))
                    .stroke(rect.channel("fill"))
                    .stroke_width(rect.channel("stroke_width") + lit(2.0))
                    .opacity(lit(0.25))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut rects = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_rects(mark, &mut rects);
    }

    assert_eq!(rects.len(), 1);
    let rect = rects[0];
    assert_eq!(rect.stroke_width.as_vec(rect.len as usize, None), vec![3.5]);
    let fill = rect.fill.as_vec(rect.len as usize, None);
    let stroke = rect.stroke.as_vec(rect.len as usize, None);
    assert_color_close(&fill[0], [0.0, 0.0, 1.0, 0.25]);
    assert_color_close(&stroke[0], [1.0, 0.0, 0.0, 0.25]);
    Ok(())
}

#[tokio::test]
async fn rect_derive_rect_outline_uses_bbox_fields() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Rect::new()
            .unit_data()
            .x(10.0)
            .x2(30.0)
            .y(20.0)
            .y2(50.0)
            .corner_radius(3.0)
            .adjust(|rect| rect.corner_radius(rect.channel("corner_radius") + lit(2.0)))
            .derive(|rect| {
                Rect::new()
                    .x(rect.bbox().left() - lit(2.0))
                    .x2(rect.bbox().right() + lit(2.0))
                    .y(rect.bbox().top() - lit(2.0))
                    .y2(rect.bbox().bottom() + lit(2.0))
                    .corner_radius(rect.channel("corner_radius") + lit(1.0))
                    .fill("transparent")
                    .stroke("#111827")
                    .stroke_width(2.0)
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let encoded = serde_json::to_string(&compiled)
        .map_err(|err| AvengerChartError::SerializationError(err.to_string()))?;
    let decoded: CompiledPlot = serde_json::from_str(&encoded)
        .map_err(|err| AvengerChartError::DeserializationError(err.to_string()))?;

    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut rects = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_rects(mark, &mut rects);
    }

    assert_eq!(rects.len(), 2);
    let base = rects[0];
    let outline = rects[1];
    assert_eq!(base.x.as_vec(base.len as usize, None), vec![10.0]);
    assert_eq!(
        base.x2.as_ref().unwrap().as_vec(base.len as usize, None),
        vec![30.0]
    );
    assert_eq!(base.y.as_vec(base.len as usize, None), vec![20.0]);
    assert_eq!(
        base.y2.as_ref().unwrap().as_vec(base.len as usize, None),
        vec![50.0]
    );
    assert_eq!(
        base.corner_radius.as_vec(base.len as usize, None),
        vec![5.0]
    );
    assert_eq!(outline.x.as_vec(outline.len as usize, None), vec![8.0]);
    assert_eq!(
        outline
            .x2
            .as_ref()
            .unwrap()
            .as_vec(outline.len as usize, None),
        vec![32.0]
    );
    assert_eq!(outline.y.as_vec(outline.len as usize, None), vec![18.0]);
    assert_eq!(
        outline
            .y2
            .as_ref()
            .unwrap()
            .as_vec(outline.len as usize, None),
        vec![52.0]
    );
    assert_eq!(
        outline.corner_radius.as_vec(outline.len as usize, None),
        vec![6.0]
    );
    Ok(())
}

#[tokio::test]
async fn text_expression_adjustment_updates_position_defined_text_and_source_data()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("shift", DataType::Float32, false),
            Field::new("label", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(Float32Array::from(vec![1.0, 2.0])),
            Arc::new(StringArray::from(vec!["a", "b"])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Text::new()
            .data(df)
            .x(10.0)
            .y(20.0)
            .text("old")
            .align("left")
            .baseline("top")
            .angle(5.0)
            .font("serif")
            .font_size(10.0)
            .font_weight("normal")
            .font_style("normal")
            .limit(20.0)
            .adjust(|text| {
                text.x(text.channel("x") + lit(1.0))
                    .y(text.channel("y") + lit(2.0))
            })
            .adjust(|text| {
                text.x(text.channel("x") + text.data("shift"))
                    .y(text.channel("y") + lit(3.0))
                    .align(lit("right"))
                    .baseline(lit("bottom"))
                    .angle(text.channel("angle") + lit(15.0))
                    .font(lit("monospace"))
                    .font_size(text.channel("font_size") + lit(2.0))
                    .font_weight(lit("bold"))
                    .font_style(lit("italic"))
                    .limit(text.channel("limit") + lit(5.0))
                    .defined(lit(false))
                    .text(text.data("label"))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut texts = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_texts(mark, &mut texts);
    }

    assert_eq!(texts.len(), 1);
    let text = texts[0];
    assert_eq!(text.x.as_vec(text.len as usize, None), vec![12.0, 13.0]);
    assert_eq!(text.y.as_vec(text.len as usize, None), vec![25.0, 25.0]);
    assert_eq!(text.dx.as_vec(text.len as usize, None), vec![0.0, 0.0]);
    assert_eq!(text.dy.as_vec(text.len as usize, None), vec![0.0, 0.0]);
    assert_eq!(
        text.align.as_vec(text.len as usize, None),
        vec![TextAlign::Right, TextAlign::Right]
    );
    assert_eq!(
        text.baseline.as_vec(text.len as usize, None),
        vec![TextBaseline::Bottom, TextBaseline::Bottom]
    );
    assert_eq!(text.angle.as_vec(text.len as usize, None), vec![20.0, 20.0]);
    assert_eq!(
        text.font.as_vec(text.len as usize, None),
        vec!["monospace".to_string(), "monospace".to_string()]
    );
    assert_eq!(
        text.font_size.as_vec(text.len as usize, None),
        vec![12.0, 12.0]
    );
    assert_eq!(
        text.font_weight.as_vec(text.len as usize, None),
        vec![
            FontWeight::Name(FontWeightNameSpec::Bold),
            FontWeight::Name(FontWeightNameSpec::Bold)
        ]
    );
    assert_eq!(
        text.font_style.as_vec(text.len as usize, None),
        vec![FontStyle::Italic, FontStyle::Italic]
    );
    assert_eq!(text.limit.as_vec(text.len as usize, None), vec![25.0, 25.0]);
    assert_eq!(
        text.defined.as_vec(text.len as usize, None),
        vec![false, false]
    );
    assert_eq!(
        text.text.as_vec(text.len as usize, None),
        vec!["a".to_string(), "b".to_string()]
    );
    Ok(())
}

#[tokio::test]
async fn text_expression_adjustment_updates_color_opacity_leader_and_bbox_channels()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Text::new()
            .unit_data()
            .x(10.0)
            .y(20.0)
            .text("label")
            .color("#111111")
            .opacity(0.8)
            .leader(false)
            .leader_offset_x(1.0)
            .leader_offset_y(2.0)
            .leader_stroke("#222222")
            .leader_stroke_width(1.0)
            .leader_stroke_dash("solid")
            .leader_stroke_cap("round")
            .leader_stroke_join("round")
            .leader_label_padding(2.0)
            .leader_target_radius(0.0)
            .leader_min_length(1.0)
            .leader_shape("straight")
            .leader_arrow("none")
            .leader_arrow_length(6.0)
            .leader_arrow_width(5.0)
            .adjust(|text| {
                text.x(text.channel("x") + lit(3.0))
                    .y(text.channel("y") + lit(4.0))
                    .leader_offset_x(lit(3.0))
                    .leader_offset_y(lit(4.0))
                    .color(lit("#ff0000"))
            })
            .adjust(|text| {
                text.x(text.bbox().left() + lit(1.0))
                    .y(text.bbox().top() + lit(2.0))
                    .leader_offset_x(lit(5.0))
                    .leader_offset_y(lit(6.0))
                    .opacity(text.channel("opacity") * lit(0.5))
                    .leader(lit(true))
                    .leader_stroke(text.channel("color"))
                    .leader_stroke_width(lit(2.5))
                    .leader_stroke_dash(lit("dashed"))
                    .leader_stroke_cap(lit("square"))
                    .leader_stroke_join(lit("bevel"))
                    .leader_label_padding(lit(6.0))
                    .leader_target_radius(lit(4.0))
                    .leader_min_length(lit(9.0))
                    .leader_shape(lit("elbow"))
                    .leader_arrow(lit("triangle"))
                    .leader_arrow_length(lit(7.0))
                    .leader_arrow_width(lit(8.0))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut texts = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_texts(mark, &mut texts);
    }

    assert_eq!(texts.len(), 1);
    let text = texts[0];
    assert_eq!(text.x.as_vec(text.len as usize, None), vec![17.0]);
    assert_eq!(text.y.as_vec(text.len as usize, None), vec![30.0]);
    assert_eq!(text.dx.as_vec(text.len as usize, None), vec![5.0]);
    assert_eq!(text.dy.as_vec(text.len as usize, None), vec![6.0]);
    assert_color_close(
        &text.color.as_vec(text.len as usize, None)[0],
        [1.0, 0.0, 0.0, 0.4],
    );
    assert_eq!(text.opacity.as_vec(text.len as usize, None), vec![0.4]);
    assert_eq!(text.leader.as_vec(text.len as usize, None), vec![true]);
    assert_color_close(
        &text.leader_stroke.as_vec(text.len as usize, None)[0],
        [1.0, 0.0, 0.0, 0.4],
    );
    assert_eq!(
        text.leader_stroke_width.as_vec(text.len as usize, None),
        vec![2.5]
    );
    assert_eq!(
        text.leader_stroke_dash
            .as_ref()
            .expect("leader stroke dash")
            .as_vec(text.len as usize, None),
        vec![vec![8.0, 4.0]]
    );
    assert_eq!(
        text.leader_stroke_cap.as_vec(text.len as usize, None),
        vec![StrokeCap::Square]
    );
    assert_eq!(
        text.leader_stroke_join.as_vec(text.len as usize, None),
        vec![StrokeJoin::Bevel]
    );
    assert_eq!(
        text.leader_label_padding.as_vec(text.len as usize, None),
        vec![6.0]
    );
    assert_eq!(
        text.leader_target_radius.as_vec(text.len as usize, None),
        vec![4.0]
    );
    assert_eq!(
        text.leader_min_length.as_vec(text.len as usize, None),
        vec![9.0]
    );
    assert_eq!(
        text.leader_shape.as_vec(text.len as usize, None),
        vec![SceneTextLeaderShape::Elbow]
    );
    assert_eq!(
        text.leader_arrow.as_vec(text.len as usize, None),
        vec![SceneTextLeaderArrow::Triangle]
    );
    assert_eq!(
        text.leader_arrow_length.as_vec(text.len as usize, None),
        vec![7.0]
    );
    assert_eq!(
        text.leader_arrow_width.as_vec(text.len as usize, None),
        vec![8.0]
    );
    Ok(())
}

#[tokio::test]
async fn image_expression_adjustment_uses_post_scale_anchor_size_source_data_and_bbox()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "dx",
            DataType::Float32,
            false,
        )])),
        vec![Arc::new(Float32Array::from(vec![3.0]))],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Image::new()
            .data(df)
            .x(20.0)
            .y(30.0)
            .width(10.0)
            .height(8.0)
            .align("center")
            .baseline("middle")
            .adjust(|image| {
                image
                    .x(image.bbox().left() + image.data("dx"))
                    .y(image.bbox().top() + lit(2.0))
                    .width(image.channel("width") + lit(5.0))
                    .height(image.channel("height") + image.bbox().bottom() - image.bbox().top())
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut images = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_images(mark, &mut images);
    }

    assert_eq!(images.len(), 1);
    let image = images[0];
    assert_eq!(image.x.as_vec(image.len as usize, None), vec![18.0]);
    assert_eq!(image.y.as_vec(image.len as usize, None), vec![28.0]);
    assert_eq!(image.width.as_vec(image.len as usize, None), vec![15.0]);
    assert_eq!(image.height.as_vec(image.len as usize, None), vec![16.0]);
    Ok(())
}

#[tokio::test]
async fn image_transform_adjustment_updates_anchor_and_size_channels()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Image::new()
            .unit_data()
            .x(20.0)
            .y(30.0)
            .width(10.0)
            .height(8.0)
            .adjust_transform(GeometryEchoAdjustment, |image, echo| {
                image
                    .x(echo.x() + lit(1.0))
                    .y(echo.y() + lit(2.0))
                    .width(echo.width() + lit(3.0))
                    .height(echo.height() + lit(4.0))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut images = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_images(mark, &mut images);
    }

    assert_eq!(images.len(), 1);
    let image = images[0];
    assert_eq!(image.x.as_vec(image.len as usize, None), vec![21.0]);
    assert_eq!(image.y.as_vec(image.len as usize, None), vec![32.0]);
    assert_eq!(image.width.as_vec(image.len as usize, None), vec![13.0]);
    assert_eq!(image.height.as_vec(image.len as usize, None), vec![12.0]);
    Ok(())
}

#[tokio::test]
async fn image_expression_adjustment_updates_align_and_baseline() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Image::new()
            .unit_data()
            .x(20.0)
            .y(30.0)
            .width(10.0)
            .height(8.0)
            .align("left")
            .baseline("top")
            .adjust(|image| image.align(lit("right")).baseline(lit("bottom")))
            .adjust(|image| image.x(image.bbox().left()).y(image.bbox().top())),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut images = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_images(mark, &mut images);
    }

    assert_eq!(images.len(), 1);
    let image = images[0];
    assert_eq!(
        image.align.as_vec(image.len as usize, None),
        vec![ImageAlign::Right]
    );
    assert_eq!(
        image.baseline.as_vec(image.len as usize, None),
        vec![ImageBaseline::Bottom]
    );
    assert_eq!(image.x.as_vec(image.len as usize, None), vec![10.0]);
    assert_eq!(image.y.as_vec(image.len as usize, None), vec![22.0]);
    Ok(())
}

#[tokio::test]
async fn image_expression_adjustment_updates_image_aspect_and_smooth_channels()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("aspect_out", DataType::Boolean, false),
            Field::new("smooth_out", DataType::Boolean, false),
        ])),
        vec![
            Arc::new(BooleanArray::from(vec![true, false])),
            Arc::new(BooleanArray::from(vec![false, true])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Image::new()
            .data(df)
            .x(20.0)
            .y(30.0)
            .width(10.0)
            .height(8.0)
            .align("left")
            .baseline("top")
            .aspect(false)
            .smooth(false)
            .adjust(|image| {
                image
                    .image(lit(TINY_PNG_DATA_URI))
                    .aspect(image.data("aspect_out"))
                    .smooth(image.data("smooth_out"))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut images = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_images(mark, &mut images);
    }

    assert_eq!(images.len(), 2);
    assert!(images[0].aspect);
    assert!(!images[0].smooth);
    assert!(!images[1].aspect);
    assert!(images[1].smooth);
    for image in &images {
        let rendered_image = image.image_iter().next().expect("image datum");
        assert_eq!(rendered_image.width, 2);
        assert_eq!(rendered_image.height, 2);
    }
    Ok(())
}

#[tokio::test]
async fn path_expression_adjustment_uses_post_scale_anchor_source_data_and_bbox()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "dx",
            DataType::Float32,
            false,
        )])),
        vec![Arc::new(Float32Array::from(vec![3.0]))],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        PathMark::new().data(df).x(10.0).y(20.0).adjust(|path| {
            path.x(path.channel("x") + path.data("dx"))
                .y(path.bbox().top() + lit(4.0))
        }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut paths = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_paths(mark, &mut paths);
    }

    assert_eq!(paths.len(), 1);
    let transforms = paths[0].transform.as_vec(paths[0].len as usize, None);
    assert_eq!(transforms.len(), 1);
    assert_eq!(transforms[0].m31, 13.0);
    assert_eq!(transforms[0].m32, 24.0);
    Ok(())
}

#[tokio::test]
async fn path_transform_adjustment_updates_anchor_channels() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        PathMark::new()
            .unit_data()
            .x(10.0)
            .y(20.0)
            .adjust_transform(Nudge::new(3.0, -2.0), |path, nudge| {
                path.x(nudge.x()).y(nudge.y())
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut paths = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_paths(mark, &mut paths);
    }

    assert_eq!(paths.len(), 1);
    let transforms = paths[0].transform.as_vec(paths[0].len as usize, None);
    assert_eq!(transforms.len(), 1);
    assert_eq!(transforms[0].m31, 13.0);
    assert_eq!(transforms[0].m32, 18.0);
    Ok(())
}

#[tokio::test]
async fn path_expression_adjustment_updates_path_transform() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        PathMark::new()
            .unit_data()
            .x(10.0)
            .y(20.0)
            .path_transform("translate(1 2)")
            .adjust(|path| path.path_transform(path.channel("path_transform")))
            .adjust(|path| path.path_transform(lit("translate(5 7)"))),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut paths = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_paths(mark, &mut paths);
    }

    assert_eq!(paths.len(), 1);
    let transforms = paths[0].transform.as_vec(paths[0].len as usize, None);
    assert_eq!(transforms.len(), 1);
    assert_eq!(transforms[0].m31, 15.0);
    assert_eq!(transforms[0].m32, 27.0);
    Ok(())
}

#[tokio::test]
async fn path_expression_adjustment_updates_path_and_style_channels()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("fill_out", DataType::Utf8, false),
            Field::new("width_out", DataType::Float32, false),
            Field::new("cap_out", DataType::Utf8, false),
            Field::new("join_out", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["#ff0000", "#0000ff"])),
            Arc::new(Float32Array::from(vec![2.0, 3.0])),
            Arc::new(StringArray::from(vec!["round", "square"])),
            Arc::new(StringArray::from(vec!["bevel", "miter"])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        PathMark::new()
            .data(df)
            .x(10.0)
            .y(20.0)
            .path("M 0 0 L 1 0")
            .path_transform("translate(0 0)")
            .fill("#111111")
            .stroke("#00ff00")
            .stroke_width(1.0)
            .stroke_cap("butt")
            .stroke_join("round")
            .opacity(1.0)
            .adjust(|path| {
                path.path(lit("M 0 0 L 2 0 L 2 2 Z"))
                    .path_transform(lit("translate(1 2)"))
                    .fill(path.data("fill_out"))
                    .stroke(path.channel("stroke"))
                    .stroke_width(path.data("width_out"))
                    .stroke_cap(path.data("cap_out"))
                    .stroke_join(path.data("join_out"))
                    .opacity(lit(0.5))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut paths = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_paths(mark, &mut paths);
    }

    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0].stroke_width, Some(2.0));
    assert_eq!(paths[1].stroke_width, Some(3.0));
    assert_eq!(paths[0].stroke_cap, StrokeCap::Round);
    assert_eq!(paths[1].stroke_cap, StrokeCap::Square);
    assert_eq!(paths[0].stroke_join, StrokeJoin::Bevel);
    assert_eq!(paths[1].stroke_join, StrokeJoin::Miter);

    assert_color_close(&paths[0].fill_vec()[0], [1.0, 0.0, 0.0, 0.5]);
    assert_color_close(&paths[1].fill_vec()[0], [0.0, 0.0, 1.0, 0.5]);
    assert_color_close(&paths[0].stroke_vec()[0], [0.0, 1.0, 0.0, 0.5]);
    assert_color_close(&paths[1].stroke_vec()[0], [0.0, 1.0, 0.0, 0.5]);

    for path in &paths {
        let transform = path.transform_vec()[0];
        assert_eq!(transform.m31, 11.0);
        assert_eq!(transform.m32, 22.0);
        assert!(
            path.path_vec()[0].iter().count() > 0,
            "adjusted path should parse to path geometry"
        );
    }
    Ok(())
}

#[tokio::test]
async fn line_expression_adjustment_uses_post_scale_vertices_source_data_and_bbox()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "dx",
            DataType::Float32,
            false,
        )])),
        vec![Arc::new(Float32Array::from(vec![1.0, 2.0]))],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Line::new().data(df).x(10.0).y(20.0).adjust(|line| {
            line.x(line.channel("x") + line.data("dx"))
                .y(line.bbox().top() + lit(4.0))
        }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut lines = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_lines(mark, &mut lines);
    }

    assert_eq!(lines.len(), 1);
    let line = lines[0];
    assert_eq!(line.x.as_vec(line.len as usize, None), vec![11.0, 12.0]);
    assert_eq!(line.y.as_vec(line.len as usize, None), vec![24.0, 24.0]);
    Ok(())
}

#[tokio::test]
async fn line_transform_adjustment_updates_vertex_channels() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "series",
            DataType::Utf8,
            false,
        )])),
        vec![Arc::new(StringArray::from(vec!["a", "a"]))],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Line::new()
            .data(df)
            .x(10.0)
            .y(20.0)
            .adjust_transform(Nudge::new(3.0, -2.0), |line, nudge| {
                line.x(nudge.x()).y(nudge.y())
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut lines = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_lines(mark, &mut lines);
    }

    assert_eq!(lines.len(), 1);
    let line = lines[0];
    assert_eq!(line.x.as_vec(line.len as usize, None), vec![13.0, 13.0]);
    assert_eq!(line.y.as_vec(line.len as usize, None), vec![18.0, 18.0]);
    Ok(())
}

#[tokio::test]
async fn line_transform_adjustment_updates_stroke_width() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "series",
            DataType::Utf8,
            false,
        )])),
        vec![Arc::new(StringArray::from(vec!["a", "a"]))],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Line::new()
            .data(df)
            .x(10.0)
            .y(20.0)
            .stroke_width(2.0)
            .adjust_transform(GeometryEchoAdjustment, |line, echo| {
                line.stroke_width(echo.stroke_width() + lit(1.5))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut lines = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_lines(mark, &mut lines);
    }

    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].stroke_width, 3.5);
    Ok(())
}

#[tokio::test]
async fn line_adjustment_preserves_partitioned_event_datum_lineage() -> Result<(), AvengerChartError>
{
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("x", DataType::Float32, false),
            Field::new("y", DataType::Float32, false),
            Field::new("dx", DataType::Float32, false),
            Field::new("stroke", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["a", "b", "c", "d"])),
            Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0, 4.0])),
            Arc::new(Float32Array::from(vec![10.0, 20.0, 30.0, 40.0])),
            Arc::new(Float32Array::from(vec![1.0, 1.0, 1.0, 1.0])),
            Arc::new(StringArray::from(vec![
                "#ef4444", "#3b82f6", "#ef4444", "#3b82f6",
            ])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new()
        .plot_size(100.0, 100.0)
        .mark(
            Line::new()
                .data(df)
                .x(col("x"))
                .y(col("y"))
                .stroke(col("stroke"))
                .adjust(|line| line.x(line.channel("x") + line.data("dx"))),
        )
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(avenger_chart::event::datum("id").is_not_null()),
        );

    let evaluated = plot.compile(&ctx).await?.evaluate(&ctx, None).await?;
    let mut lines = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_lines(mark, &mut lines);
    }

    assert!(
        lines.len() > 1,
        "line test should exercise partitioned output"
    );
    assert_eq!(
        event_datum_rows_with_id(&evaluated.event_datums.rows),
        lines.len()
    );
    assert_eq!(
        flattened_event_ids(&evaluated.event_datums.rows),
        vec!["a", "b", "c", "d"]
    );
    let rendered_len: usize = lines.iter().map(|line| line.len as usize).sum();
    let event_len: usize = evaluated
        .event_datums
        .rows
        .iter()
        .filter(|rows| rows.rows.column_by_name("id").is_some())
        .map(|rows| rows.rows.num_rows())
        .sum();
    assert_eq!(event_len, rendered_len);
    Ok(())
}

#[tokio::test]
async fn line_expression_adjustment_updates_stroke_width() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "extra",
            DataType::Float32,
            false,
        )])),
        vec![Arc::new(Float32Array::from(vec![1.0, 2.0]))],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Line::new()
            .data(df)
            .x(10.0)
            .y(20.0)
            .stroke_width(2.0)
            .adjust(|line| line.stroke_width(line.channel("stroke_width") + line.data("extra"))),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut lines = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_lines(mark, &mut lines);
    }

    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines
            .iter()
            .map(|line| line.stroke_width)
            .collect::<Vec<_>>(),
        vec![3.0, 4.0]
    );
    assert_eq!(lines.iter().map(|line| line.len as usize).sum::<usize>(), 2);
    Ok(())
}

#[tokio::test]
async fn line_expression_adjustment_updates_style_and_defined_channels()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("stroke_out", DataType::Utf8, false),
            Field::new("defined_out", DataType::Boolean, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["#ff0000", "#0000ff", "#0000ff"])),
            Arc::new(BooleanArray::from(vec![true, true, false])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Line::new()
            .data(df)
            .x(10.0)
            .y(20.0)
            .stroke("#111111")
            .stroke_width(2.0)
            .stroke_dash("solid")
            .stroke_cap("butt")
            .stroke_join("miter")
            .opacity(1.0)
            .defined(true)
            .adjust(|line| {
                line.stroke(line.data("stroke_out"))
                    .stroke_width(line.channel("stroke_width") + lit(1.0))
                    .stroke_dash(lit("dashed"))
                    .stroke_cap(lit("round"))
                    .stroke_join(lit("bevel"))
                    .opacity(lit(0.5))
                    .defined(line.data("defined_out"))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut lines = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_lines(mark, &mut lines);
    }

    assert_eq!(lines.len(), 2);
    assert_color_close(&lines[0].stroke, [1.0, 0.0, 0.0, 0.5]);
    assert_color_close(&lines[1].stroke, [0.0, 0.0, 1.0, 0.5]);
    for line in &lines {
        assert_eq!(line.stroke_width, 3.0);
        assert_eq!(line.stroke_dash, Some(vec![8.0, 4.0]));
        assert_eq!(line.stroke_cap, StrokeCap::Round);
        assert_eq!(line.stroke_join, StrokeJoin::Bevel);
    }
    assert_eq!(
        lines
            .iter()
            .flat_map(|line| line.defined.as_vec(line.len as usize, None))
            .collect::<Vec<_>>(),
        vec![true, true, false]
    );
    Ok(())
}

#[tokio::test]
async fn trail_expression_adjustment_uses_post_scale_vertices_source_data_and_bbox()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "dx",
            DataType::Float32,
            false,
        )])),
        vec![Arc::new(Float32Array::from(vec![1.0, 2.0]))],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Trail::new().data(df).x(10.0).y(20.0).adjust(|trail| {
            trail
                .x(trail.channel("x") + trail.data("dx"))
                .y(trail.bbox().top() + lit(4.0))
        }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut trails = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_trails(mark, &mut trails);
    }

    assert_eq!(trails.len(), 1);
    let trail = trails[0];
    assert_eq!(trail.x.as_vec(trail.len as usize, None), vec![11.0, 12.0]);
    assert_eq!(trail.y.as_vec(trail.len as usize, None), vec![24.0, 24.0]);
    Ok(())
}

#[tokio::test]
async fn trail_transform_adjustment_updates_vertex_channels() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "series",
            DataType::Utf8,
            false,
        )])),
        vec![Arc::new(StringArray::from(vec!["a", "a"]))],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Trail::new()
            .data(df)
            .x(10.0)
            .y(20.0)
            .adjust_transform(Nudge::new(3.0, -2.0), |trail, nudge| {
                trail.x(nudge.x()).y(nudge.y())
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut trails = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_trails(mark, &mut trails);
    }

    assert_eq!(trails.len(), 1);
    let trail = trails[0];
    assert_eq!(trail.x.as_vec(trail.len as usize, None), vec![13.0, 13.0]);
    assert_eq!(trail.y.as_vec(trail.len as usize, None), vec![18.0, 18.0]);
    Ok(())
}

#[tokio::test]
async fn trail_adjustment_preserves_partitioned_event_datum_lineage()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("x", DataType::Float32, false),
            Field::new("y", DataType::Float32, false),
            Field::new("dx", DataType::Float32, false),
            Field::new("stroke", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["a", "b", "c", "d"])),
            Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0, 4.0])),
            Arc::new(Float32Array::from(vec![10.0, 20.0, 30.0, 40.0])),
            Arc::new(Float32Array::from(vec![1.0, 1.0, 1.0, 1.0])),
            Arc::new(StringArray::from(vec![
                "#ef4444", "#3b82f6", "#ef4444", "#3b82f6",
            ])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new()
        .plot_size(100.0, 100.0)
        .mark(
            Trail::new()
                .data(df)
                .x(col("x"))
                .y(col("y"))
                .size(5.0)
                .stroke(col("stroke"))
                .adjust(|trail| trail.x(trail.channel("x") + trail.data("dx"))),
        )
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(avenger_chart::event::datum("id").is_not_null()),
        );

    let evaluated = plot.compile(&ctx).await?.evaluate(&ctx, None).await?;
    let mut trails = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_trails(mark, &mut trails);
    }

    assert!(
        trails.len() > 1,
        "trail test should exercise partitioned output"
    );
    assert_eq!(
        event_datum_rows_with_id(&evaluated.event_datums.rows),
        trails.len()
    );
    assert_eq!(
        flattened_event_ids(&evaluated.event_datums.rows),
        vec!["a", "b", "c", "d"]
    );
    let rendered_len: usize = trails.iter().map(|trail| trail.len as usize).sum();
    let event_len: usize = evaluated
        .event_datums
        .rows
        .iter()
        .filter(|rows| rows.rows.column_by_name("id").is_some())
        .map(|rows| rows.rows.num_rows())
        .sum();
    assert_eq!(event_len, rendered_len);
    Ok(())
}

#[tokio::test]
async fn trail_expression_adjustment_updates_size() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "extra",
            DataType::Float32,
            false,
        )])),
        vec![Arc::new(Float32Array::from(vec![1.0, 2.0]))],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Trail::new()
            .data(df)
            .x(10.0)
            .y(20.0)
            .size(3.0)
            .adjust(|trail| trail.size(trail.channel("size") + trail.data("extra"))),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut trails = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_trails(mark, &mut trails);
    }

    assert_eq!(trails.len(), 1);
    let trail = trails[0];
    assert_eq!(trail.size.as_vec(trail.len as usize, None), vec![4.0, 5.0]);
    Ok(())
}

#[tokio::test]
async fn trail_expression_adjustment_updates_style_and_defined_channels()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("stroke_out", DataType::Utf8, false),
            Field::new("defined_out", DataType::Boolean, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["#ff0000", "#0000ff", "#0000ff"])),
            Arc::new(BooleanArray::from(vec![true, true, false])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Trail::new()
            .data(df)
            .x(10.0)
            .y(20.0)
            .size(3.0)
            .stroke("#111111")
            .opacity(1.0)
            .defined(true)
            .adjust(|trail| {
                trail
                    .stroke(trail.data("stroke_out"))
                    .opacity(lit(0.5))
                    .defined(trail.data("defined_out"))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut trails = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_trails(mark, &mut trails);
    }

    assert_eq!(trails.len(), 2);
    assert_color_close(&trails[0].stroke, [1.0, 0.0, 0.0, 0.5]);
    assert_color_close(&trails[1].stroke, [0.0, 0.0, 1.0, 0.5]);
    assert_eq!(
        trails
            .iter()
            .flat_map(|trail| trail.defined.as_vec(trail.len as usize, None))
            .collect::<Vec<_>>(),
        vec![true, true, false]
    );
    Ok(())
}

#[tokio::test]
async fn area_expression_adjustment_uses_post_scale_vertices_source_data_and_bbox()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "dx",
            DataType::Float32,
            false,
        )])),
        vec![Arc::new(Float32Array::from(vec![1.0, 2.0]))],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Area::new()
            .data(df)
            .x(10.0)
            .y(20.0)
            .x2(10.0)
            .y2(5.0)
            .adjust(|area| {
                area.x(area.channel("x") + area.data("dx"))
                    .x2(area.channel("x2") + area.data("dx"))
                    .y(area.bbox().bottom() + lit(4.0))
                    .y2(area.bbox().top() - lit(2.0))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut areas = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_areas(mark, &mut areas);
    }

    assert_eq!(areas.len(), 1);
    let area = areas[0];
    assert_eq!(area.x.as_vec(area.len as usize, None), vec![11.0, 12.0]);
    assert_eq!(area.x2.as_vec(area.len as usize, None), vec![11.0, 12.0]);
    assert_eq!(area.y.as_vec(area.len as usize, None), vec![24.0, 24.0]);
    assert_eq!(area.y2.as_vec(area.len as usize, None), vec![3.0, 3.0]);
    Ok(())
}

#[tokio::test]
async fn area_transform_adjustment_updates_vertex_channels() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "series",
            DataType::Utf8,
            false,
        )])),
        vec![Arc::new(StringArray::from(vec!["a", "a"]))],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Area::new()
            .data(df)
            .x(10.0)
            .y(20.0)
            .x2(10.0)
            .y2(5.0)
            .stroke_width(2.0)
            .adjust_transform(GeometryEchoAdjustment, |area, echo| {
                area.x(echo.x() + lit(1.0))
                    .y(echo.y() + lit(2.0))
                    .x2(echo.x2() + lit(3.0))
                    .y2(echo.y2() + lit(4.0))
                    .stroke_width(echo.stroke_width() + lit(1.5))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut areas = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_areas(mark, &mut areas);
    }

    assert_eq!(areas.len(), 1);
    let area = areas[0];
    assert_eq!(area.x.as_vec(area.len as usize, None), vec![11.0, 11.0]);
    assert_eq!(area.y.as_vec(area.len as usize, None), vec![22.0, 22.0]);
    assert_eq!(area.x2.as_vec(area.len as usize, None), vec![13.0, 13.0]);
    assert_eq!(area.y2.as_vec(area.len as usize, None), vec![9.0, 9.0]);
    assert_eq!(area.stroke_width, 3.5);
    Ok(())
}

#[tokio::test]
async fn area_adjustment_preserves_partitioned_event_datum_lineage() -> Result<(), AvengerChartError>
{
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("x", DataType::Float32, false),
            Field::new("y", DataType::Float32, false),
            Field::new("y2", DataType::Float32, false),
            Field::new("dx", DataType::Float32, false),
            Field::new("fill", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["a", "b", "c", "d"])),
            Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0, 4.0])),
            Arc::new(Float32Array::from(vec![10.0, 20.0, 30.0, 40.0])),
            Arc::new(Float32Array::from(vec![0.0, 0.0, 0.0, 0.0])),
            Arc::new(Float32Array::from(vec![1.0, 1.0, 1.0, 1.0])),
            Arc::new(StringArray::from(vec![
                "#ef4444", "#3b82f6", "#ef4444", "#3b82f6",
            ])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new()
        .plot_size(100.0, 100.0)
        .mark(
            Area::new()
                .data(df)
                .x(col("x"))
                .x2(col("x"))
                .y(col("y"))
                .y2(col("y2"))
                .fill(col("fill"))
                .adjust(|area| {
                    area.x(area.channel("x") + area.data("dx"))
                        .x2(area.channel("x2") + area.data("dx"))
                }),
        )
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(avenger_chart::event::datum("id").is_not_null()),
        );

    let evaluated = plot.compile(&ctx).await?.evaluate(&ctx, None).await?;
    let mut areas = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_areas(mark, &mut areas);
    }

    assert!(
        areas.len() > 1,
        "area test should exercise partitioned output"
    );
    assert_eq!(
        event_datum_rows_with_id(&evaluated.event_datums.rows),
        areas.len()
    );
    assert_eq!(
        flattened_event_ids(&evaluated.event_datums.rows),
        vec!["a", "b", "c", "d"]
    );
    let rendered_len: usize = areas.iter().map(|area| area.len as usize).sum();
    let event_len: usize = evaluated
        .event_datums
        .rows
        .iter()
        .filter(|rows| rows.rows.column_by_name("id").is_some())
        .map(|rows| rows.rows.num_rows())
        .sum();
    assert_eq!(event_len, rendered_len);
    Ok(())
}

#[tokio::test]
async fn area_expression_adjustment_updates_stroke_width() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "extra",
            DataType::Float32,
            false,
        )])),
        vec![Arc::new(Float32Array::from(vec![1.0, 2.0]))],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Area::new()
            .data(df)
            .x(10.0)
            .y(20.0)
            .x2(10.0)
            .y2(5.0)
            .stroke_width(2.0)
            .adjust(|area| area.stroke_width(area.channel("stroke_width") + area.data("extra"))),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut areas = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_areas(mark, &mut areas);
    }

    assert_eq!(areas.len(), 2);
    assert_eq!(
        areas
            .iter()
            .map(|area| area.stroke_width)
            .collect::<Vec<_>>(),
        vec![3.0, 4.0]
    );
    assert_eq!(areas.iter().map(|area| area.len as usize).sum::<usize>(), 2);
    Ok(())
}

#[tokio::test]
async fn area_expression_adjustment_updates_style_and_defined_channels()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("orientation_out", DataType::Utf8, false),
            Field::new("fill_out", DataType::Utf8, false),
            Field::new("defined_out", DataType::Boolean, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec![
                "vertical",
                "horizontal",
                "horizontal",
            ])),
            Arc::new(StringArray::from(vec!["#ff0000", "#0000ff", "#0000ff"])),
            Arc::new(BooleanArray::from(vec![true, true, false])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Area::new()
            .data(df)
            .x(10.0)
            .y(20.0)
            .x2(10.0)
            .y2(5.0)
            .fill("#111111")
            .stroke("#00ff00")
            .stroke_width(2.0)
            .stroke_dash("solid")
            .stroke_cap("butt")
            .stroke_join("round")
            .opacity(1.0)
            .defined(true)
            .adjust(|area| {
                area.orientation(area.data("orientation_out"))
                    .fill(area.data("fill_out"))
                    .stroke(area.channel("stroke"))
                    .stroke_width(area.channel("stroke_width") + lit(1.0))
                    .stroke_dash(lit("dashed"))
                    .stroke_cap(lit("square"))
                    .stroke_join(lit("bevel"))
                    .opacity(lit(0.5))
                    .defined(area.data("defined_out"))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled)?;
    let evaluated = decoded.evaluate(&ctx, None).await?;
    let mut areas = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_areas(mark, &mut areas);
    }

    assert_eq!(areas.len(), 2);
    assert_eq!(areas[0].orientation, AreaOrientation::Vertical);
    assert_eq!(areas[1].orientation, AreaOrientation::Horizontal);
    assert_color_close(&areas[0].fill, [1.0, 0.0, 0.0, 0.5]);
    assert_color_close(&areas[1].fill, [0.0, 0.0, 1.0, 0.5]);
    for area in &areas {
        assert_color_close(&area.stroke, [0.0, 1.0, 0.0, 0.5]);
        assert_eq!(area.stroke_width, 3.0);
        assert_eq!(area.stroke_dash, Some(vec![8.0, 4.0]));
        assert_eq!(area.stroke_cap, StrokeCap::Square);
        assert_eq!(area.stroke_join, StrokeJoin::Bevel);
    }
    assert_eq!(
        areas
            .iter()
            .flat_map(|area| area.defined.as_vec(area.len as usize, None))
            .collect::<Vec<_>>(),
        vec![true, true, false]
    );
    Ok(())
}

#[tokio::test]
async fn polar_symbol_adjustment_errors_until_supported() {
    let ctx = SessionContext::new();
    let plot = Plot::<Polar>::new().mark(
        Symbol::<Polar>::new()
            .unit_data()
            .adjust(|point| point.x(point.channel("x") + lit(1.0))),
    );

    let err = match plot.compile(&ctx).await {
        Ok(_) => panic!("polar symbol adjustment should not compile until it is implemented"),
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("Symbol<Polar> adjustments are not implemented yet")
    );
}

#[tokio::test]
async fn polar_line_adjustment_errors_until_supported() {
    let ctx = SessionContext::new();
    let plot = Plot::<Polar>::new().mark(
        Line::<Polar>::new()
            .unit_data()
            .adjust(|point| point.x(point.channel("x") + lit(1.0))),
    );

    let err = match plot.compile(&ctx).await {
        Ok(_) => panic!("polar line adjustment should not compile until it is implemented"),
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("Line<Polar> adjustments are not implemented yet")
    );
}

#[tokio::test]
async fn item_frame_expression_in_regular_channel_errors() {
    let ctx = SessionContext::new();
    let item = AdjustItem::<Symbol<Cartesian>, PointGeometryItem>::default();
    let plot = Plot::<Cartesian>::new()
        .plot_size(100.0, 100.0)
        .mark(Symbol::new().unit_data().x(item.channel("x")).y(10.0));

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    let err = match compiled.evaluate(&ctx, None).await {
        Ok(_) => panic!("item-frame expression in ordinary channel should error"),
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("only valid inside mark effect closures"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn item_frame_expression_in_scale_domain_config_errors() {
    let ctx = SessionContext::new();
    let item = AdjustItem::<Symbol<Cartesian>, PointGeometryItem>::default();
    let bad_item_x = item.channel("x");
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .unit_data()
            .x_with(lit(5.0), |x| {
                x.scale_with::<Linear>(move |scale| scale.domain((lit(0.0), bad_item_x.clone())))
            })
            .y(10.0),
    );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    let err = match compiled.evaluate(&ctx, None).await {
        Ok(_) => panic!("item-frame expression in scale config should error"),
        Err(err) => err,
    };
    let message = err.to_string();
    assert!(
        message.contains("only valid inside mark effect closures")
            && message.contains("scale configuration"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn item_frame_expression_in_axis_config_errors() {
    let ctx = SessionContext::new();
    let item = AdjustItem::<Symbol<Cartesian>, PointGeometryItem>::default();
    let bad_item_x = item.channel("x");
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .unit_data()
            .x_with(lit(5.0), |x| x.axis(|axis| axis.title(bad_item_x.clone())))
            .y(10.0),
    );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    let err = match compiled.evaluate(&ctx, None).await {
        Ok(_) => panic!("item-frame expression in axis config should error"),
        Err(err) => err,
    };
    let message = err.to_string();
    assert!(
        message.contains("only valid inside mark effect closures")
            && message.contains("axis configuration"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn item_frame_expression_in_legend_config_errors() {
    let ctx = SessionContext::new();
    let item = AdjustItem::<Symbol<Cartesian>, PointGeometryItem>::default();
    let bad_item_x = item.channel("x");
    let plot = Plot::<Cartesian>::new().plot_size(100.0, 100.0).mark(
        Symbol::new()
            .unit_data()
            .x(lit(5.0))
            .y(lit(10.0))
            .fill_with(lit("category"), |fill| {
                fill.legend(|legend| legend.title(bad_item_x.clone()))
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    let err = match compiled.evaluate(&ctx, None).await {
        Ok(_) => panic!("item-frame expression in legend config should error"),
        Err(err) => err,
    };
    let message = err.to_string();
    assert!(
        message.contains("only valid inside mark effect closures")
            && message.contains("legend configuration"),
        "unexpected error: {err}"
    );
}
