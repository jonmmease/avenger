use avenger_chart::coords::Cartesian;
use avenger_chart::marks::rect::Rect;
use avenger_chart::marks::ChannelValue;
use avenger_chart::plot::Plot;
use datafusion::prelude::*;

#[tokio::test]
async fn test_data_domain_inference() -> Result<(), Box<dyn std::error::Error>> {
    // Create sample data
    let ctx = SessionContext::new();
    let df = ctx.sql(
        "SELECT * FROM (VALUES 
            ('A', 20.0),
            ('B', 35.0),
            ('C', 30.0)
        ) AS t(category, value)"
    ).await?;

    // Create a plot without explicit domains
    let mut plot = Plot::new(Cartesian)
        .preferred_size(200.0, 150.0)
        .data(df)
        .scale_x(|s| s)  // Should infer discrete domain from category column
        .scale_y(|s| s)  // Should infer numeric domain from value column
        .mark(
            Rect::new()
                .x("category")
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0))
                .y2("value")
                .fill(lit("#4682b4"))
        );

    // Apply default domain to x scale
    plot.apply_default_domain("x");
    
    // Check that the x scale now has a domain
    let x_scale = plot.get_scale("x").unwrap();
    assert!(x_scale.has_explicit_domain());
    
    // Apply default domain to y scale
    plot.apply_default_domain("y");
    
    // Check that the y scale now has a domain
    let y_scale = plot.get_scale("y").unwrap();
    assert!(y_scale.has_explicit_domain());
    
    Ok(())
}

#[tokio::test]
async fn test_numeric_domain_inference() -> Result<(), Box<dyn std::error::Error>> {
    // Create scatter plot data
    let ctx = SessionContext::new();
    let df = ctx.sql(
        "SELECT * FROM (VALUES 
            (10.0, 20.0),
            (20.0, 35.0),
            (30.0, 15.0),
            (40.0, 45.0)
        ) AS t(x, y)"
    ).await?;

    // Create a plot without explicit domains
    let mut plot = Plot::new(Cartesian)
        .preferred_size(200.0, 150.0)
        .data(df)
        .scale_x(|s| s)  // Should compute min/max from x column
        .scale_y(|s| s)  // Should compute min/max from y column
        .mark(
            Rect::new()
                .x("x")
                .x2(col("x").add(lit(2.0)))
                .y("y")  
                .y2(col("y").add(lit(2.0)))
                .fill(lit("#ff0000"))
        );

    // Apply default domains
    plot.apply_default_domain("x");
    plot.apply_default_domain("y");
    
    // Check that both scales now have domains
    assert!(plot.get_scale("x").unwrap().has_explicit_domain());
    assert!(plot.get_scale("y").unwrap().has_explicit_domain());
    
    Ok(())
}