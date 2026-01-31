//! Example showing how external crates can use position channel macros with axis configuration

use avenger_chart::{cartesian::Cartesian, plot::Plot};
use avenger_chart_external_test::external_coord_system::{Cube, Isometric, IsometricAxis};
use avenger_chart_external_test::external_mark::HexBin;
use datafusion::{logical_expr::col, prelude::*};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Example 1: Using HexBin with Cartesian coordinates and axis configuration
    {
        let ctx = SessionContext::new();
        let df = ctx
            .sql("SELECT x, y, count FROM ...")
            .await
            .unwrap_or_else(|_| ctx.read_empty().unwrap());

        let _plot = Plot::<Cartesian>::new().data(df).mark(
            HexBin::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0_f32, 100.0_f32)))
                        .axis(|a| a.title("Hexbin X Axis").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0_f32, 50.0_f32)))
                        .axis(|a| a.title("Hexbin Y Axis"))
                })
                .size(col("count"))
                .fill("#4682b4"),
        );

        println!("Created HexBin plot with custom axis configuration!");
    }

    // Example 2: Using Cube with Isometric coordinates and axis configuration
    {
        let ctx = SessionContext::new();
        let df = ctx
            .sql("SELECT x, y, z, color FROM ...")
            .await
            .unwrap_or_else(|_| ctx.read_empty().unwrap());

        let _plot = Plot::<Isometric>::new().data(df).mark(
            Cube::new()
                .iso_x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0_f32, 10.0_f32)))
                        .axis(|_a| IsometricAxis {
                            channel: "iso_x".to_string(),
                            visible: true,
                        })
                })
                .iso_y_with(col("y"), |c| c.scale(|s| s.domain((0.0_f32, 10.0_f32))))
                .iso_z_with(col("z"), |c| c.scale(|s| s.domain((0.0_f32, 10.0_f32))))
                .fill(col("color"))
                .stroke("#000"),
        );

        println!("Created Isometric Cube plot with custom position configuration!");
    }

    Ok(())
}
