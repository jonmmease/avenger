//! Transform system for data visualization
//!
//! This module provides a Transform trait and associated types that work
//! similar to Observable Plot's transform system.

pub mod bin;
pub mod core;
pub mod group;
pub mod stack;

// Re-export commonly used types
pub use bin::{Bin, BinNd};
pub use core::{ChannelInfo, DataContext, Transform, TransformInput};
pub use group::Group;
pub use stack::{Stack, StackOffset, StackOrder};

// Example usage:
// ```rust
// use datafusion::prelude::*;
// use datafusion::functions_aggregate::expr_fn::{sum, count, avg};
// use avenger_chart::transforms::{Bin, BinNd, Group};
// use avenger_chart::coords::{Cartesian, Polar};
// use avenger_chart::marks::{Rect, Arc, Symbol};
//
// // Single-dimensional binning with simple API
// Rect::<Cartesian>::new()
//     .data(df.clone())
//     .transform(
//         Bin::<Cartesian>::x("weight")
//             .width(10.0)           // Simple method name!
//             .nice(true)
//             .aggregate(count(lit(1)))
//             .extra_aggregate("fill", avg(col("value")))  // Color by average value
//     )?
//     .x(col("weight_bin_start"))    // Column: {field}_bin_start
//     .x2(col("weight_bin_end"))     // Column: {field}_bin_end
//     .y(col("count"))               // Aggregation column preserves name
//     // The following would be set automatically by the transform:
//     // .fill(col("avg_value"))    // Automatically mapped from AVG(value)
//
// // Multi-dimensional binning (2D histogram)
// Rect::<Cartesian>::new()
//     .data(df.clone())
//     .transform(
//         BinNd::<Cartesian>::xy("price", "weight")
//             .width_x(100.0)      // Price bins of 100
//             .bins_y(20)          // 20 weight bins
//             .nice_x(true)        // Nice price boundaries
//             .aggregate(count(lit(1)))
//             .extra_aggregate("opacity", avg(col("quality")))  // Opacity by quality
//     )?
//     .x(col("price_bin_start"))
//     .x2(col("price_bin_end"))
//     .y(col("weight_bin_start"))
//     .y2(col("weight_bin_end"))
//     .fill(col("count"))         // Color by count
//     // The following would be set automatically by the transform:
//     // .opacity(col("avg_quality"))  // Automatically mapped from AVG(quality)
//
// // Alternative: using index-based configuration
// Rect::<Cartesian>::new()
//     .data(df.clone())
//     .transform(
//         BinNd::<Cartesian>::xy("price", "weight")
//             .width_for(0, 100.0)    // First field (price)
//             .bins_for(1, 20)        // Second field (weight)
//             .aggregate(count(lit(1)))
//     )?
//
// // Bar chart with single field grouping
// Rect::<Cartesian>::new()
//     .data(df.clone())
//     .transform(
//         Group::<Cartesian>::x("category")
//             .aggregate(sum(col("quantity")))
//     )?
//     .x(col("category"))           // Original column name preserved
//     .y(col("sum_quantity"))       // Automatically mapped from aggregate
//
// // Grouped bar chart preparation (for future stacking)
// Rect::<Cartesian>::new()
//     .data(df.clone())
//     .transform(
//         Group::<Cartesian>::xfill("month", "product")
//             .aggregate(sum(col("sales")))  // Goes to y channel
//     )?
//     .x(col("month"))
//     .y(col("sum_sales"))          // Aggregated sales
//     .fill(col("product"))          // Color by product
//
// // Scatter plot with multi-field grouping
// Symbol::<Cartesian>::new()
//     .data(df.clone())
//     .transform(
//         Group::<Cartesian>::xy("category", "region", "size")
//             .aggregate(count(lit(1)))
//             .extra_aggregate("fill", avg(col("price")))
//     )?
//     .x(col("category"))           // Grouped by category
//     .y(col("region"))             // Grouped by region
//     .size(col("count"))           // Count mapped to size channel
//     // The following would be set automatically by the transform:
//     // .fill(col("avg_price"))    // Automatically mapped from AVG(price)
//
// // Polar binning with simple API
// Arc::<Polar>::new()
//     .data(df.clone())
//     .transform(
//         Bin::<Polar>::r("magnitude")
//             .width(0.5)           // Simple method name!
//             .aggregate(count(lit(1)))
//     )?
//     .r(col("magnitude_bin_start"))
//     .r2(col("magnitude_bin_end"))
//     .theta(col("count"))
//
// // Multi-dimensional polar binning
// Arc::<Polar>::new()
//     .data(df.clone())
//     .transform(
//         BinNd::<Polar>::rtheta("magnitude", "angle")
//             .width_r(0.5)
//             .bins_theta(12)      // 12 angular bins (30 degrees each)
//             .aggregate(count(lit(1)))
//     )?
//     .r(col("magnitude_bin_start"))
//     .r2(col("magnitude_bin_end"))
//     .theta(col("angle_bin_start"))
//     .theta2(col("angle_bin_end"))
//     .fill(col("count"))
// ```
