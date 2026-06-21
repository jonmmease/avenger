use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use arrow::{
    array::{Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use avenger_chart::channel::LegendableChannel;
use avenger_chart::facet::coord::FacetColumn;
use avenger_chart::facet::marks::FacetColumnSubplotChannels;
use avenger_chart::plot::Plot;
use avenger_chart::prelude::Subplot;
use avenger_chart_core::{ChannelValue, CoordinateSystem};
use avenger_chart_treemap::{
    TreeHeader, TreeLabel, TreeRect, Treemap, TreemapGuide, TreemapHeaderBars, TreemapPadding,
};
use avenger_common::canvas::CanvasDimensions;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::{
    functions_aggregate::expr_fn::sum,
    logical_expr::{col, lit},
    prelude::SessionContext,
};
use image::RgbaImage;

const BASELINE_DIR: &str = "tests/baselines";
const FAILURE_DIR: &str = "tests/failures";
const BLESS_ENV: &str = "AVENGER_TREEMAP_BLESS_BASELINES";
const DEFAULT_SCALE: f32 = 2.0;
const DEFAULT_THRESHOLD: f64 = 0.9999;

#[tokio::test]
async fn treemap_sum_value_full() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(sales_data()).unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "product"])
            .value(sum(col("sales"))),
    )
    .data(df)
    .plot_size(480.0, 280.0)
    .mark(TreeRect::new().fill(col("division")).stroke("#ffffff"));

    assert_visual_match(plot, "treemap_sum_value_full").await;
}

#[tokio::test]
async fn treemap_selected_overlay_same_layout() {
    let ctx = SessionContext::new();
    let base = ctx.read_batch(sales_data()).unwrap();
    let selected = ctx
        .read_batch(sales_data())
        .unwrap()
        .filter(col("division").eq(lit("International")))
        .unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "product"])
            .value(sum(col("sales"))),
    )
    .data(base)
    .plot_size(480.0, 280.0)
    .mark(TreeRect::new().id("base").fill("#d8dde3").stroke("#ffffff"))
    .mark(
        TreeRect::new()
            .id("selected")
            .data(selected)
            .fill("#d62728")
            .opacity(0.85)
            .stroke("#ffffff"),
    );

    assert_visual_match(plot, "treemap_selected_overlay_same_layout").await;
}

#[tokio::test]
async fn treemap_stacked_cell_segments() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(segment_data()).unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "product"])
            .value(sum(col("sales"))),
    )
    .data(df)
    .plot_size(480.0, 280.0)
    .mark(
        TreeRect::new()
            .id("segments")
            .fill(col("segment"))
            .v(ChannelValue::from(col("v0")).no_scale())
            .v2(ChannelValue::from(col("v1")).no_scale())
            .stroke("#ffffff"),
    );

    assert_visual_match(plot, "treemap_stacked_cell_segments").await;
}

#[tokio::test]
async fn treemap_group_headers() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(deep_data()).unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .display_levels(3),
    )
    .data(df)
    .plot_size(560.0, 320.0)
    .configure_guide(
        TreemapGuide::new()
            .headers(true)
            .separators(true)
            .breadcrumbs(false),
    )
    .mark(TreeRect::new().fill(col("region")).stroke("#ffffff"));

    assert_visual_match(plot, "treemap_group_headers").await;
}

#[tokio::test]
async fn treemap_strict_area_overlay_headers() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(deep_data()).unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .display_levels(3),
    )
    .data(df)
    .plot_size(560.0, 320.0)
    .configure_guide(
        TreemapGuide::new()
            .headers(true)
            .separators(true)
            .breadcrumbs(false),
    )
    .mark(TreeRect::new().fill(col("region")).stroke("#ffffff"));

    assert_visual_match(plot, "treemap_strict_area_overlay_headers").await;
}

#[tokio::test]
async fn treemap_leaf_labels_basic() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(sales_data()).unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "product"])
            .value(sum(col("sales"))),
    )
    .data(df)
    .plot_size(480.0, 280.0)
    .mark(TreeRect::new().fill(col("division")).stroke("#ffffff"))
    .mark(TreeLabel::new().color("#ffffff"));

    assert_visual_match(plot, "treemap_leaf_labels_basic").await;
}

#[tokio::test]
async fn treemap_leaf_labels_elide() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(label_elide_data()).unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "product"])
            .value(sum(col("sales"))),
    )
    .data(df)
    .plot_size(420.0, 220.0)
    .mark(TreeRect::new().fill(col("division")).stroke("#ffffff"))
    .mark(TreeLabel::new().color("#ffffff"));

    assert_visual_match(plot, "treemap_leaf_labels_elide").await;
}

#[tokio::test]
async fn treemap_leaf_labels_zoom_window() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(deep_data()).unwrap();
    let mark_df = ctx
        .read_batch(deep_data())
        .unwrap()
        .filter(col("division").eq(lit("Enterprise")))
        .unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .root_path_id("division=Enterprise")
            .display_levels(2),
    )
    .data(df)
    .plot_size(560.0, 320.0)
    .mark(
        TreeRect::new()
            .data(mark_df)
            .fill(col("region"))
            .stroke("#ffffff"),
    )
    .mark(TreeLabel::new().color("#ffffff"));

    assert_visual_match(plot, "treemap_leaf_labels_zoom_window").await;
}

#[tokio::test]
async fn treemap_reserved_header_space() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(deep_data()).unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .display_levels(3)
            .header_bars(TreemapHeaderBars::enabled().height_px(24.0)),
    )
    .data(df)
    .plot_size(560.0, 320.0)
    .configure_guide(
        TreemapGuide::new()
            .headers(true)
            .separators(true)
            .breadcrumbs(false),
    )
    .mark(TreeRect::new().fill(col("region")).stroke("#ffffff"));

    assert_visual_match(plot, "treemap_reserved_header_space").await;
}

#[tokio::test]
async fn treemap_header_geometry_small_groups() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(tiny_group_data()).unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "product"])
            .value(sum(col("sales")))
            .header_bars(
                TreemapHeaderBars::enabled()
                    .height_px(22.0)
                    .min_size_px(72.0, 34.0),
            ),
    )
    .data(df)
    .plot_size(480.0, 240.0)
    .mark(TreeRect::new().fill(col("division")).stroke("#ffffff"))
    .mark(
        TreeHeader::new()
            .fill(col("division"))
            .stroke("#ffffff")
            .text_color("#ffffff"),
    )
    .mark(TreeLabel::new().color("#ffffff"));

    assert_visual_match(plot, "treemap_header_geometry_small_groups").await;
}

#[tokio::test]
async fn treemap_group_header_bars() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(deep_data()).unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .display_levels(3)
            .header_bars(TreemapHeaderBars::enabled().height_px(24.0)),
    )
    .data(df)
    .plot_size(560.0, 320.0)
    .mark(TreeRect::new().fill(col("region")).stroke("#ffffff"))
    .mark(
        TreeHeader::new()
            .fill(col("division"))
            .stroke("#ffffff")
            .text_color("#ffffff"),
    )
    .mark(TreeLabel::new().color("#ffffff"));

    assert_visual_match(plot, "treemap_group_header_bars").await;
}

#[tokio::test]
async fn treemap_group_header_bars_long_labels() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(long_label_data()).unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .display_levels(3)
            .header_bars(TreemapHeaderBars::enabled().height_px(24.0)),
    )
    .data(df)
    .plot_size(620.0, 340.0)
    .mark(TreeRect::new().fill(col("region")).stroke("#ffffff"))
    .mark(
        TreeHeader::new()
            .fill(col("division"))
            .stroke("#ffffff")
            .text_color("#ffffff"),
    )
    .mark(TreeLabel::new().color("#ffffff"));

    assert_visual_match(plot, "treemap_group_header_bars_long_labels").await;
}

#[tokio::test]
async fn treemap_group_header_bars_color_legend() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(deep_data()).unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .display_levels(3)
            .header_bars(TreemapHeaderBars::enabled().height_px(24.0)),
    )
    .data(df)
    .canvas_size(680.0, 340.0)
    .mark(
        TreeHeader::new()
            .fill_with(col("division"), |fill| {
                fill.legend(|legend| legend.title("Division"))
            })
            .stroke("#ffffff")
            .text_color("#ffffff"),
    )
    .mark(TreeRect::new().fill("#dbeafe").stroke("#ffffff"))
    .mark(TreeLabel::new().color("#1f2937"));

    assert_visual_match(plot, "treemap_group_header_bars_color_legend").await;
}

#[tokio::test]
async fn treemap_depth_gaps_show_hierarchy() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(deep_data()).unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .display_levels(3)
            .padding(TreemapPadding::default().depth_inner_px([10.0, 3.0, 1.0])),
    )
    .data(df)
    .plot_size(560.0, 320.0)
    .mark(TreeRect::new().fill(col("region")).stroke("#ffffff"))
    .mark(TreeLabel::new().color("#ffffff"));

    assert_visual_match(plot, "treemap_depth_gaps_show_hierarchy").await;
}

#[tokio::test]
async fn treemap_multi_level_headers_depth_limited() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(deep_data()).unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .display_levels(4)
            .header_bars(
                TreemapHeaderBars::enabled()
                    .height_px(18.0)
                    .depth_range(1..=2)
                    .min_size_px(42.0, 28.0),
            ),
    )
    .data(df)
    .plot_size(620.0, 360.0)
    .mark(TreeRect::new().fill(col("team")).stroke("#ffffff"))
    .mark(
        TreeHeader::new()
            .depth_range(1..=2)
            .fill(col("region"))
            .stroke("#ffffff")
            .text_color("#ffffff")
            .font_size(12.0),
    )
    .mark(TreeLabel::new().font_size(11.0).color("#ffffff"));

    assert_visual_match(plot, "treemap_multi_level_headers_depth_limited").await;
}

#[tokio::test]
async fn treemap_tiny_groups_hide_headers_and_labels() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(tiny_group_data()).unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "product"])
            .value(sum(col("sales")))
            .header_bars(
                TreemapHeaderBars::enabled()
                    .height_px(24.0)
                    .min_size_px(70.0, 40.0),
            ),
    )
    .data(df)
    .plot_size(360.0, 220.0)
    .mark(TreeRect::new().fill(col("division")).stroke("#ffffff"))
    .mark(
        TreeHeader::new()
            .fill(col("division"))
            .stroke("#ffffff")
            .text_color("#ffffff"),
    )
    .mark(TreeLabel::new().min_size_px(90.0, 28.0).color("#ffffff"));

    assert_visual_match(plot, "treemap_tiny_groups_hide_headers_and_labels").await;
}

#[tokio::test]
async fn treemap_zoom_window_breadcrumbs() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(deep_data()).unwrap();
    let mark_df = ctx
        .read_batch(deep_data())
        .unwrap()
        .filter(col("division").eq(lit("Enterprise")))
        .unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .root_path_id("division=Enterprise")
            .display_levels(2),
    )
    .data(df)
    .plot_size(560.0, 320.0)
    .configure_guide(
        TreemapGuide::new()
            .headers(true)
            .separators(true)
            .breadcrumbs(true),
    )
    .mark(
        TreeRect::new()
            .data(mark_df)
            .fill(col("region"))
            .stroke("#ffffff"),
    );

    assert_visual_match(plot, "treemap_zoom_window_breadcrumbs").await;
}

#[tokio::test]
async fn treemap_internal_node_depth_mode() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(deep_data()).unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales"))),
    )
    .data(df)
    .plot_size(480.0, 280.0)
    .mark(
        TreeRect::new()
            .depth(2)
            .fill(col("region"))
            .stroke("#ffffff"),
    );

    assert_visual_match(plot, "treemap_internal_node_depth_mode").await;
}

#[tokio::test]
async fn treemap_faceted_independent_layout_scope() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(facet_data()).unwrap();
    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .canvas_size(640.0, 320.0)
        .mark(
            Subplot::new(
                Plot::with_coord(
                    Treemap::new()
                        .path_columns(["division", "product"])
                        .value(sum(col("sales"))),
                )
                .mark(TreeRect::new().id("base").fill("#d8dde3").stroke("#ffffff"))
                .mark(
                    TreeRect::new()
                        .id("selected")
                        .data(
                            ctx.read_batch(facet_data())
                                .unwrap()
                                .filter(col("division").eq(lit("Enterprise")))
                                .unwrap(),
                        )
                        .fill("#2f80ed")
                        .opacity(0.82)
                        .stroke("#ffffff"),
                ),
            )
            .column_with(col("market"), |c| c.guide(|g| g.title("Market"))),
        );

    assert_visual_match(plot, "treemap_faceted_independent_layout_scope").await;
}

#[tokio::test]
async fn treemap_long_labels_headers_breadcrumbs() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(long_label_data()).unwrap();
    let mark_df = ctx
        .read_batch(long_label_data())
        .unwrap()
        .filter(col("division").eq(lit("International growth markets")))
        .unwrap();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .root_path_id("division=International growth markets")
            .display_levels(2),
    )
    .data(df)
    .plot_size(620.0, 340.0)
    .configure_guide(
        TreemapGuide::new()
            .headers(true)
            .separators(true)
            .breadcrumbs(true),
    )
    .mark(
        TreeRect::new()
            .data(mark_df)
            .fill(col("region"))
            .stroke("#ffffff"),
    );

    assert_visual_match(plot, "treemap_long_labels_headers_breadcrumbs").await;
}

async fn assert_visual_match<C>(plot: Plot<C>, baseline_name: &str)
where
    C: CoordinateSystem,
{
    let ctx = SessionContext::new();
    let compiled = plot.compile(&ctx).await.expect("compile treemap plot");
    let evaluated = compiled
        .evaluate(&ctx, None)
        .await
        .expect("evaluate treemap plot");
    let image = render_scene_graph_to_wgpu_image(&evaluated.scene_graph).await;
    let baseline_path = PathBuf::from(BASELINE_DIR).join(format!("{baseline_name}.png"));
    if std::env::var_os(BLESS_ENV).is_some() {
        save_image(&baseline_path, &image);
        return;
    }
    compare_image(&baseline_path, baseline_name, &image, DEFAULT_THRESHOLD);
}

async fn render_scene_graph_to_wgpu_image(scene_graph: &SceneGraph) -> RgbaImage {
    let dimensions = CanvasDimensions {
        size: [scene_graph.width, scene_graph.height],
        scale: DEFAULT_SCALE,
    };

    let mut canvas = PngCanvas::new(dimensions, CanvasConfig::default())
        .await
        .expect("create visual test canvas");
    canvas.set_scene(scene_graph).expect("set scene graph");
    canvas.render().await.expect("render scene graph")
}

fn compare_image(baseline_path: &Path, baseline_name: &str, actual: &RgbaImage, threshold: f64) {
    let actual_path = PathBuf::from(FAILURE_DIR).join(format!("{baseline_name}_actual.png"));
    let diff_path = PathBuf::from(FAILURE_DIR).join(format!("{baseline_name}_diff.png"));
    if !baseline_path.exists() {
        save_image(&actual_path, actual);
        panic!(
            "No treemap visual baseline found at '{}'. Actual saved to '{}'. Run with {BLESS_ENV}=1 to bless.",
            baseline_path.display(),
            actual_path.display()
        );
    }

    let expected = image::open(baseline_path)
        .unwrap_or_else(|err| panic!("load baseline '{}': {err}", baseline_path.display()))
        .into_rgba8();
    if expected.dimensions() != actual.dimensions() {
        save_image(&actual_path, actual);
        panic!(
            "Treemap baseline dimensions differ for {baseline_name}: expected {:?}, actual {:?}. Actual saved to '{}'.",
            expected.dimensions(),
            actual.dimensions(),
            actual_path.display()
        );
    }

    let result = image_compare::rgba_hybrid_compare(&expected, actual)
        .unwrap_or_else(|err| panic!("compare {baseline_name}: {err}"));
    if result.score < threshold {
        save_image(&actual_path, actual);
        save_image(&diff_path, &result.image.to_color_map().into_rgba8());
        panic!(
            "Treemap baseline {baseline_name} score {:.6} is below threshold {:.6}. Actual saved to '{}', diff saved to '{}'.",
            result.score,
            threshold,
            actual_path.display(),
            diff_path.display()
        );
    }
}

fn save_image(path: &Path, image: &RgbaImage) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|err| panic!("create {}: {err}", parent.display()));
    }
    image
        .save(path)
        .unwrap_or_else(|err| panic!("save {}: {err}", path.display()));
}

fn sales_data() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("division", DataType::Utf8, false),
            Field::new("product", DataType::Utf8, false),
            Field::new("sales", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec![
                "North America",
                "North America",
                "International",
                "International",
            ])),
            Arc::new(StringArray::from(vec![
                "Platform", "Services", "Platform", "Services",
            ])),
            Arc::new(Float64Array::from(vec![42.0, 28.0, 35.0, 25.0])),
        ],
    )
    .expect("sales data")
}

fn segment_data() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("division", DataType::Utf8, false),
            Field::new("product", DataType::Utf8, false),
            Field::new("segment", DataType::Utf8, false),
            Field::new("sales", DataType::Float64, false),
            Field::new("v0", DataType::Float64, false),
            Field::new("v1", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec![
                "North America",
                "North America",
                "North America",
                "International",
                "International",
            ])),
            Arc::new(StringArray::from(vec![
                "Platform", "Platform", "Services", "Platform", "Services",
            ])),
            Arc::new(StringArray::from(vec![
                "Enterprise",
                "Consumer",
                "Enterprise",
                "Enterprise",
                "Consumer",
            ])),
            Arc::new(Float64Array::from(vec![30.0, 12.0, 28.0, 35.0, 25.0])),
            Arc::new(Float64Array::from(vec![0.0, 0.7142857, 0.0, 0.0, 0.0])),
            Arc::new(Float64Array::from(vec![0.7142857, 1.0, 1.0, 1.0, 1.0])),
        ],
    )
    .expect("segment data")
}

fn label_elide_data() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("division", DataType::Utf8, false),
            Field::new("product", DataType::Utf8, false),
            Field::new("sales", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec![
                "Enterprise",
                "Enterprise",
                "Consumer",
                "Consumer",
                "Consumer",
            ])),
            Arc::new(StringArray::from(vec![
                "Analytics platform",
                "Cloud infrastructure services",
                "Retail marketplace operations",
                "Extremely tiny cell label",
                "Another very small category",
            ])),
            Arc::new(Float64Array::from(vec![45.0, 30.0, 18.0, 4.0, 3.0])),
        ],
    )
    .expect("label elide data")
}

fn tiny_group_data() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("division", DataType::Utf8, false),
            Field::new("product", DataType::Utf8, false),
            Field::new("sales", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec![
                "Major platform",
                "Major platform",
                "Major platform",
                "Long tail",
                "Long tail",
                "Tiny",
                "Tiny",
            ])),
            Arc::new(StringArray::from(vec![
                "Core analytics",
                "Workflow automation",
                "Enterprise support",
                "Partner portal",
                "Developer tools",
                "Archive",
                "Labs",
            ])),
            Arc::new(Float64Array::from(vec![
                60.0, 38.0, 24.0, 7.0, 5.0, 1.2, 0.8,
            ])),
        ],
    )
    .expect("tiny group data")
}

fn deep_data() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("division", DataType::Utf8, false),
            Field::new("region", DataType::Utf8, false),
            Field::new("team", DataType::Utf8, false),
            Field::new("product", DataType::Utf8, false),
            Field::new("sales", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec![
                "Enterprise",
                "Enterprise",
                "Enterprise",
                "Enterprise",
                "Enterprise",
                "Enterprise",
                "Consumer",
                "Consumer",
            ])),
            Arc::new(StringArray::from(vec![
                "North America",
                "North America",
                "North America",
                "International",
                "International",
                "International",
                "North America",
                "International",
            ])),
            Arc::new(StringArray::from(vec![
                "Platform",
                "Services",
                "Operations",
                "Platform",
                "Services",
                "Operations",
                "Retail",
                "Retail",
            ])),
            Arc::new(StringArray::from(vec![
                "Core",
                "Support",
                "Automation",
                "Core",
                "Support",
                "Automation",
                "Storefront",
                "Marketplace",
            ])),
            Arc::new(Float64Array::from(vec![
                42.0, 28.0, 18.0, 35.0, 25.0, 20.0, 30.0, 22.0,
            ])),
        ],
    )
    .expect("deep data")
}

fn facet_data() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("market", DataType::Utf8, false),
            Field::new("division", DataType::Utf8, false),
            Field::new("product", DataType::Utf8, false),
            Field::new("sales", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec![
                "North", "North", "North", "North", "South", "South", "South", "South",
            ])),
            Arc::new(StringArray::from(vec![
                "Enterprise",
                "Enterprise",
                "Consumer",
                "Consumer",
                "Enterprise",
                "Enterprise",
                "Consumer",
                "Consumer",
            ])),
            Arc::new(StringArray::from(vec![
                "Platform", "Services", "Retail", "Support", "Platform", "Services", "Retail",
                "Support",
            ])),
            Arc::new(Float64Array::from(vec![
                60.0, 20.0, 15.0, 5.0, 18.0, 52.0, 24.0, 16.0,
            ])),
        ],
    )
    .expect("facet data")
}

fn long_label_data() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("division", DataType::Utf8, false),
            Field::new("region", DataType::Utf8, false),
            Field::new("team", DataType::Utf8, false),
            Field::new("product", DataType::Utf8, false),
            Field::new("sales", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec![
                "International growth markets",
                "International growth markets",
                "International growth markets",
                "International growth markets",
                "International growth markets",
                "North America enterprise",
                "North America enterprise",
                "North America enterprise",
            ])),
            Arc::new(StringArray::from(vec![
                "Asia Pacific strategic accounts",
                "Asia Pacific strategic accounts",
                "Europe partner expansion",
                "Europe partner expansion",
                "Latin America emerging channels",
                "United States platform group",
                "Canada customer operations",
                "Mexico growth services",
            ])),
            Arc::new(StringArray::from(vec![
                "Analytics platform",
                "Cloud infrastructure",
                "Customer operations",
                "Partner success",
                "Marketplace team",
                "Core platform",
                "Support team",
                "Services team",
            ])),
            Arc::new(StringArray::from(vec![
                "Workspace",
                "Pipeline",
                "Renewals",
                "Enablement",
                "Localization",
                "Developer tools",
                "Premium care",
                "Implementation",
            ])),
            Arc::new(Float64Array::from(vec![
                42.0, 28.0, 35.0, 18.0, 24.0, 50.0, 22.0, 20.0,
            ])),
        ],
    )
    .expect("long label data")
}
