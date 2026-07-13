use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use avenger_chart::{
    channel::LegendableChannel,
    marks::symbol::Symbol,
    plot::{Chart, CompiledPlot, EvaluationRequest, SelectionAssignment, SelectionStateUpdate},
    prelude::{
        ChartWidgetPlacementExt, ChromePosition, LegendPosition, Theme, WidgetItemRow, WidgetItems,
    },
    zerod::ZeroDCoord,
};
use avenger_chart_core::{
    CoordinationScope, ResolvedSelectionClauseScope, SelectionClause, SelectionClauseUpdate,
    SelectionEqualityDimensionValue, SelectionPredicateSpec, SelectionPredicateUpdate,
};
use avenger_chart_widgets::{Button, ButtonVariant, Checkbox, CheckboxList};
use avenger_common::canvas::CanvasDimensions;
use avenger_scenegraph::scene_graph::SceneGraph;
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
