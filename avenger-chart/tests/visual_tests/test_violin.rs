use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::{
    array::{Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use datafusion::dataframe::DataFrame;
use datafusion::prelude::{SessionContext, col, lit};
use std::sync::Arc;

fn grouped_violin_data(ctx: &SessionContext) -> DataFrame {
    let mut groups = Vec::<String>::new();
    let mut values = Vec::<f64>::new();

    let mut add_cluster = |group: &str, center: f64, spread: f64, count: usize, phase: f64| {
        for i in 0..count {
            let t = i as f64;
            let jitter =
                ((t * 1.618 + phase).sin() * 0.55 + (t * 0.73 + phase).cos() * 0.25) * spread;
            groups.push(group.to_string());
            values.push(center + jitter);
        }
    };

    add_cluster("Alpha", -1.05, 0.72, 18, 0.1);
    add_cluster("Alpha", 1.08, 0.52, 18, 1.3);
    add_cluster("Beta", 0.2, 0.62, 44, 2.1);
    add_cluster("Gamma", 0.95, 1.35, 30, 3.4);

    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("group", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(groups)) as _,
            Arc::new(Float64Array::from(values)) as _,
        ],
    )
    .expect("grouped violin batch");
    ctx.read_batch(batch).expect("grouped violin dataframe")
}

fn nested_violin_data(ctx: &SessionContext) -> DataFrame {
    let mut markets = Vec::<String>::new();
    let mut divisions = Vec::<String>::new();
    let mut teams = Vec::<String>::new();
    let mut values = Vec::<f64>::new();

    let mut add_cluster =
        |market: &str, division: &str, team: &str, center: f64, spread: f64, count: usize| {
            let phase = (market.len() + division.len() * 3 + team.len() * 7) as f64 * 0.19;
            for i in 0..count {
                let t = i as f64;
                let wave = (t * 1.37 + phase).sin() * 0.62 + (t * 0.61 + phase * 0.7).cos() * 0.28;
                markets.push(market.to_string());
                divisions.push(division.to_string());
                teams.push(team.to_string());
                values.push(center + wave * spread);
            }
        };

    add_cluster("North", "Platform", "Alpha", -0.95, 0.64, 18);
    add_cluster("North", "Platform", "Beta", 0.48, 0.72, 38);
    add_cluster("North", "Experience", "Alpha", 1.05, 0.52, 24);
    add_cluster("North", "Experience", "Gamma", 1.72, 0.48, 12);
    add_cluster("South", "Platform", "Alpha", -0.62, 0.56, 13);
    add_cluster("South", "Platform", "Gamma", 0.08, 0.62, 28);
    add_cluster("South", "Experience", "Beta", 1.18, 0.74, 44);
    add_cluster("South", "Experience", "Gamma", 1.92, 0.58, 20);

    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("market", DataType::Utf8, false),
            Field::new("division", DataType::Utf8, false),
            Field::new("team", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(markets)) as _,
            Arc::new(StringArray::from(divisions)) as _,
            Arc::new(StringArray::from(teams)) as _,
            Arc::new(Float64Array::from(values)) as _,
        ],
    )
    .expect("nested violin batch");
    ctx.read_batch(batch).expect("nested violin dataframe")
}

fn vertical_grouped_violin() -> Violin {
    Violin::new()
        .x_with(col("group"), |x| {
            x.scale_with::<Band>(|scale| scale.padding_inner(0.18))
                .axis(|axis| axis.title("Group"))
        })
        .y_with(col("value"), |y| {
            y.axis(|axis| axis.title("Value").grid(true))
        })
        .bandwidth(0.34)
        .steps(120)
        .density_extent(-2.8, 3.2)
        .density_extent_resolve(KdeResolve::Shared)
        .width(0.78)
        .fill_with(col("group"), |fill| {
            fill.legend(|legend| legend.title("Group"))
        })
        .stroke("#172033")
        .stroke_width(1.0)
        .opacity(0.72)
}

fn nested_violin_mark() -> Violin {
    Violin::new()
        .x_with(nested(["division", "team"]), |x| {
            x.axis(|axis| axis.title("Team grouped by division").grid(false))
                .level(0, |level| level.padding_inner(0.34).padding_outer(0.12))
                .level(1, |level| {
                    level.nest_scope(NestScope::Shared).padding_inner(0.1)
                })
        })
        .y_with(col("value"), |y| {
            y.scale(|scale| scale.domain((-2.4, 3.0)))
                .axis(|axis| axis.title("Value").grid(true))
        })
        .counts(true)
        .bandwidth(0.3)
        .steps(120)
        .density_extent(-2.4, 3.0)
        .density_extent_resolve(KdeResolve::Shared)
        .width(0.84)
        .fill_with(col("division"), |fill| {
            fill.legend(|legend| legend.title("Division"))
        })
        .stroke("#1f2937")
        .stroke_width(0.9)
        .opacity(0.72)
}

#[tokio::test]
async fn violin_compound_vertical_grouped() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Violin compound mark")
        .subtitle("KDE density is normalized by one max across groups")
        .canvas_size(720.0, 440.0)
        .data(grouped_violin_data(&ctx))
        .mark(vertical_grouped_violin());

    let compiled = plot.compile(&ctx).await.expect("compile violin");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "violin",
        "violin_compound_vertical_grouped",
    )
    .await;
}

#[tokio::test]
async fn violin_compound_horizontal_grouped() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Horizontal violin compound mark")
        .canvas_size(720.0, 440.0)
        .data(grouped_violin_data(&ctx))
        .mark(
            Violin::new()
                .horizontal()
                .x_with(col("value"), |x| {
                    x.axis(|axis| axis.title("Value").grid(true))
                })
                .y_with(col("group"), |y| {
                    y.scale_with::<Band>(|scale| scale.padding_inner(0.18))
                        .axis(|axis| axis.title("Group"))
                })
                .bandwidth(0.34)
                .steps(120)
                .density_extent(-2.8, 3.2)
                .density_extent_resolve(KdeResolve::Shared)
                .width(0.78)
                .fill_with(col("group"), |fill| {
                    fill.legend(|legend| legend.title("Group"))
                })
                .stroke("#172033")
                .stroke_width(1.0)
                .opacity(0.72),
        );

    let compiled = plot.compile(&ctx).await.expect("compile violin");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "violin",
        "violin_compound_horizontal_grouped",
    )
    .await;
}

#[tokio::test]
async fn violin_compound_nested_band_coarse_fill() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Nested violin compound mark")
        .subtitle("Fill is coarser than the detail fields that split each body")
        .canvas_size(800.0, 460.0)
        .data(nested_violin_data(&ctx))
        .mark(nested_violin_mark());

    let compiled = plot.compile(&ctx).await.expect("compile nested violin");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "violin",
        "violin_compound_nested_band_coarse_fill",
    )
    .await;
}

#[tokio::test]
async fn violin_compound_nested_faceted_shared_widths() {
    let ctx = SessionContext::new();
    let leaf = Plot::<Cartesian>::new().mark(
        nested_violin_mark()
            .density_data_scope(CoordinationScope::Shared)
            .x_with(nested(["division", "team"]), |x| {
                x.axis(|axis| axis.title("Team grouped by division").grid(false))
                    .level(0, |level| {
                        level
                            .domain_scope(CoordinationScope::Shared)
                            .padding_inner(0.34)
                            .padding_outer(0.12)
                    })
                    .level(1, |level| {
                        level
                            .domain_scope(CoordinationScope::Shared)
                            .nest_scope(NestScope::Shared)
                            .padding_inner(0.1)
                    })
            })
            .y_with(col("value"), |y| {
                y.with_domain_scope(CoordinationScope::Shared)
                    .scale(|scale| scale.domain((-2.4, 3.0)))
                    .axis(|axis| axis.title("Value").grid(true))
            })
            .fill_with(col("division"), |fill| {
                fill.with_domain_scope(CoordinationScope::Shared)
                    .legend(|legend| legend.title("Division"))
            }),
    );
    let plot = Chart::<FacetColumn>::new()
        .title("Faceted nested violin compound mark")
        .subtitle("KDE and max-density normalization run at shared facet scope")
        .canvas_size(920.0, 440.0)
        .data(nested_violin_data(&ctx))
        .mark(Subplot::new(leaf).column(col("market")));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile faceted nested violin");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "violin",
        "violin_compound_nested_faceted_shared_widths",
    )
    .await;
}

#[tokio::test]
async fn violin_compound_per_violin_width_normalization() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Per-violin width normalization")
        .subtitle("Each group reaches the configured maximum band width")
        .canvas_size(720.0, 440.0)
        .data(grouped_violin_data(&ctx))
        .mark(
            vertical_grouped_violin()
                .width_normalization(ViolinWidthNormalization::PerViolin)
                .fill("#a7f3d0")
                .stroke("#065f46"),
        );

    let compiled = plot.compile(&ctx).await.expect("compile violin");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "violin",
        "violin_compound_per_violin_width_normalization",
    )
    .await;
}

#[tokio::test]
async fn violin_compound_event_targets_resolve_body_part() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .data(grouped_violin_data(&ctx))
        .mark(vertical_grouped_violin().id("my_violin"))
        .event_binding(ChartEventBinding::on_between_end(
            ChartEventStream::on(ChartEventType::MouseDown).mark("my_violin"),
            ChartEventStream::on(ChartEventType::MouseUp),
        ))
        .event_binding(ChartEventBinding::on_between_end(
            ChartEventStream::on(ChartEventType::MouseDown).mark("my_violin.body"),
            ChartEventStream::on(ChartEventType::MouseUp),
        ));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile violin target paths");
    let resolved_start_paths = compiled
        .event_bindings()
        .iter()
        .map(|binding| {
            binding
                .between
                .as_ref()
                .expect("between binding")
                .start
                .resolved_mark_paths()
                .map(|paths| paths.to_vec())
                .expect("resolved mark paths")
        })
        .collect::<Vec<_>>();
    assert_eq!(resolved_start_paths, vec![vec![vec![0]], vec![vec![0]]]);
}

#[tokio::test]
async fn violin_compound_scene_query_targets_resolve_body_part() {
    let ctx = SessionContext::new();
    let compiled = Chart::<Cartesian>::new()
        .data(grouped_violin_data(&ctx))
        .mark(vertical_grouped_violin().id("my_violin"))
        .selection(Selection::new("picked"))
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click).set_selection(
                "picked",
                SelectionUpdate::replace_all_from_scene_query(SelectionSceneQuery::new(
                    SceneGeometryQuery::rect(lit(0.0), lit(0.0), lit(10.0), lit(10.0))
                        .mark("my_violin.body")
                        .datum_field(SceneQueryDatumField::new("group")),
                )),
            ),
        )
        .compile(&ctx)
        .await
        .expect("compile violin scene query target");

    let binding = compiled.event_bindings().first().expect("event binding");
    let SelectionUpdate::ReplaceAllFromSceneQuery { query } =
        &binding.selection_assignments[0].update
    else {
        panic!("expected scene query selection update");
    };
    assert_eq!(
        query.query.target.resolved_mark_paths(),
        Some(&[vec![0usize]][..])
    );
}
