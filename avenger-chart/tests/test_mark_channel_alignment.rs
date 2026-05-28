use avenger_chart::prelude::*;
use avenger_common::{
    types::ColorOrGradient,
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_scenegraph::marks::mark::SceneMarkType;
use avenger_scenegraph::marks::{
    area::SceneAreaMark, image::SceneImageMark, mark::SceneMark, path::ScenePathMark,
    rule::SceneRuleMark, symbol::SceneSymbolMark, text::SceneTextMark, trail::SceneTrailMark,
};
use datafusion::{
    arrow::{
        array::{Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::{DataFrame, SessionContext},
};
use std::sync::Arc;

fn channel<'a>(
    channels: &'a [avenger_chart_core::ChannelDescriptor],
    name: &str,
) -> &'a avenger_chart_core::ChannelDescriptor {
    channels
        .iter()
        .find(|channel| channel.name == name)
        .unwrap_or_else(|| panic!("missing channel descriptor {name}"))
}

fn collect_symbols<'a>(mark: &'a SceneMark, symbols: &mut Vec<&'a SceneSymbolMark>) {
    match mark {
        SceneMark::Symbol(symbol) => symbols.push(symbol),
        SceneMark::Group(group) => {
            for mark in &group.marks {
                collect_symbols(mark, symbols);
            }
        }
        _ => {}
    }
}

fn collect_rules<'a>(mark: &'a SceneMark, rules: &mut Vec<&'a SceneRuleMark>) {
    match mark {
        SceneMark::Rule(rule) => rules.push(rule),
        SceneMark::Group(group) => {
            for mark in &group.marks {
                collect_rules(mark, rules);
            }
        }
        _ => {}
    }
}

fn collect_text<'a>(mark: &'a SceneMark, text: &mut Vec<&'a SceneTextMark>) {
    match mark {
        SceneMark::Text(text_mark) => text.push(text_mark),
        SceneMark::Group(group) => {
            for mark in &group.marks {
                collect_text(mark, text);
            }
        }
        _ => {}
    }
}

fn collect_areas<'a>(mark: &'a SceneMark, areas: &mut Vec<&'a SceneAreaMark>) {
    match mark {
        SceneMark::Area(area) => areas.push(area),
        SceneMark::Group(group) => {
            for mark in &group.marks {
                collect_areas(mark, areas);
            }
        }
        _ => {}
    }
}

fn collect_trails<'a>(mark: &'a SceneMark, trails: &mut Vec<&'a SceneTrailMark>) {
    match mark {
        SceneMark::Trail(trail) => trails.push(trail),
        SceneMark::Group(group) => {
            for mark in &group.marks {
                collect_trails(mark, trails);
            }
        }
        _ => {}
    }
}

fn collect_images<'a>(mark: &'a SceneMark, images: &mut Vec<&'a SceneImageMark>) {
    match mark {
        SceneMark::Image(image) => images.push(image),
        SceneMark::Group(group) => {
            for mark in &group.marks {
                collect_images(mark, images);
            }
        }
        _ => {}
    }
}

fn collect_paths<'a>(mark: &'a SceneMark, paths: &mut Vec<&'a ScenePathMark>) {
    match mark {
        SceneMark::Path(path) => paths.push(path),
        SceneMark::Group(group) => {
            for mark in &group.marks {
                collect_paths(mark, paths);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn cartesian_mark_channel_descriptors_align_with_existing_renderers() {
    let ctx = SessionContext::new();

    let symbol_plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .opacity(col("opacity"))
            .stroke_width(2.0),
    );
    let symbol = symbol_plot.compile(&ctx).await.unwrap().marks()[0].clone();
    let symbol_channels = symbol.supported_channels();
    assert!(!channel(&symbol_channels, "stroke_width").allow_column_ref);
    assert!(channel(&symbol_channels, "opacity").allow_column_ref);

    let line_plot =
        Plot::<Cartesian>::new().mark(Line::new().x(col("x")).y(col("y")).opacity(col("opacity")));
    let line = line_plot.compile(&ctx).await.unwrap().marks()[0].clone();
    let line_channels = line.supported_channels();
    assert!(channel(&line_channels, "opacity").allow_column_ref);
    assert!(
        line_channels
            .iter()
            .all(|channel| channel.name != "stroke_opacity")
    );

    let rect_plot = Plot::<Cartesian>::new().mark(
        Rect::new()
            .x(col("x"))
            .x2(col("x2"))
            .y(col("y"))
            .y2(col("y2"))
            .corner_radius(col("corner_radius")),
    );
    let rect = rect_plot.compile(&ctx).await.unwrap().marks()[0].clone();
    let rect_channels = rect.supported_channels();
    assert!(channel(&rect_channels, "corner_radius").allow_column_ref);
}

#[test]
fn cartesian_scene_mark_coverage_is_explicit() {
    fn cartesian_coverage(mark_type: SceneMarkType) -> &'static str {
        match mark_type {
            SceneMarkType::Arc => "explicitly deferred to polar/sector-style marks",
            SceneMarkType::Area => "Area<Cartesian>",
            SceneMarkType::Path => "PathMark<Cartesian>",
            SceneMarkType::Symbol => "Symbol<Cartesian>",
            SceneMarkType::Line => "Line<Cartesian>",
            SceneMarkType::Trail => "Trail<Cartesian>",
            SceneMarkType::Rect => "Rect<Cartesian>",
            SceneMarkType::Rule => "Rule<Cartesian>",
            SceneMarkType::Text => "Text<Cartesian>",
            SceneMarkType::Image => "Image<Cartesian>",
            SceneMarkType::Group => "explicitly covered by layout/composition",
        }
    }

    assert_eq!(
        cartesian_coverage(SceneMarkType::Path),
        "PathMark<Cartesian>"
    );
    assert_eq!(
        cartesian_coverage(SceneMarkType::Arc),
        "explicitly deferred to polar/sector-style marks"
    );
    assert_eq!(
        cartesian_coverage(SceneMarkType::Group),
        "explicitly covered by layout/composition"
    );
}

#[tokio::test]
async fn rule_and_text_channel_descriptors_expose_scene_mark_channels() {
    let ctx = SessionContext::new();

    let rule_plot = Plot::<Cartesian>::new().mark(
        Rule::new()
            .x(col("x"))
            .y(col("y"))
            .x2(col("x2"))
            .y2(col("y2"))
            .stroke_cap(col("cap"))
            .opacity(col("opacity")),
    );
    let rule = rule_plot.compile(&ctx).await.unwrap().marks()[0].clone();
    let rule_channels = rule.supported_channels();
    assert!(channel(&rule_channels, "stroke_cap").allow_column_ref);
    assert!(channel(&rule_channels, "opacity").allow_column_ref);

    let text_plot = Plot::<Cartesian>::new().mark(
        Text::new()
            .x(col("x"))
            .y(col("y"))
            .text(col("label"))
            .align(col("align"))
            .baseline(col("baseline")),
    );
    let text = text_plot.compile(&ctx).await.unwrap().marks()[0].clone();
    let text_channels = text.supported_channels();
    assert!(channel(&text_channels, "text").allow_column_ref);
    assert!(channel(&text_channels, "font_size").allow_column_ref);
}

#[tokio::test]
async fn area_and_trail_channel_descriptors_expose_scene_mark_channels() {
    let ctx = SessionContext::new();

    let area_plot = Plot::<Cartesian>::new().mark(
        Area::new()
            .x(col("x"))
            .y(col("y"))
            .y2(0.0)
            .orientation("horizontal")
            .stroke_width(col("width"))
            .stroke_dash(col("dash"))
            .opacity(col("opacity")),
    );
    let area = area_plot.compile(&ctx).await.unwrap().marks()[0].clone();
    let area_channels = area.supported_channels();
    assert!(channel(&area_channels, "stroke_width").allow_column_ref);
    assert!(channel(&area_channels, "stroke_dash").allow_column_ref);
    assert!(channel(&area_channels, "opacity").allow_column_ref);
    assert!(!channel(&area_channels, "orientation").allow_column_ref);

    let trail_plot = Plot::<Cartesian>::new().mark(
        Trail::new()
            .x(col("x"))
            .y(col("y"))
            .size(col("size"))
            .stroke(col("stroke"))
            .opacity(col("opacity")),
    );
    let trail = trail_plot.compile(&ctx).await.unwrap().marks()[0].clone();
    let trail_channels = trail.supported_channels();
    assert!(channel(&trail_channels, "size").allow_column_ref);
    assert!(channel(&trail_channels, "stroke").allow_column_ref);
    assert!(channel(&trail_channels, "opacity").allow_column_ref);
}

#[tokio::test]
async fn image_and_path_channel_descriptors_expose_scene_mark_channels() {
    let ctx = SessionContext::new();

    let image_plot = Plot::<Cartesian>::new().mark(
        Image::new()
            .x(col("x"))
            .y(col("y"))
            .image(TINY_PNG_DATA_URI)
            .width(col("width"))
            .height(col("height"))
            .align(col("align"))
            .baseline(col("baseline"))
            .aspect(false),
    );
    let image = image_plot.compile(&ctx).await.unwrap().marks()[0].clone();
    let image_channels = image.supported_channels();
    assert!(channel(&image_channels, "image").allow_column_ref);
    assert!(channel(&image_channels, "align").allow_column_ref);
    assert!(channel(&image_channels, "baseline").allow_column_ref);
    assert!(!channel(&image_channels, "aspect").allow_column_ref);
    assert!(!channel(&image_channels, "smooth").allow_column_ref);

    let path_plot = Plot::<Cartesian>::new().mark(
        PathMark::new()
            .x(col("x"))
            .y(col("y"))
            .path("M -8 -8 L 8 -8 L 0 8 Z")
            .transform("rotate(15)")
            .fill(col("fill"))
            .stroke_width(col("width"))
            .opacity(col("opacity")),
    );
    let path = path_plot.compile(&ctx).await.unwrap().marks()[0].clone();
    let path_channels = path.supported_channels();
    assert!(channel(&path_channels, "path").allow_column_ref);
    assert!(channel(&path_channels, "transform").allow_column_ref);
    assert!(channel(&path_channels, "opacity").allow_column_ref);
    assert!(!channel(&path_channels, "stroke_width").allow_column_ref);
}

#[tokio::test]
async fn symbol_opacity_is_folded_into_fill_and_stroke_alpha() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(0.0)
            .y(0.0)
            .fill("#ff0000")
            .stroke("#0000ff")
            .opacity(0.25),
    );

    let evaluated = plot
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let mut symbols = Vec::new();
    for mark in &evaluated.scene_graph.marks {
        collect_symbols(mark, &mut symbols);
    }

    let symbol = symbols.first().expect("symbol mark");
    assert_eq!(
        alpha(&symbol.fill),
        0.25,
        "fill alpha should include opacity"
    );
    assert_eq!(
        alpha(&symbol.stroke),
        0.25,
        "stroke alpha should include opacity"
    );
}

#[tokio::test]
async fn rule_opacity_is_folded_into_stroke_alpha() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().mark(
        Rule::new()
            .x(0.0)
            .y(0.0)
            .x2(1.0)
            .y2(1.0)
            .stroke("#ff0000")
            .opacity(0.25),
    );

    let evaluated = plot
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let mut rules = Vec::new();
    for mark in &evaluated.scene_graph.marks {
        collect_rules(mark, &mut rules);
    }

    let rule = rules.first().expect("rule mark");
    assert_eq!(alpha(&rule.stroke), 0.25);
}

#[tokio::test]
async fn area_opacity_is_folded_into_fill_and_stroke_alpha() {
    let ctx = SessionContext::new();
    let data = xy_data(&ctx);
    let plot = Plot::<Cartesian>::new().data(data).mark(
        Area::new()
            .x(col("x"))
            .y(col("y"))
            .y2(0.0)
            .fill("#ff0000")
            .stroke("#0000ff")
            .stroke_width(2.0)
            .opacity(0.25),
    );

    let evaluated = plot
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let mut areas = Vec::new();
    for mark in &evaluated.scene_graph.marks {
        collect_areas(mark, &mut areas);
    }

    let area = areas.first().expect("area mark");
    assert_eq!(color_alpha(&area.fill), 0.25);
    assert_eq!(color_alpha(&area.stroke), 0.25);
}

#[tokio::test]
async fn trail_opacity_is_folded_into_stroke_alpha() {
    let ctx = SessionContext::new();
    let data = xy_data(&ctx);
    let plot = Plot::<Cartesian>::new().data(data).mark(
        Trail::new()
            .x(col("x"))
            .y(col("y"))
            .size(10.0)
            .stroke("#ff0000")
            .opacity(0.25),
    );

    let evaluated = plot
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let mut trails = Vec::new();
    for mark in &evaluated.scene_graph.marks {
        collect_trails(mark, &mut trails);
    }

    let trail = trails.first().expect("trail mark");
    assert_eq!(color_alpha(&trail.stroke), 0.25);
}

#[tokio::test]
async fn varying_area_and_trail_scalar_styles_partition_scene_marks() {
    let ctx = SessionContext::new();

    let area_plot = Plot::<Cartesian>::new().data(styled_xy_data(&ctx)).mark(
        Area::new()
            .x(col("x"))
            .y(col("y"))
            .y2(0.0)
            .fill(ChannelValue::from(col("fill")).no_scale())
            .opacity_with(col("opacity"), |c| c.no_scale()),
    );
    let area_evaluated = area_plot
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let mut areas = Vec::new();
    for mark in &area_evaluated.scene_graph.marks {
        collect_areas(mark, &mut areas);
    }
    assert_eq!(
        areas.len(),
        2,
        "area should partition when fill or opacity varies"
    );

    let trail_plot = Plot::<Cartesian>::new().data(styled_xy_data(&ctx)).mark(
        Trail::new()
            .x(col("x"))
            .y(col("y"))
            .size(8.0)
            .stroke(ChannelValue::from(col("fill")).no_scale())
            .opacity_with(col("opacity"), |c| c.no_scale()),
    );
    let trail_evaluated = trail_plot
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let mut trails = Vec::new();
    for mark in &trail_evaluated.scene_graph.marks {
        collect_trails(mark, &mut trails);
    }
    assert_eq!(
        trails.len(),
        2,
        "trail should partition when stroke or opacity varies"
    );
}

#[tokio::test]
async fn image_data_uri_renders_without_network_resources() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().mark(
        Image::new()
            .x(0.5)
            .y(0.5)
            .image(TINY_PNG_DATA_URI)
            .width(16.0)
            .height(16.0)
            .align("center")
            .baseline("middle")
            .smooth(false),
    );

    let evaluated = plot
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let mut images = Vec::new();
    for mark in &evaluated.scene_graph.marks {
        collect_images(mark, &mut images);
    }

    let image = images.first().expect("image mark");
    let rgba = image.image.first().expect("decoded image");
    assert_eq!((rgba.width, rgba.height), (2, 2));
}

#[tokio::test]
async fn path_opacity_is_folded_into_fill_and_stroke_alpha() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().mark(
        PathMark::new()
            .x(0.5)
            .y(0.5)
            .path("M -8 -8 L 8 -8 L 0 8 Z")
            .fill("#ff0000")
            .stroke("#0000ff")
            .stroke_width(1.5)
            .opacity(0.25),
    );

    let evaluated = plot
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let mut paths = Vec::new();
    for mark in &evaluated.scene_graph.marks {
        collect_paths(mark, &mut paths);
    }

    let path = paths.first().expect("path mark");
    assert_eq!(alpha(&path.fill), 0.25);
    assert_eq!(alpha(&path.stroke), 0.25);
}

#[tokio::test]
async fn text_column_values_render_as_strings_without_scale() {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("label", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(Float64Array::from(vec![0.0, 1.0])),
            Arc::new(Float64Array::from(vec![0.0, 1.0])),
            Arc::new(StringArray::from(vec!["left", "right"])),
        ],
    )
    .unwrap();
    let df = ctx.read_batch(batch).unwrap();
    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(Text::new().x(col("x")).y(col("y")).text(col("label")));

    let evaluated = plot
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let mut text_marks = Vec::new();
    for mark in &evaluated.scene_graph.marks {
        collect_text(mark, &mut text_marks);
    }

    let text = text_marks.first().expect("text mark");
    assert_eq!(
        text.text.as_vec(2, None),
        vec!["left".to_string(), "right".to_string()]
    );
}

fn alpha(colors: &ScalarOrArray<ColorOrGradient>) -> f32 {
    match colors.value() {
        ScalarOrArrayValue::Scalar(ColorOrGradient::Color(color)) => color[3],
        ScalarOrArrayValue::Array(colors) => match colors.first().expect("first color") {
            ColorOrGradient::Color(color) => color[3],
            ColorOrGradient::GradientIndex(_) => panic!("expected concrete color"),
        },
        ScalarOrArrayValue::Scalar(ColorOrGradient::GradientIndex(_)) => {
            panic!("expected concrete color")
        }
    }
}

fn color_alpha(color: &ColorOrGradient) -> f32 {
    match color {
        ColorOrGradient::Color(color) => color[3],
        ColorOrGradient::GradientIndex(_) => panic!("expected concrete color"),
    }
}

fn xy_data(ctx: &SessionContext) -> DataFrame {
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ])),
        vec![
            Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0])),
            Arc::new(Float64Array::from(vec![0.5, 1.0, 0.25])),
        ],
    )
    .unwrap();

    ctx.read_batch(batch).unwrap()
}

fn styled_xy_data(ctx: &SessionContext) -> DataFrame {
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("fill", DataType::Utf8, false),
            Field::new("opacity", DataType::Float64, false),
        ])),
        vec![
            Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0])),
            Arc::new(Float64Array::from(vec![0.5, 1.0, 0.25, 0.8])),
            Arc::new(StringArray::from(vec![
                "#ff0000", "#ff0000", "#0000ff", "#0000ff",
            ])),
            Arc::new(Float64Array::from(vec![0.45, 0.45, 0.8, 0.8])),
        ],
    )
    .unwrap();

    ctx.read_batch(batch).unwrap()
}

const TINY_PNG_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAG0lEQVR4nGO4o6b2XzX59X8GscVe/3+dEf0PAE8fCXZKLiUkAAAAAElFTkSuQmCC";
