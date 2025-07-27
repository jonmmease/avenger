use avenger_chart::coords::Cartesian;
use avenger_chart::marks::ChannelValue;
use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use avenger_chart::scales::Scale;
use datafusion::arrow::array::{Array, AsArray};
use datafusion::arrow::datatypes;
use datafusion::prelude::*;

#[tokio::test]
async fn test_data_domain_inference() -> Result<(), Box<dyn std::error::Error>> {
    // Create sample data
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES 
            ('A', 20.0),
            ('B', 35.0),
            ('C', 30.0)
        ) AS t(category, value)",
        )
        .await?;

    // Create a plot without explicit domains
    let mut plot = Plot::new(Cartesian)
        .preferred_size(200.0, 150.0)
        .data(df)
        .scale_x(|_s| {
            // Use band scale for categorical data
            Scale::new(avenger_scales::scales::band::BandScale)
                .option("padding_inner", lit(0.1))
                .option("padding_outer", lit(0.1))
                .option("align", lit(0.5))
        })
        .scale_y(|s| s) // Should infer numeric domain from value column
        .mark(
            Rect::new()
                .x("category")
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0))
                .y2("value")
                .fill(lit("#4682b4")),
        );

    // Apply default domain to x scale
    plot.apply_default_domain("x");

    // Check that the x scale now has a domain
    let x_scale = plot.get_scale("x").unwrap();
    assert!(x_scale.has_explicit_domain());

    // Debug: print what kind of domain we have
    println!("X scale domain: {:?}", x_scale.get_domain());

    // Evaluate the x scale domain to verify we get unique categories
    let x_domain = eval_scale_domain(&ctx, &x_scale).await?;
    let x_domain_array = x_domain.as_list::<i32>().value(0);
    let x_domain_strings = x_domain_array.as_string::<i32>();

    // Should have 3 unique categories in some order
    let len = x_domain_strings.value_length(0)
        + x_domain_strings.value_length(1)
        + x_domain_strings.value_length(2);
    assert!(len > 0); // Verify we got data
    let mut categories: Vec<_> = (0..x_domain_strings.len())
        .map(|i| x_domain_strings.value(i))
        .collect();
    categories.sort();
    assert_eq!(categories, vec!["A", "B", "C"]);

    // Apply default domain to y scale
    plot.apply_default_domain("y");

    // Check that the y scale now has a domain
    let y_scale = plot.get_scale("y").unwrap();
    assert!(y_scale.has_explicit_domain());

    // Evaluate the y scale domain to verify we get min/max
    let y_domain = eval_scale_domain(&ctx, &y_scale).await?;
    let y_domain_array = y_domain.as_list::<i32>().value(0);
    let y_domain_floats = y_domain_array.as_primitive::<datatypes::Float32Type>();

    // Should have [min, max] = [0.0, 35.0] because y encoding includes lit(0.0)
    assert_eq!(y_domain_floats.len(), 2);
    assert_eq!(y_domain_floats.value(0), 0.0);
    assert_eq!(y_domain_floats.value(1), 35.0);

    Ok(())
}

// Helper function to evaluate a scale's domain expression
async fn eval_scale_domain(
    ctx: &SessionContext,
    scale: &Scale,
) -> Result<datafusion::arrow::array::ArrayRef, Box<dyn std::error::Error>> {
    let domain_expr = scale
        .get_domain()
        .compile(scale.get_scale_impl().infer_domain_from_data_method())?;

    // Create a unit dataframe to evaluate the expression
    let unit_df = ctx.read_empty()?;
    let result_df = unit_df.select(vec![domain_expr.alias("domain")])?;
    let batches = result_df.collect().await?;

    Ok(batches[0].column_by_name("domain").unwrap().clone())
}

#[tokio::test]
async fn test_numeric_domain_inference() -> Result<(), Box<dyn std::error::Error>> {
    // Create scatter plot data
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES 
            (10.0, 20.0),
            (20.0, 35.0),
            (30.0, 15.0),
            (40.0, 45.0)
        ) AS t(x, y)",
        )
        .await?;

    // Create a plot without explicit domains
    let mut plot = Plot::new(Cartesian)
        .preferred_size(200.0, 150.0)
        .data(df)
        .scale_x(|s| s) // Should compute min/max from x column
        .scale_y(|s| s) // Should compute min/max from y column
        .mark(
            Rect::new()
                .x("x")
                .x2(col("x").add(lit(2.0)))
                .y("y")
                .y2(col("y").add(lit(2.0)))
                .fill(lit("#ff0000")),
        );

    // Apply default domains
    plot.apply_default_domain("x");
    plot.apply_default_domain("y");

    // Check that both scales now have domains
    let x_scale = plot.get_scale("x").unwrap();
    let y_scale = plot.get_scale("y").unwrap();
    assert!(x_scale.has_explicit_domain());
    assert!(y_scale.has_explicit_domain());

    // Evaluate the x scale domain to verify we get min/max
    let x_domain = eval_scale_domain(&ctx, &x_scale).await?;
    let x_domain_array = x_domain.as_list::<i32>().value(0);
    let x_domain_floats = x_domain_array.as_primitive::<datatypes::Float32Type>();

    // Should have [min, max] = [10.0, 42.0] because x2 encoding adds 2.0
    assert_eq!(x_domain_floats.len(), 2);
    assert_eq!(x_domain_floats.value(0), 10.0);
    assert_eq!(x_domain_floats.value(1), 42.0);

    // Evaluate the y scale domain to verify we get min/max
    let y_domain = eval_scale_domain(&ctx, &y_scale).await?;
    let y_domain_array = y_domain.as_list::<i32>().value(0);
    let y_domain_floats = y_domain_array.as_primitive::<datatypes::Float32Type>();

    // Should have [min, max] = [15.0, 47.0] because y2 encoding adds 2.0
    assert_eq!(y_domain_floats.len(), 2);
    assert_eq!(y_domain_floats.value(0), 15.0);
    assert_eq!(y_domain_floats.value(1), 47.0);

    Ok(())
}

#[tokio::test]
async fn test_scale_options_preserved_during_domain_inference()
-> Result<(), Box<dyn std::error::Error>> {
    // Create sample data
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES 
            ('A', 20.0),
            ('B', 35.0),
            ('C', 30.0)
        ) AS t(category, value)",
        )
        .await?;

    // Create a plot with scale options set
    let mut plot = Plot::new(Cartesian)
        .preferred_size(200.0, 150.0)
        .data(df)
        .scale_x(|_s| {
            Scale::new(avenger_scales::scales::band::BandScale)
                .option("padding_inner", lit(0.2))
                .option("padding_outer", lit(0.3))
        })
        .scale_y(|s| s.option("zero", lit(true)).option("nice", lit(true)))
        .mark(
            Rect::new()
                .x("category")
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0))
                .y2("value")
                .fill(lit("#4682b4")),
        );

    // Apply default domains
    plot.apply_default_domain("x");
    plot.apply_default_domain("y");

    // Check that options are preserved
    let x_scale = plot.get_scale("x").unwrap();
    let x_options = x_scale.get_options();
    assert_eq!(x_options.get("padding_inner").unwrap(), &lit(0.2));
    assert_eq!(x_options.get("padding_outer").unwrap(), &lit(0.3));

    let y_scale = plot.get_scale("y").unwrap();
    let y_options = y_scale.get_options();
    assert_eq!(y_options.get("zero").unwrap(), &lit(true));
    assert_eq!(y_options.get("nice").unwrap(), &lit(true));

    // Also verify that domains were actually applied
    assert!(x_scale.has_explicit_domain());
    assert!(y_scale.has_explicit_domain());

    Ok(())
}
