use avenger_chart::prelude::*;
use datafusion::common::ScalarValue;
use datafusion::prelude::*;
use indexmap::IndexMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();

    // Simple test data
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES
        ('2020-01-01'::DATE, 100.0, 'AAPL'),
        ('2020-02-01'::DATE, 105.0, 'AAPL'),
        ('2020-01-01'::DATE, 200.0, 'GOOG'),
        ('2020-02-01'::DATE, 210.0, 'GOOG')
    ) AS t(date, price, symbol)",
        )
        .await?;

    // CSS with height-based media query
    let css = r#"
        legend {
            position: right;
        }

        @media (height >= 300px) {
            legend {
                position: bottom;
            }
        }
    "#;

    let theme = Theme::from_css(css)?;

    let width = {
        let __avenger_param_name = "width";
        let __avenger_param_default: datafusion::common::ScalarValue =
            (ScalarValue::from(600.0)).into();
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };
    let height = {
        let __avenger_param_name = "height";
        let __avenger_param_default: datafusion::common::ScalarValue =
            (ScalarValue::from(300.0)).into();
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };

    let plot = Chart::<Cartesian>::new()
        .theme(theme)
        .data(df)
        .param(width.clone())
        .param(height.clone())
        .canvas_size(&width, &height)
        .mark(
            Line::new()
                .x(col("date"))
                .y(col("price"))
                .stroke_with(col("symbol"), |c| {
                    c.scale_with::<Ordinal>(|s| s).legend(|l| l.title("Stock"))
                }),
        );

    let compiled = plot.compile(&ctx).await?;

    println!("\n=== Testing with height=400 (should be bottom) ===");
    let mut params = IndexMap::new();
    params.insert("width".to_string(), ScalarValue::from(600.0));
    params.insert("height".to_string(), ScalarValue::from(400.0));
    let result_bottom = compiled.evaluate(&ctx, Some(params)).await?;

    println!("\n=== Testing with height=200 (should be right) ===");
    let mut params = IndexMap::new();
    params.insert("width".to_string(), ScalarValue::from(600.0));
    params.insert("height".to_string(), ScalarValue::from(200.0));
    let result_right = compiled.evaluate(&ctx, Some(params)).await?;

    // Save to files for inspection
    use avenger_chart::doc::render::render_evaluated_plot_to_png;
    if let Err(e) =
        render_evaluated_plot_to_png(&result_bottom, "/tmp/test_legend_bottom.png").await
    {
        eprintln!("Failed to render bottom legend: {:?}", e);
    }
    if let Err(e) = render_evaluated_plot_to_png(&result_right, "/tmp/test_legend_right.png").await
    {
        eprintln!("Failed to render right legend: {:?}", e);
    }

    println!("\nSaved outputs to:");
    println!("  /tmp/test_legend_bottom.png (height=400, should show legend at bottom)");
    println!("  /tmp/test_legend_right.png (height=200, should show legend on right)");

    Ok(())
}
