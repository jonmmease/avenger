//! Test for CompiledPlot serialization

use avenger_chart::cartesian::{CartesianRectPositionChannels, CartesianSymbolPositionChannels};
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::{CompiledPlot, Plot};
use avenger_chart::prelude::{
    Cartesian, CoordinationScope, Linear, NestScope, Parallel, ParallelAxisOverlay, ParallelLine,
    ParallelSymbol, Rect, ScaleChannelConfig,
};
use datafusion::prelude::{SessionContext, col, lit};

#[tokio::test]
async fn test_compiled_plot() {
    // Create a session context
    let ctx = SessionContext::new();

    // Create a simple plot
    let plot = Plot::new()
        .canvas_size(400.0, 300.0)
        .mark(Symbol::new().x(col("x")).y(col("y")));

    // Compile the plot to get the renderer
    let renderer = plot.compile(&ctx).await.unwrap();

    // Serialize to JSON
    let json = serde_json::to_string_pretty(&renderer).unwrap();

    // Deserialize back
    let _deserialized: CompiledPlot = serde_json::from_str(&json).unwrap();

    // Just verify it round-trips successfully
    assert!(json.contains("coord_transform"));
    assert!(json.contains("marks"));
}

#[tokio::test]
async fn test_compiled_plot_with_nested_position_metadata() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().mark(
        Rect::new()
            .x_with(avenger_chart::prelude::nested(["quarter", "team"]), |x| {
                x.level(0, |l| l.domain_scope(CoordinationScope::Shared))
                    .level(1, |l| {
                        l.domain_scope(CoordinationScope::Shared)
                            .nest_scope(NestScope::Shared)
                            .label_with(col("team_label"))
                            .axis(|a| a.visible(false))
                    })
            })
            .x2_with(col(":x"), |x| x.band(1.0))
            .y(lit(0.0))
            .y2(col("value")),
    );

    let compiled = plot.compile(&ctx).await.unwrap();
    let json = serde_json::to_string_pretty(&compiled).unwrap();
    let _deserialized: CompiledPlot = serde_json::from_str(&json).unwrap();

    assert!(json.contains("nested_band_config"));
    assert!(json.contains("source_columns"));
    assert!(json.contains("nest_scope"));
    assert!(json.contains("domain_coordination"));
    assert!(json.contains("label_expr"));
}

#[tokio::test]
async fn test_compiled_parallel_plot() {
    let ctx = SessionContext::new();
    let df = ctx
        .sql("SELECT * FROM (VALUES (21.0, 'usa'), (28.0, 'japan')) AS t(mpg, origin)")
        .await
        .unwrap();
    let plot = Plot::with_coord(Parallel::new().dimension("mpg", col("mpg")).dimension_with(
        "origin",
        col("origin"),
        |dimension| dimension.axis(|axis| axis.title("Origin")),
    ))
    .data(df)
    .mark(
        ParallelAxisOverlay::new(
            "mpg",
            Plot::<Cartesian>::new().mark(
                Rect::new()
                    .x_with(lit(0.0), |x| {
                        x.scale_with::<Linear>(|scale| scale.domain((0.0, 1.0)))
                            .axis(|axis| axis.visible(false))
                    })
                    .x2(lit(1.0))
                    .y_with(lit(20.0), |y| y.axis(|axis| axis.visible(false)))
                    .y2(lit(30.0))
                    .fill("rgba(42, 115, 219, 0.18)"),
            ),
        )
        .width_px(24.0),
    )
    .mark(ParallelLine::new().stroke(col("origin")).opacity(0.6))
    .mark(ParallelSymbol::new().fill(col("origin")).size(64.0));

    let compiled = plot.compile(&ctx).await.unwrap();
    let json = serde_json::to_string_pretty(&compiled).unwrap();
    let _deserialized: CompiledPlot = serde_json::from_str(&json).unwrap();

    assert!(json.contains("ParallelTransform"));
    assert!(json.contains("CompiledParallelAxisOverlay"));
    assert!(json.contains("CompiledParallelLine"));
    assert!(json.contains("CompiledParallelSymbol"));
    assert!(json.contains("compiled_guide"));
    assert!(json.contains("coordinate_scale_sources"));
    assert!(json.contains("__avenger_parallel_dim_mpg"));
}
