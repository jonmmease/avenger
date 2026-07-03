//! Group view scopes: authoring, compile-time lowering onto child marks,
//! validation, and serialization.

use avenger_chart::prelude::*;
use avenger_scenegraph::{marks::mark::SceneMark, scene_graph::SceneGraph};
use datafusion::prelude::{SessionContext, col, lit};

fn count_symbols(scene: &SceneGraph) -> usize {
    fn walk(mark: &SceneMark, total: &mut usize) {
        match mark {
            SceneMark::Group(group) => {
                for child in &group.marks {
                    walk(child, total);
                }
            }
            SceneMark::Symbol(symbol) => {
                *total += symbol.len as usize;
            }
            _ => {}
        }
    }
    let mut total = 0;
    for mark in &scene.marks {
        walk(mark, &mut total);
    }
    total
}

async fn xy_dataframe(ctx: &SessionContext, rows: usize) -> datafusion::dataframe::DataFrame {
    let values = (0..rows)
        .map(|index| format!("({}.0, {}.0)", index + 1, (index + 1) * 2))
        .collect::<Vec<_>>()
        .join(", ");
    ctx.sql(&format!("SELECT * FROM (VALUES {values}) AS t(x, y)"))
        .await
        .unwrap()
}

/// A group view scope lowers onto both children: the plot compiles, view
/// domains come from the view declarations, and both marks render.
#[tokio::test]
async fn group_view_lowers_onto_children() {
    let ctx = SessionContext::new();
    let df = xy_dataframe(&ctx, 5).await;
    let plot = Plot::<Cartesian>::new().mark(
        MarkGroup::<Cartesian>::new().data(df).view(
            View::cartesian()
                .id("pts")
                .x_domain(col("x"))
                .y_domain(col("y")),
            |group, _v| {
                group
                    .mark(Symbol::new().x(col("x")).y(col("y")).size(20.0))
                    .mark(
                        Symbol::new()
                            .x(col("x"))
                            .y(col("y") + lit(0.5))
                            .size(10.0),
                    )
            },
        ),
    );
    let compiled = plot.compile(&ctx).await.unwrap();
    let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
    assert_eq!(count_symbols(&evaluated.scene_graph), 10);
}

/// Group-view plots survive serialization: the compiled plot round-trips
/// through JSON and evaluates identically.
#[tokio::test]
async fn group_view_serialization_round_trip() {
    let ctx = SessionContext::new();
    let df = xy_dataframe(&ctx, 4).await;
    let plot = Plot::<Cartesian>::new().mark(
        MarkGroup::<Cartesian>::new().data(df).view(
            View::cartesian()
                .id("pts")
                .x_domain(col("x"))
                .y_domain(col("y")),
            |group, _v| group.mark(Symbol::new().x(col("x")).y(col("y")).size(20.0)),
        ),
    );
    let compiled = plot.compile(&ctx).await.unwrap();
    let direct = compiled.evaluate(&ctx, None).await.unwrap();

    let serialized = serde_json::to_string(&compiled).expect("serialize compiled plot");
    let deserialized: avenger_chart::plot::CompiledPlot =
        serde_json::from_str(&serialized).expect("deserialize compiled plot");
    let round_tripped = deserialized.evaluate(&ctx, None).await.unwrap();

    assert_eq!(
        count_symbols(&direct.scene_graph),
        count_symbols(&round_tripped.scene_graph)
    );
    assert_eq!(count_symbols(&round_tripped.scene_graph), 4);
}

/// A mark-level view scope inside a viewed group is rejected.
#[tokio::test]
async fn mark_view_inside_group_view_errors() {
    let ctx = SessionContext::new();
    let df = xy_dataframe(&ctx, 3).await;
    let plot = Plot::<Cartesian>::new().mark(
        MarkGroup::<Cartesian>::new().data(df).view(
            View::cartesian()
                .id("outer")
                .x_domain(col("x"))
                .y_domain(col("y")),
            |group, _v| {
                group.mark(Symbol::new().view(
                    View::cartesian()
                        .id("inner")
                        .x_domain(col("x"))
                        .y_domain(col("y")),
                    |mark, _v| mark.x(col("x")).y(col("y")),
                ))
            },
        ),
    );
    let err = plot
        .compile(&ctx)
        .await
        .err()
        .expect("nested mark view should fail");
    assert!(
        err.to_string().contains("nested view scopes are not supported"),
        "unexpected error: {err}"
    );
}

/// A viewed group inside a viewed group is rejected.
#[tokio::test]
async fn nested_group_views_error() {
    let ctx = SessionContext::new();
    let df = xy_dataframe(&ctx, 3).await;
    let plot = Plot::<Cartesian>::new().mark(
        MarkGroup::<Cartesian>::new().data(df).view(
            View::cartesian()
                .id("outer")
                .x_domain(col("x"))
                .y_domain(col("y")),
            |group, _v| {
                group.mark(MarkGroup::<Cartesian>::new().view(
                    View::cartesian()
                        .id("inner")
                        .x_domain(col("x"))
                        .y_domain(col("y")),
                    |group, _v| group.mark(Symbol::new().x(col("x")).y(col("y"))),
                ))
            },
        ),
    );
    let err = plot
        .compile(&ctx)
        .await
        .err()
        .expect("nested group views should fail");
    assert!(
        err.to_string().contains("nested view scopes are not supported"),
        "unexpected error: {err}"
    );
}

/// Duplicate view ids across the plot are rejected.
#[tokio::test]
async fn duplicate_view_ids_error() {
    let ctx = SessionContext::new();
    let df = xy_dataframe(&ctx, 3).await;
    let group = |df: datafusion::dataframe::DataFrame| {
        MarkGroup::<Cartesian>::new().data(df).view(
            View::cartesian()
                .id("pts")
                .x_domain(col("x"))
                .y_domain(col("y")),
            |group, _v| group.mark(Symbol::new().x(col("x")).y(col("y"))),
        )
    };
    let plot = Plot::<Cartesian>::new()
        .mark(group(df.clone()))
        .mark(group(df));
    let err = plot
        .compile(&ctx)
        .await
        .err()
        .expect("duplicate view ids should fail");
    assert!(
        err.to_string().contains("Duplicate view id 'pts'"),
        "unexpected error: {err}"
    );
}
