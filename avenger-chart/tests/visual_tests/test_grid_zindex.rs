use super::helpers::assert_visual_match_default;
use avenger_chart::cartesian::Cartesian;

use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use avenger_chart::scales::Linear;
use datafusion::prelude::*;
// Visual tests for grid line z-index behavior

#[tokio::test]
async fn test_grid_lines_behind_data() {
    // Create a plot with grid lines and a semi-transparent rectangle
    // The grid lines should be visible through the rectangle if they're correctly behind it

    // Create a simple dataframe with one row (rect mark doesn't need data but Plot requires it)
    let ctx = SessionContext::new();
    let df = ctx.read_empty().expect("Failed to create empty DataFrame");

    let plot = Plot::<Cartesian>::new()
        .data(df)
        // Add a semi-transparent rectangle that covers part of the grid
        .mark(
            Rect::new()
                .x_with(lit(2.0), |c| {
                    c.scale_with::<Linear>(|s| s.domain((0.0, 10.0)))
                        .axis(|a| a.title("X").grid(true))
                })
                .x2(lit(8.0))
                .y_with(lit(3.0), |c| {
                    c.scale_with::<Linear>(|s| s.domain((0.0, 10.0)))
                        .axis(|a| a.title("Y").grid(true))
                })
                .y2(lit(7.0))
                .fill("#ff0000")
                .opacity(0.5), // Semi-transparent so we can see grid lines through it
        );

    assert_visual_match_default(plot, "grid_zindex", "grid_lines_behind_data").await;
}
