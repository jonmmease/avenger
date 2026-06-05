use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::{
    array::{Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use std::sync::Arc;

fn kde_density_data(ctx: &SessionContext) -> DataFrame {
    let values = vec![
        -2.65, -2.35, -2.1, -1.95, -1.7, -1.55, -1.35, -1.05, -0.8, -0.55, 0.72, 0.9, 1.05, 1.18,
        1.34, 1.48, 1.62, 1.78, 1.96, 2.14, 2.35, 2.62, 2.94, 3.22,
    ];
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Float64,
            false,
        )])),
        vec![Arc::new(Float64Array::from(values)) as _],
    )
    .expect("kde density batch");
    ctx.read_batch(batch).expect("kde density dataframe")
}

fn grouped_density_data(ctx: &SessionContext) -> DataFrame {
    let series = vec![
        "North", "North", "North", "North", "North", "North", "North", "North", "North", "North",
        "North", "North", "South", "South", "South", "South", "South", "South", "South", "South",
        "South", "South", "South", "South",
    ];
    let values = vec![
        -2.4, -2.15, -1.9, -1.72, -1.5, -1.25, -0.92, -0.65, -0.28, 0.1, 0.35, 0.62, 0.2, 0.54,
        0.84, 1.08, 1.28, 1.48, 1.68, 1.95, 2.24, 2.55, 2.88, 3.22,
    ];
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("series", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(series)) as _,
            Arc::new(Float64Array::from(values)) as _,
        ],
    )
    .expect("grouped kde batch");
    ctx.read_batch(batch).expect("grouped kde dataframe")
}

#[tokio::test]
async fn kde_density_area() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("KDE density")
        .subtitle("Bimodal area density from an eager transform")
        .canvas_size(680.0, 420.0)
        .data(kde_density_data(&ctx))
        .mark(
            Area::new().transform(
                Kde::new(col("value"))
                    .bandwidth(0.38)
                    .steps(160)
                    .extent(-3.2, 3.8)
                    .as_fields("sample", "density"),
                |mark, kde| {
                    mark.x_with(kde.value(), |c| c.axis(|a| a.title("Sample")))
                        .y_with(kde.density(), |c| c.axis(|a| a.title("Density")))
                        .y2_with(lit(0.0), |c| c.with_scale_name("y"))
                        .fill("#2f80ed")
                        .stroke("#174ea6")
                        .stroke_width(1.5)
                        .opacity(0.58)
                        .order(ChannelValue::from(kde.value()).no_scale())
                },
            ),
        );

    let compiled = plot.compile(&ctx).await.expect("compile kde area");
    assert_visual_match_default(&compiled, &ctx, None, "transform_kde", "kde_density_area").await;
}

#[tokio::test]
async fn kde_grouped_density_lines() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Grouped KDE density")
        .subtitle("Shared sample grid with a categorical color legend")
        .canvas_size(720.0, 440.0)
        .data(grouped_density_data(&ctx))
        .mark(
            Line::new().transform(
                Kde::new(col("value"))
                    .group_by([col("series")])
                    .bandwidth(0.42)
                    .steps(150)
                    .resolve(KdeResolve::Shared)
                    .as_fields("sample", "density"),
                |mark, kde| {
                    mark.x_with(kde.value(), |c| c.axis(|a| a.title("Sample")))
                        .y_with(kde.density(), |c| c.axis(|a| a.title("Density")))
                        .stroke_with(col("series"), |c| c.legend(|l| l.title("Series")))
                        .stroke_width(3.0)
                        .opacity(0.86)
                        .order(ChannelValue::from(kde.value()).no_scale())
                },
            ),
        );

    let compiled = plot.compile(&ctx).await.expect("compile grouped kde");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_kde",
        "kde_grouped_density_lines",
    )
    .await;
}
