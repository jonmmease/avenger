use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use avenger_chart::{
    cartesian::CartesianSymbolPositionChannels,
    channel::LegendableChannel,
    marks::symbol::Symbol,
    plot::{Chart, CompiledPlot, EvaluationRequest, SelectionAssignment, SelectionStateUpdate},
    prelude::{
        Cartesian, ChartWidgetPlacementExt, ChromePosition, HConcat, LegendPosition, Plot,
        Selection, Subplot, Theme, TrackSizing, WidgetCell, WidgetItemRow, WidgetItems,
    },
    transforms::Filter,
    zerod::ZeroDCoord,
};
use avenger_chart_core::{
    CoordinationScope, ResolvedSelectionClauseScope, SelectionClause, SelectionClauseUpdate,
    SelectionEqualityDimensionValue, SelectionPredicateSpec, SelectionPredicateUpdate,
};
use avenger_chart_widgets::{
    Button, ButtonVariant, Checkbox, CheckboxList, RadioButtonList, Slider,
};
use avenger_common::canvas::CanvasDimensions;
use avenger_scenegraph::{
    marks::{group::SceneGroup, mark::SceneMark, rect::SceneRectMark},
    scene_graph::SceneGraph,
};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::prelude::SessionContext;
use datafusion::prelude::col;
use image::RgbaImage;

const BASELINE_DIR: &str = "tests/baselines";
const FAILURE_DIR: &str = "tests/failures";
const BLESS_ENV: &str = "AVENGER_WIDGETS_BLESS_WGPU_BASELINES";
const DEFAULT_SCALE: f32 = 2.0;
const DEFAULT_THRESHOLD: f64 = 0.9999;

#[tokio::test]
async fn scaffold_harness_round_trip() {
    let ctx = SessionContext::new();
    let chart = Chart::<ZeroDCoord>::new()
        .canvas_size(96.0, 64.0)
        .plot_size(64.0, 32.0)
        .mark(
            Symbol::new()
                .size(256.0)
                .shape("square")
                .fill("#0072B2")
                .stroke("#202020")
                .stroke_width(1.0),
        );

    let compiled = chart.compile(&ctx).await.expect("compile smoke chart");
    let encoded = bincode::serialize(&compiled).expect("serialize compiled smoke chart");
    let decoded: CompiledPlot =
        bincode::deserialize(&encoded).expect("deserialize compiled smoke chart");

    let direct = compiled
        .evaluate(&ctx, None)
        .await
        .expect("evaluate direct smoke chart");
    let round_tripped = decoded
        .evaluate(&ctx, None)
        .await
        .expect("evaluate round-tripped smoke chart");
    let direct_image = render_scene_graph_to_wgpu_image(&direct.scene_graph).await;
    let round_trip_image = render_scene_graph_to_wgpu_image(&round_tripped.scene_graph).await;

    let equivalence = image_compare::rgba_hybrid_compare(&direct_image, &round_trip_image)
        .expect("compare direct and round-tripped images");
    assert_eq!(equivalence.score, 1.0, "round trip changed smoke scene");
    assert_visual_match("scaffold/harness_smoke", &direct_image);
}

#[tokio::test]
async fn checkbox_state_baselines() {
    for (scheme, theme) in [("light", Theme::light()), ("dark", Theme::dark())] {
        for (state, checked) in [("unchecked", false), ("checked", true)] {
            let ctx = SessionContext::new();
            let compiled = Chart::<ZeroDCoord>::new()
                .theme(theme.clone())
                .canvas_size(260.0, 96.0)
                .plot_size(128.0, 64.0)
                .mark(Symbol::new().size(196.0).fill("#0072B2"))
                .widget(Checkbox::new("regions", "Regions", checked).position(ChromePosition::Left))
                .compile(&ctx)
                .await
                .expect("compile checkbox baseline");
            assert_compiled_visual_match(
                &compiled,
                &ctx,
                &format!("checkbox/checkbox_{state}_{scheme}"),
            )
            .await;
        }
    }
}

#[tokio::test]
async fn button_baselines() {
    for (scheme, theme) in [("light", Theme::light()), ("dark", Theme::dark())] {
        let ctx = SessionContext::new();
        let compiled = Chart::<ZeroDCoord>::new()
            .theme(theme)
            .canvas_size(280.0, 96.0)
            .plot_size(128.0, 64.0)
            .mark(Symbol::new().size(196.0).fill("#0072B2"))
            .widget(
                Button::new("clear")
                    .label("Clear selection")
                    .variant(ButtonVariant::Accent)
                    .position(ChromePosition::Left),
            )
            .compile(&ctx)
            .await
            .expect("compile button baseline");
        assert_compiled_visual_match(
            &compiled,
            &ctx,
            &format!("button/clear_selection_button_{scheme}"),
        )
        .await;
    }
}

#[tokio::test]
async fn slider_baselines() {
    for (scheme, theme) in [("light", Theme::light()), ("dark", Theme::dark())] {
        let ctx = SessionContext::new();
        let compiled = Chart::<ZeroDCoord>::new()
            .theme(theme)
            .canvas_size(340.0, 112.0)
            .plot_size(128.0, 72.0)
            .mark(Symbol::new().size(196.0).fill("#0072B2"))
            .widget(
                Slider::new("volume", 0.0, 100.0)
                    .step(5.0)
                    .default(65.0)
                    .title("Volume")
                    .format(".0f")
                    .position(ChromePosition::Left),
            )
            .compile(&ctx)
            .await
            .expect("compile slider baseline");
        assert_compiled_visual_match(&compiled, &ctx, &format!("slider/slider_{scheme}")).await;
    }
}

#[tokio::test]
async fn checkbox_list_baselines() {
    for (scheme, theme) in [("light", Theme::light()), ("dark", Theme::dark())] {
        let ctx = SessionContext::new();
        let items = ["North", "South", "West"]
            .into_iter()
            .map(|region| {
                WidgetItemRow::new([
                    (
                        "value".to_string(),
                        datafusion::common::ScalarValue::Utf8(Some(region.to_lowercase())),
                    ),
                    (
                        "label".to_string(),
                        datafusion::common::ScalarValue::Utf8(Some(region.to_string())),
                    ),
                ])
            })
            .collect();
        let compiled = Chart::<ZeroDCoord>::new()
            .theme(theme)
            .canvas_size(320.0, 180.0)
            .plot_size(128.0, 96.0)
            .mark(Symbol::new().size(196.0).fill("#0072B2"))
            .widget(
                CheckboxList::new("regions", WidgetItems::Static(items))
                    .position(ChromePosition::Left),
            )
            .compile(&ctx)
            .await
            .expect("compile checkbox-list baseline");
        assert_compiled_visual_match(
            &compiled,
            &ctx,
            &format!("checkbox-list/checkbox_list_{scheme}"),
        )
        .await;
        assert_checkbox_list_selected_visual_match(
            &compiled,
            &format!("checkbox-list/checkbox_list_selected_{scheme}"),
        )
        .await;
    }
}

#[tokio::test]
async fn radio_button_list_baselines() {
    for (scheme, theme) in [("light", Theme::light()), ("dark", Theme::dark())] {
        let ctx = SessionContext::new();
        let items = ["North", "South", "West"]
            .into_iter()
            .map(|region| {
                WidgetItemRow::new([
                    (
                        "value".to_string(),
                        datafusion::common::ScalarValue::Utf8(Some(region.to_lowercase())),
                    ),
                    (
                        "label".to_string(),
                        datafusion::common::ScalarValue::Utf8(Some(region.to_string())),
                    ),
                ])
            })
            .collect();
        let compiled = Chart::<ZeroDCoord>::new()
            .theme(theme)
            .canvas_size(320.0, 180.0)
            .plot_size(128.0, 96.0)
            .mark(Symbol::new().size(196.0).fill("#0072B2"))
            .widget(
                RadioButtonList::new("regions", WidgetItems::Static(items))
                    .default("south")
                    .position(ChromePosition::Left),
            )
            .compile(&ctx)
            .await
            .expect("compile radio-button-list baseline");
        assert_compiled_visual_match(
            &compiled,
            &ctx,
            &format!("radio-button-list/radio_button_list_selected_{scheme}"),
        )
        .await;
    }
}

#[tokio::test]
async fn widget_top_slot_with_legend_baselines() {
    for (scheme, theme) in [("light", Theme::light()), ("dark", Theme::dark())] {
        let ctx = SessionContext::new();
        let data = ctx
            .sql("SELECT * FROM (VALUES ('North'), ('South'), ('West')) AS t(region)")
            .await
            .expect("build legend data");
        let compiled = Chart::<ZeroDCoord>::new()
            .theme(theme)
            .canvas_size(360.0, 220.0)
            .plot_size(180.0, 88.0)
            .data(data)
            .mark(
                Symbol::new()
                    .size(196.0)
                    .fill_with(col("region"), |channel| {
                        channel
                            .legend(|legend| legend.title("Region").position(LegendPosition::Top))
                    }),
            )
            .widget(Checkbox::new("labels", "Show labels", true).position(ChromePosition::Top))
            .compile(&ctx)
            .await
            .expect("compile top-slot baseline");
        assert_compiled_visual_match(
            &compiled,
            &ctx,
            &format!("chrome/widget_top_slot_with_legend_{scheme}"),
        )
        .await;
    }
}

#[tokio::test]
async fn custom_checkbox_button_baselines() {
    for (scheme, mut theme) in [("light", Theme::light()), ("dark", Theme::dark())] {
        theme
            .append_css(
                r#"
                checkbox#custom-choice {
                    height: 40px;
                    --widget-accent: #D55E00;
                }
                checkbox#custom-choice::part(box) {
                    choice-control-size: 18px;
                    corner-radius: 4px;
                    stroke-width: 2px;
                }
                checkbox#custom-choice::part(label) {
                    control-label-gap: 12px;
                    font-size: 16px;
                    font-weight: 500;
                }
                checkbox#custom-choice::part(focus-ring) { focus-gap: 3px; }
                button#custom-action {
                    height: 40px;
                    --widget-accent: #D55E00;
                }
                button#custom-action::part(box) {
                    button-min-width: 108px;
                    button-inline-padding: 22px;
                    corner-radius: 8px;
                    stroke-width: 2px;
                }
                button#custom-action::part(label) {
                    font-size: 16px;
                    font-weight: 700;
                }
                button#custom-action::part(focus-ring) { focus-gap: 3px; }
                "#,
            )
            .expect("append custom widget CSS");
        let ctx = SessionContext::new();
        let compiled = Chart::<ZeroDCoord>::new()
            .theme(theme)
            .canvas_size(440.0, 260.0)
            .plot_size(180.0, 176.0)
            .mark(Symbol::new().size(256.0).fill("#0072B2"))
            .widget(
                Checkbox::new("custom-choice", "Custom choice", true)
                    .position(ChromePosition::Left),
            )
            .widget(
                Checkbox::new("stock-choice", "Stock choice", true).position(ChromePosition::Left),
            )
            .widget(
                Button::new("custom-action")
                    .label("Custom action")
                    .variant(ButtonVariant::Accent)
                    .position(ChromePosition::Left),
            )
            .widget(
                Button::new("stock-action")
                    .label("Stock action")
                    .variant(ButtonVariant::Accent)
                    .position(ChromePosition::Left),
            )
            .compile(&ctx)
            .await
            .expect("compile custom widget baseline");
        assert_compiled_visual_match(
            &compiled,
            &ctx,
            &format!("theming/custom_checkbox_button_{scheme}"),
        )
        .await;
    }
}

#[tokio::test]
async fn custom_slider_baselines() {
    for (scheme, mut theme) in [("light", Theme::light()), ("dark", Theme::dark())] {
        theme
            .append_css(
                r#"
                slider#custom-slider {
                    height: 42px;
                    padding-inline: 6px;
                    --widget-accent: #D55E00;
                }
                slider#custom-slider::part(track) {
                    slider-min-width: 140px;
                    slider-track-height: 5px;
                }
                slider#custom-slider::part(handle) {
                    slider-handle-size: 22px;
                    slider-handle-border-width: 3px;
                }
                slider#custom-slider::part(label),
                slider#custom-slider::part(value-label) { font-size: 14px; }
                slider#custom-slider::part(value-label) { slider-value-padding: 18px; }
                "#,
            )
            .expect("append custom slider CSS");
        let ctx = SessionContext::new();
        let compiled = Chart::<ZeroDCoord>::new()
            .theme(theme)
            .canvas_size(400.0, 128.0)
            .plot_size(128.0, 80.0)
            .mark(Symbol::new().size(196.0).fill("#0072B2"))
            .widget(
                Slider::new("custom-slider", -20.0, 40.0)
                    .step(5.0)
                    .default(15.0)
                    .title("Temperature")
                    .format(".0f")
                    .position(ChromePosition::Left),
            )
            .compile(&ctx)
            .await
            .expect("compile custom slider baseline");
        assert_compiled_visual_match(&compiled, &ctx, &format!("theming/custom_slider_{scheme}"))
            .await;
    }
}

#[tokio::test]
async fn widget_cell_sidebar_baselines() {
    for (scheme, mut theme) in [("light", Theme::light()), ("dark", Theme::dark())] {
        theme
            .append_css(
                r#"
                checkbox-list#regions {
                    min-width: 148px;
                    height: 32px;
                    padding-inline: 12px;
                    padding-block: 10px;
                    item-gap: 0px;
                }
                checkbox-list#regions::part(container) {
                    fill: var(--widget-surface);
                    stroke: var(--widget-border);
                    stroke-width: 1px;
                    corner-radius: 4px;
                }
                "#,
            )
            .expect("append sidebar widget CSS");
        let ctx = Arc::new(SessionContext::new());
        let data = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('North', 1.0, 6.0), ('North', 2.0, 7.5), ('North', 3.0, 6.8),
                    ('South', 2.2, 2.0), ('South', 3.4, 3.1), ('South', 4.2, 2.4),
                    ('West',  5.0, 5.8), ('West',  6.1, 4.9), ('West',  7.0, 6.4)
                ) AS t(region, x_value, y_value)",
            )
            .await
            .expect("build sidebar scatter data");
        let regions = Selection::new("regions").empty_selects_all();
        let selected = regions.predicate();
        let scatter = Plot::<Cartesian>::new()
            .data(data)
            .mark(
                Symbol::new()
                    .x_with(col("x_value"), |x| x.axis(|axis| axis.title("X")))
                    .y_with(col("y_value"), |y| {
                        y.axis(|axis| axis.title("Y").grid(true))
                    })
                    .size(140.0)
                    .fill("#C8CDD2")
                    .stroke("#FFFFFF")
                    .stroke_width(1.0),
            )
            .mark(
                Symbol::new()
                    .transform_no_output(Filter::new(selected), |mark| mark)
                    .x(col("x_value"))
                    .y(col("y_value"))
                    .size(140.0)
                    .fill("#0072B2")
                    .stroke("#FFFFFF")
                    .stroke_width(1.0),
            );
        let compiled = Chart::<HConcat>::new()
            .theme(theme)
            .title("Regional performance")
            .canvas_size(800.0, 440.0)
            .plot_size(748.0, 332.0)
            .configure_coord(|coord| {
                coord
                    .widths([TrackSizing::Auto, TrackSizing::Flex(1.0)])
                    .spacing(24.0)
            })
            .mark(
                WidgetCell::widget(
                    CheckboxList::new("regions", region_items())
                        .value(col("region"))
                        .label(col("label"))
                        .selection(&regions),
                )
                .name("filters"),
            )
            .mark(Subplot::new(scatter).name("scatter"))
            .compile(ctx.as_ref())
            .await
            .expect("compile widget-cell sidebar baseline");
        let encoded = bincode::serialize(&compiled).expect("serialize sidebar baseline");
        let decoded: CompiledPlot =
            bincode::deserialize(&encoded).expect("deserialize sidebar baseline");
        let mut session = Arc::new(decoded).instantiate(ctx);
        seed_region_selection(&mut session);
        let evaluated = session
            .evaluate(EvaluationRequest::new())
            .await
            .expect("evaluate sidebar baseline");
        assert_sidebar_container_contract(&evaluated, scheme);
        let image = render_scene_graph_to_wgpu_image(&evaluated.scene_graph).await;
        assert_visual_match(&format!("cell/widget_cell_sidebar_{scheme}"), &image);
    }
}

fn region_items() -> WidgetItems {
    WidgetItems::Static(
        ["North", "South", "West"]
            .into_iter()
            .map(|region| {
                WidgetItemRow::new([
                    (
                        "region".to_string(),
                        datafusion::common::ScalarValue::Utf8(Some(region.to_string())),
                    ),
                    (
                        "label".to_string(),
                        datafusion::common::ScalarValue::Utf8(Some(region.to_string())),
                    ),
                ])
            })
            .collect(),
    )
}

fn seed_region_selection(session: &mut avenger_chart::plot::PlotSession) {
    let field_expr = match SelectionClauseUpdate::equality("seed")
        .dimension(col("region"), "North")
        .build()
        .predicate
    {
        SelectionPredicateUpdate::Equality { mut dimensions } => dimensions.remove(0).field_expr,
        _ => unreachable!("equality builder produced another predicate"),
    };
    session
        .apply_selection_patch(vec![SelectionAssignment {
            selection_id: "regions".to_string(),
            update: SelectionStateUpdate::ReplaceAllClauses {
                clauses: vec![SelectionClause {
                    id: "seed-north".to_string(),
                    scope: ResolvedSelectionClauseScope {
                        sharing: CoordinationScope::Shared,
                        owner_path: Vec::new(),
                    },
                    predicate: SelectionPredicateSpec::Equality {
                        dimensions: vec![SelectionEqualityDimensionValue {
                            id: "region".to_string(),
                            field_expr,
                            value: datafusion::common::ScalarValue::Utf8(Some("North".to_string())),
                        }],
                    },
                    facet_context: Vec::new(),
                }],
            },
        }])
        .expect("seed sidebar region selection");
}

fn assert_sidebar_container_contract(
    evaluated: &avenger_chart::render::EvaluatedPlot,
    scheme: &str,
) {
    let controls = find_group(&evaluated.scene_graph.marks, "filters").expect("filters cell");
    let list = find_group(&controls.marks, "regions").expect("regions widget");
    let container = find_rect(&list.marks, "container").expect("list container");
    let frame = &evaluated.widget_frames.by_widget_id["regions"].bounds;
    assert_eq!(container.x_vec(), vec![0.0]);
    assert_eq!(container.y_vec(), vec![0.0]);
    assert_eq!(container.x2_vec(), vec![frame.width]);
    assert_eq!(container.y2_vec(), vec![frame.height]);
    assert_eq!(container.stroke_width.as_vec(1, None), vec![1.0]);

    let expected_edge = match scheme {
        "light" => "#D7D7D7",
        "dark" => "#505050",
        _ => unreachable!("known color scheme"),
    };
    assert!(matches!(
        container.stroke.as_vec(1, None)[0],
        avenger_color::ColorOrGradient::Color(color)
            if color == avenger_color::parse_color_string(expected_edge).unwrap()
    ));

    let accent = avenger_color::parse_color_string("#0072B2").unwrap();
    assert!(
        list.marks.iter().all(|mark| {
            let SceneMark::Rect(rect) = mark else {
                return true;
            };
            let spans_width = rect
                .x_vec()
                .iter()
                .zip(rect.x2_vec())
                .any(|(x, x2)| (*x).abs() < f32::EPSILON && (x2 - frame.width).abs() < 0.01);
            let is_accent = rect.fill.as_vec(1, None).iter().any(|fill| {
            matches!(fill, avenger_color::ColorOrGradient::Color(color) if *color == accent)
        });
            !(spans_width && is_accent)
        }),
        "sidebar must not contain a full-width accent stripe"
    );
}

fn find_group<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneGroup> {
    for mark in marks {
        if let SceneMark::Group(group) = mark {
            if group.name == name {
                return Some(group);
            }
            if let Some(found) = find_group(&group.marks, name) {
                return Some(found);
            }
        }
    }
    None
}

fn find_rect<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneRectMark> {
    for mark in marks {
        match mark {
            SceneMark::Rect(rect) if rect.name == name => return Some(rect),
            SceneMark::Group(group) => {
                if let Some(found) = find_rect(&group.marks, name) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

async fn assert_compiled_visual_match(compiled: &CompiledPlot, ctx: &SessionContext, name: &str) {
    let encoded = bincode::serialize(compiled).expect("serialize compiled widget baseline");
    let decoded: CompiledPlot =
        bincode::deserialize(&encoded).expect("deserialize compiled widget baseline");
    let evaluated = decoded
        .evaluate(ctx, None)
        .await
        .expect("evaluate widget baseline");
    let image = render_scene_graph_to_wgpu_image(&evaluated.scene_graph).await;
    assert_visual_match(name, &image);
}

async fn assert_checkbox_list_selected_visual_match(compiled: &CompiledPlot, name: &str) {
    let encoded = bincode::serialize(compiled).expect("serialize selected checkbox-list baseline");
    let decoded: CompiledPlot =
        bincode::deserialize(&encoded).expect("deserialize selected checkbox-list baseline");
    let ctx = Arc::new(SessionContext::new());
    let mut session = Arc::new(decoded).instantiate(ctx);
    let field_expr = match SelectionClauseUpdate::equality("seed")
        .dimension(col("value"), "south")
        .build()
        .predicate
    {
        SelectionPredicateUpdate::Equality { mut dimensions } => dimensions.remove(0).field_expr,
        _ => unreachable!("equality builder produced another predicate"),
    };
    session
        .apply_selection_patch(vec![SelectionAssignment {
            selection_id: "regions__selection".to_string(),
            update: SelectionStateUpdate::ReplaceAllClauses {
                clauses: vec![SelectionClause {
                    id: "external-south".to_string(),
                    scope: ResolvedSelectionClauseScope {
                        sharing: CoordinationScope::Free,
                        owner_path: Vec::new(),
                    },
                    predicate: SelectionPredicateSpec::Equality {
                        dimensions: vec![SelectionEqualityDimensionValue {
                            id: "value".to_string(),
                            field_expr,
                            value: datafusion::common::ScalarValue::Utf8(Some("south".to_string())),
                        }],
                    },
                    facet_context: Vec::new(),
                }],
            },
        }])
        .expect("seed checkbox-list selection");
    let evaluated = session
        .evaluate(EvaluationRequest::new())
        .await
        .expect("evaluate selected checkbox-list baseline");
    let image = render_scene_graph_to_wgpu_image(&evaluated.scene_graph).await;
    assert_visual_match(name, &image);
}

async fn render_scene_graph_to_wgpu_image(scene_graph: &SceneGraph) -> RgbaImage {
    let dimensions = CanvasDimensions {
        size: [scene_graph.width, scene_graph.height],
        scale: DEFAULT_SCALE,
    };
    let mut canvas = PngCanvas::new(dimensions, CanvasConfig::default())
        .await
        .expect("create widget visual-test canvas");
    canvas
        .set_scene(scene_graph)
        .expect("set widget smoke scene");
    canvas.render().await.expect("render widget smoke scene")
}

fn assert_visual_match(name: &str, actual: &RgbaImage) {
    let baseline_path = PathBuf::from(BASELINE_DIR).join(format!("{name}.png"));
    if std::env::var_os(BLESS_ENV).is_some() {
        save_image(&baseline_path, actual);
        return;
    }

    let actual_path = PathBuf::from(FAILURE_DIR).join(format!("{name}_actual.png"));
    let diff_path = PathBuf::from(FAILURE_DIR).join(format!("{name}_diff.png"));
    if !baseline_path.exists() {
        save_image(&actual_path, actual);
        panic!(
            "No widget baseline at '{}'. Actual saved to '{}'. Bless with {BLESS_ENV}=1.",
            baseline_path.display(),
            actual_path.display()
        );
    }

    let expected = image::open(&baseline_path)
        .unwrap_or_else(|error| panic!("load '{}': {error}", baseline_path.display()))
        .into_rgba8();
    assert_eq!(
        expected.dimensions(),
        actual.dimensions(),
        "widget baseline dimensions differ for {name}"
    );
    let comparison =
        image_compare::rgba_hybrid_compare(&expected, actual).expect("compare widget baseline");
    if comparison.score < DEFAULT_THRESHOLD {
        save_image(&actual_path, actual);
        save_image(&diff_path, &comparison.image.to_color_map().into_rgba8());
        panic!(
            "Widget baseline {name} score {:.6} is below {:.6}. Actual: '{}'; diff: '{}'.",
            comparison.score,
            DEFAULT_THRESHOLD,
            actual_path.display(),
            diff_path.display()
        );
    }
}

fn save_image(path: &Path, image: &RgbaImage) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("create '{}': {error}", parent.display()));
    }
    image
        .save(path)
        .unwrap_or_else(|error| panic!("save '{}': {error}", path.display()));
}
