use avenger_chart::prelude::*;
use datafusion::prelude::*;
use indexmap::IndexMap;

async fn facet_data(
    ctx: &SessionContext,
) -> Result<datafusion::dataframe::DataFrame, datafusion::error::DataFusionError> {
    ctx.sql(
        "SELECT 'east' AS region, 'A' AS species, 1.0 AS x, 2.0 AS y
         UNION ALL SELECT 'west', 'A', 2.0, 3.0
         UNION ALL SELECT 'east', 'B', 3.0, 4.0
         UNION ALL SELECT 'west', 'B', 4.0, 5.0",
    )
    .await
}

fn leaf_plot() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(Symbol::new().x(col("x")).y(col("y")))
}

#[tokio::test]
async fn facet_dimensions_do_not_build_row_column_or_wrap_scales()
-> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();
    let df = facet_data(&ctx).await?;

    let column_plot = Plot::<FacetColumn>::new()
        .data(df.clone())
        .mark(Subplot::new(leaf_plot()).column(col("region")));
    let column_compiled = column_plot.compile(&ctx).await?;
    let column_scales = column_compiled
        .build_scales_for_dataframe(&df, 400.0, 300.0, &ctx, &IndexMap::new())
        .await?;
    assert!(!column_scales.contains_key("column"));

    let row_plot = Plot::<FacetRow>::new()
        .data(df.clone())
        .mark(Subplot::new(leaf_plot()).row(col("species")));
    let row_compiled = row_plot.compile(&ctx).await?;
    let row_scales = row_compiled
        .build_scales_for_dataframe(&df, 400.0, 300.0, &ctx, &IndexMap::new())
        .await?;
    assert!(!row_scales.contains_key("row"));

    let wrap_plot = Plot::<FacetWrap>::new()
        .data(df.clone())
        .mark(Subplot::new(leaf_plot()).wrap(col("region")));
    let wrap_compiled = wrap_plot.compile(&ctx).await?;
    let wrap_scales = wrap_compiled
        .build_scales_for_dataframe(&df, 400.0, 300.0, &ctx, &IndexMap::new())
        .await?;
    assert!(!wrap_scales.contains_key("wrap"));

    Ok(())
}

#[tokio::test]
async fn facet_dimension_scale_config_is_rejected() {
    let ctx = SessionContext::new();
    let df = facet_data(&ctx).await.expect("facet data");
    let scaled_column =
        ChannelValue::from(col("region")).scale_with::<Band>(|scale| scale.padding_inner(0.2));

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .mark(Subplot::new(leaf_plot()).column(scaled_column));

    let err = match plot.compile(&ctx).await {
        Ok(_) => panic!("facet dimension scale config should fail"),
        Err(err) => err,
    };
    let message = err.to_string();
    assert!(message.contains("Coordinate channel 'column'"), "{message}");
    assert!(
        message.contains("does not support scale or legend configuration"),
        "{message}"
    );
}

#[tokio::test]
async fn facet_wrap_dimension_scale_config_is_rejected() {
    let ctx = SessionContext::new();
    let df = facet_data(&ctx).await.expect("facet data");
    let scaled_wrap =
        ChannelValue::from(col("region")).scale_with::<Band>(|scale| scale.padding_inner(0.2));

    let plot = Plot::<FacetWrap>::new()
        .data(df)
        .mark(Subplot::new(leaf_plot()).wrap(scaled_wrap));

    let err = match plot.compile(&ctx).await {
        Ok(_) => panic!("facet wrap dimension scale config should fail"),
        Err(err) => err,
    };
    let message = err.to_string();
    assert!(message.contains("Coordinate channel 'wrap'"), "{message}");
    assert!(
        message.contains("does not support scale or legend configuration"),
        "{message}"
    );
}

#[tokio::test]
async fn facet_dimension_legend_config_is_rejected() {
    let ctx = SessionContext::new();
    let df = facet_data(&ctx).await.expect("facet data");
    let legend_column = ChannelValue::from(col("region")).legend(Legend::new().title("Region"));

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .mark(Subplot::new(leaf_plot()).column(legend_column));

    let err = match plot.compile(&ctx).await {
        Ok(_) => panic!("facet dimension legend config should fail"),
        Err(err) => err,
    };
    let message = err.to_string();
    assert!(message.contains("Coordinate channel 'column'"), "{message}");
    assert!(
        message.contains("does not support scale or legend configuration"),
        "{message}"
    );
}
