//! Demonstration of the Transform API
//!
//! This example shows how the coordinate-aware transform system works with DataFusion DataFrames.
//!
//! Note: When fully implemented, transforms will automatically set channel mappings.
//! For example, Bin::x("price") will automatically map:
//! - x channel to price_bin_start column
//! - x2 channel to price_bin_end column  
//! - y/y2 channels to aggregation result columns
//!
use avenger_chart::coords::Cartesian;
use avenger_chart::marks::rect::Rect;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use avenger_chart::transforms::{Bin, BinNd, Group, Transform};
use datafusion::arrow::array::{Float64Array, Int32Array, RecordBatch, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::functions_aggregate::expr_fn::{count, sum};
use datafusion::logical_expr::lit;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> datafusion::error::Result<()> {
    // Create sample sales data
    let ctx = SessionContext::new();
    let schema = Arc::new(Schema::new(vec![
        Field::new("product", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("price", DataType::Float64, false),
        Field::new("quantity", DataType::Int32, false),
        Field::new("weight", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "Laptop", "Mouse", "Keyboard", "Monitor", "Laptop", "Mouse", "Keyboard", "Monitor",
                "Tablet", "Phone",
            ])),
            Arc::new(StringArray::from(vec![
                "Electronics",
                "Accessories",
                "Accessories",
                "Electronics",
                "Electronics",
                "Accessories",
                "Accessories",
                "Electronics",
                "Electronics",
                "Electronics",
            ])),
            Arc::new(Float64Array::from(vec![
                1200.0, 25.0, 75.0, 350.0, 1500.0, 30.0, 80.0, 400.0, 800.0, 900.0,
            ])),
            Arc::new(Int32Array::from(vec![2, 5, 3, 1, 1, 8, 4, 2, 3, 4])),
            Arc::new(Float64Array::from(vec![
                2.5, 0.1, 0.3, 5.0, 2.3, 0.1, 0.3, 5.5, 1.0, 0.2,
            ])),
        ],
    )?;

    let df = ctx.read_batch(batch)?;

    println!("=== Coordinate-Aware Transform API Examples ===\n");

    // Example 1: Using transforms on marks (not DataFrames)
    println!("1. Transform API is only available on marks, not DataFrames.");

    // Actually create and transform a mark
    let _grouped_rect = Rect::<Cartesian>::new()
        .data(df.clone())
        .transform(Group::x("category").aggregate(sum(col("quantity"))))?;
    // Transform would automatically sets:
    // .x(col("category"))
    // .y(col("sum_quantity"))

    println!("Created grouped rect mark with transform");
    println!("  - Groups by category");
    println!("  - Y channel shows sum of quantity");
    // Note: Transform implementation is a stub - would produce columns:
    //   category, sum_quantity

    // Example 2: Cartesian Binning
    println!("\n2. Cartesian Binning on marks:");

    let _binned_rect = Rect::<Cartesian>::new().data(df.clone()).transform(
        Bin::x("weight")
            .width(2.0) // Simple method name for single dimension!
            .aggregate(count(col("weight"))),
    )?;
    // Note: The transform would automatically set:
    // .x(col("weight_bin_start"))
    // .x2(col("weight_bin_end"))
    // .y(col("count"))

    println!("Created binned rect mark");
    println!("  - Bins weight field with width 2.0");
    println!("  - Shows count in each bin");
    // Note: Transform implementation is a stub - would produce columns:
    //   weight_bin_start, weight_bin_end, count

    // Example 3: Multiple transforms on the same data
    println!("\n3. Multiple transforms on the same data:");

    // Create a binned histogram by price with extra aggregations
    let _price_histogram = Rect::<Cartesian>::new().data(df.clone()).transform(
        Bin::x("price")
            .bins(5)
            .aggregate(count(lit(1)))
            .extra_aggregate(
                "fill",
                datafusion::functions_aggregate::expr_fn::avg(col("quantity")),
            ),
    )?;
    // Note: The transform would automatically set:
    // .x(col("price_bin_start"))
    // .x2(col("price_bin_end"))
    // .y2(col("count"))
    // .fill(col("avg_quantity"))  // Color by average quantity per bin
    // We only need to explicitly set:
    // .y(lit(0))  // Start bars from 0

    println!("Created price histogram with 5 bins");
    println!("  - X axis: price ranges");
    println!("  - Y axis: count of items in each bin");
    println!("  - Color: average quantity per price bin (via extra_aggregate)");

    // Example 4: Demonstrating channel mappings
    println!("\n4. Transform metadata and channel mappings:");

    // Create a transform and inspect its metadata
    let transform = Bin::<Cartesian>::x("weight")
        .width(1.0)
        .aggregate(sum(col("quantity")));

    let output_channels = transform.output_channels();
    println!("Bin transform output channels:");
    for channel in output_channels {
        println!(
            "  - {}: {} ({})",
            channel.name, channel.description, channel.data_type
        );
    }

    // Example 5: Combining transforms with plot specifications
    println!("\n5. Complete visualization specifications:");

    // Example 6: Show actual DataFrame operations
    println!("\n6. DataFrame operations (what transforms do internally):");

    // Show what a group transform would do
    let category_agg = df
        .clone()
        .aggregate(vec![col("category")], vec![sum(col("quantity"))])?;

    println!("Group by category aggregation:");
    category_agg.show().await?;

    // Example 7: Complete histogram specification
    println!("\n7. Complete histogram plot:");

    // Create a histogram of price distribution
    let _histogram_plot = Plot::new(Cartesian).mark(
        Rect::new()
            .data(df.clone())
            .transform(
                Bin::x("price")
                    .bins(10) // Simple API for single dimension
                    .nice(true)
                    .aggregate(count(lit(1))),
            )?
            .y(lit(0)) // Start bars from 0
            .fill(lit("#4682b4")) // Steel blue
            .stroke(lit("white"))
            .stroke_width(lit(1.0)), // The following would be set automatically by the transform:
                                     // .x(col("price_bin_start"))
                                     // .x2(col("price_bin_end"))
                                     // .y2(col("count"))
    );

    println!("Created histogram plot:");
    println!("  - 800x600 pixels");
    println!("  - 10 bins on price field");
    println!("  - Steel blue bars with white borders");

    // Example 8: Multi-dimensional binning with BinNd
    println!("\n8. Multi-dimensional binning (2D histogram):");

    // Create a 2D histogram
    let _histogram_2d = Rect::<Cartesian>::new()
        .data(df.clone())
        .transform(
            BinNd::xy("price", "weight")
                .width_x(500.0) // Price bins of 500
                .bins_y(5) // 5 weight bins
                .aggregate(count(lit(1))),
        )?
        .fill(col("count")); // Color by count
    // The following would be set automatically by the transform:
    // .x(col("price_bin_start"))
    // .x2(col("price_bin_end"))
    // .y(col("weight_bin_start"))
    // .y2(col("weight_bin_end"))

    println!("Created 2D histogram:");
    println!("  - Price binned by 500 units");
    println!("  - Weight binned into 5 bins");
    println!("  - Color shows count in each 2D bin");

    println!("Created histogram plot:");
    println!("  - 800x600 pixels");
    println!("  - 10 bins on price field");
    println!("  - Steel blue bars with white borders");

    // Example 9: Multi-field grouping with xy
    println!("\n9. Multi-field grouping example:");

    // Create a scatter plot grouped by category and product
    let _scatter_plot = Symbol::<Cartesian>::new()
        .data(df.clone())
        .transform(
            Group::xy("category", "product", "size")
                .aggregate(sum(col("quantity")))
                .extra_aggregate(
                    "fill",
                    datafusion::functions_aggregate::expr_fn::avg(col("price")),
                ),
        )?
        .size(col("sum_quantity")); // Size by total quantity
    // The following would be set automatically by the transform:
    // .x(col("category"))
    // .y(col("product"))
    // .fill(col("avg(price)"))   // DataFusion generates column name from AVG(price)

    println!("Created scatter plot with multi-field grouping:");
    println!("  - Groups by both category (x) and product (y)");
    println!("  - Size shows sum of quantities (aggregation mapped to 'size' channel)");
    println!("  - Color shows average price (extra_aggregate mapped to 'fill' channel)");
    println!("  - DataFusion automatically generates column names from expressions");

    // Example 10: Transform composition
    println!("\n10. Key concepts demonstrated:");
    println!("  - Transforms are applied to marks, not DataFrames");
    println!("  - Single-dimensional Bin has simple API (width, bins, nice)");
    println!("  - Multi-dimensional BinNd uses dimension-specific methods (width_x, bins_y)");
    println!("  - Column naming: field_bin_start, field_bin_end for bins");
    println!("  - Group transforms preserve original column names");
    println!("  - Multi-field grouping with Group::xy() for 2D aggregations");
    println!("  - Transforms store metadata for scale inference");
    println!("  - Coordinate-aware API (Bin::x vs Bin::r, BinNd::xy vs BinNd::rtheta)");

    Ok(())
}
