//! Examples of using derive to create child marks

use avenger_chart::derive::{DeriveFn, LabelPoints, TextAlign};
use avenger_chart::adjust::AdjustFn;
use avenger_chart::coords::Cartesian;
use avenger_chart::marks::symbol::Symbol;
use datafusion::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();
    
    // Create sample data
    let df = ctx.sql("
        SELECT 
            'USA' as country,
            'North America' as continent,
            80000.0 as gdp_per_capita,
            78.5 as life_expectancy,
            300.0 as population,
            10.0 as x,
            20.0 as y,
            100.0 as value,
            'Point A' as name,
            1 as id,
            'A' as category,
            50.0 as mean,
            5.0 as stderr
        UNION ALL
        SELECT 'China', 'Asia', 10000.0, 76.0, 1400.0, 30.0, 40.0, 200.0, 'Point B', 2, 'B', 75.0, 7.0
    ").await?;
    
    // Example 1: Using LabelPoints implementation
    let _labeled_scatter = Symbol::<Cartesian>::new()
        .data(df.clone())
        .x("gdp_per_capita")
        .y("life_expectancy") 
        .size("population")
        .derive(LabelPoints::new("country")
            .offset_y(-8.0)  // 8 pixels above
            .align(TextAlign::Center)
            .font_size(10.0)
        );
    
    println!("Created labeled scatter plot");

    // Example 2: Using a lambda with wrapper
    let _points_with_values = Symbol::<Cartesian>::new()
        .data(df.clone())
        .x("category")
        .y("value")
        .derive(DeriveFn::new(|scaled_df, _context| {
            // Create symbol as child mark (in real implementation, would be Text)
            Ok(Box::new(Symbol::<Cartesian>::new().data(scaled_df)) as Box<dyn avenger_chart::marks::Mark<Cartesian>>)
        }));
    
    println!("Created points with value labels");

    // Example 3: Error bars using derive
    let _bars_with_errors = Symbol::<Cartesian>::new()
        .data(df.clone())
        .x("category")
        .y("mean")
        .derive(DeriveFn::new(|scaled_df, _context| {
            // Create error bars from confidence intervals
            let error_df = scaled_df
                .with_column("error_low", col("y") - col("stderr") * lit(1.96))?
                .with_column("error_high", col("y") + col("stderr") * lit(1.96))?;
            
            Ok(Box::new(Symbol::<Cartesian>::new().data(error_df)) as Box<dyn avenger_chart::marks::Mark<Cartesian>>)
        }));
    
    println!("Created bars with error bars");

    // Example 4: Multiple derived marks
    let _complex_points = Symbol::<Cartesian>::new()
        .data(df.clone())
        .x("x")
        .y("y")
        .size("value")
        // Add labels
        .derive(LabelPoints::new("name").offset_y(-10.0))
        // Add confidence ellipses  
        .derive(DeriveFn::new(|scaled_df, _context| {
            // In a real implementation, this would calculate ellipse paths
            Ok(Box::new(Symbol::<Cartesian>::new().data(scaled_df)) as Box<dyn avenger_chart::marks::Mark<Cartesian>>)
        }));
    
    println!("Created complex points with multiple derived marks");

    // Example 5: Conditional derived marks
    let _scatter_with_outlier_labels = Symbol::<Cartesian>::new()
        .data(df.clone())
        .x("x")
        .y("y")
        .derive(DeriveFn::new(|scaled_df, context| {
            // Only label points that are outliers (far from center)
            let center_x = context.width() / 2.0;
            let _center_y = context.height() / 2.0;
            
            // Simplified outlier detection
            let outliers_df = scaled_df
                .filter(col("x").gt(lit(center_x)))?; // Simple filter
            
            Ok(Box::new(Symbol::<Cartesian>::new().data(outliers_df)) as Box<dyn avenger_chart::marks::Mark<Cartesian>>)
        }));
    
    println!("Created scatter plot with outlier labels");

    // Example 6: Voronoi cells from points  
    let _voronoi_scatter = Symbol::<Cartesian>::new()
        .data(df.clone())
        .x("x")
        .y("y")
        .derive(DeriveFn::new(|scaled_df, _context| {
            // In a real implementation, this would compute Voronoi tessellation
            Ok(Box::new(Symbol::<Cartesian>::new().data(scaled_df)) as Box<dyn avenger_chart::marks::Mark<Cartesian>>)
        }));
    
    println!("Created Voronoi scatter plot");

    // Example 7: Derived marks with adjustments
    example_with_smart_labels().await?;
    
    println!("\nAll derive examples completed successfully!");
    Ok(())
}

// Helper to show how derived marks can be adjusted too
async fn example_with_smart_labels() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();
    let df = ctx.sql("SELECT 'Point' as name, 10.0 as x, 20.0 as y").await?;
    
    let _smart_labeled = Symbol::<Cartesian>::new()
        .data(df)
        .x("x")
        .y("y")
        .derive(DeriveFn::new(|scaled_df, _context| {
            // In a real implementation, this would create Text marks
            // For now, we'll create smaller symbols to represent labels
            let labels = Symbol::<Cartesian>::new()
                .data(scaled_df)
                .adjust(AdjustFn::new(|df, _context| {
                    // Smart label placement adjustment would go here
                    Ok(df)
                }));
            Ok(Box::new(labels) as Box<dyn avenger_chart::marks::Mark<Cartesian>>)
        }));
    
    println!("Created smart labeled scatter plot");
    Ok(())
}
