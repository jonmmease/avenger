use super::datasets;
use super::helpers::assert_visual_match_default;
use avenger_chart::param::Param;
use avenger_chart::prelude::*;
use datafusion::scalar::ScalarValue;
use indexmap::IndexMap;

#[tokio::test]
async fn test_param_fill_color() {
    let ctx = datafusion::prelude::SessionContext::new();
    let df = datasets::simple_categories();

    // Create a parameter for fill color with blue as default
    let fill_color_param = {
        let __avenger_param_name = "fill_color";
        let __avenger_param_default: datafusion::common::ScalarValue =
            (ScalarValue::Utf8(Some("#4682b4".to_string()))).into();
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };

    let plot = Chart::<Cartesian>::new()
        .data(df)
        .param(fill_color_param.clone())
        .mark(
            Rect::new()
                .x_with(col("category"), |c| {
                    c.scale_with::<Band>(|s| {
                        s.domain_discrete(vec![lit("A"), lit("B"), lit("C"), lit("D"), lit("E")])
                    })
                    .axis(|a| a.title("Category").grid(false))
                })
                .x2_with(col(":x"), |c| c.band(0.8))
                .y_with(lit(0.0), |c| {
                    c.scale(|s| s.domain((0.0, 100.0)))
                        .axis(|a| a.title("Value").grid(true))
                })
                .y2(col("value"))
                // Use the parameter for fill color - no scale needed for color strings
                .fill_with(fill_color_param.expr(), |c| c.no_scale())
                .stroke("#333333")
                .stroke_width(1.0),
        );

    // Compile once
    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");

    assert_visual_match_default(&compiled, &ctx, None, "param", "fill_color_default").await;

    // Render with override params (red)
    let mut override_params = IndexMap::new();
    override_params.insert(
        "fill_color".to_string(),
        ScalarValue::Utf8(Some("#ff6b6b".to_string())),
    );
    assert_visual_match_default(
        &compiled,
        &ctx,
        Some(override_params),
        "param",
        "fill_color_override",
    )
    .await;
}
