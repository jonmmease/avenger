use avenger_chart::prelude::*;

#[test]
fn test_axis_config_from_channel() {
    // Create a simple plot with axis configuration via channel
    let _symbol = Symbol::<Cartesian>::new()
        .x_with(col("x"), |c| {
            c.scale(|s| s.domain((0.0, 100.0)))
                .axis(|a| a.title("X Axis from Channel"))
        })
        .y_with(col("y"), |c| {
            c.scale(|s| s.domain((0.0, 50.0)))
                .axis(|a| a.title("Y Axis from Channel").grid(true))
        });

    // Test that it compiles - that's the main verification for now
    // In a full test, we would create a plot and render it
}

#[test]
fn test_axis_config_with_no_scale() {
    // Test that axis config works with no_scale channels
    let _symbol = Symbol::<Cartesian>::new()
        .x_with(10.0, |c| c.no_scale().axis(|a| a.title("Fixed X")))
        .y_with(col("y"), |c| c.axis(|a| a.title("Y Column")));
}

#[test]
fn test_axis_config_precedence() {
    // Test that channel-level axis config takes precedence over plot-level
    // Since plot-level axis config is separate from channel-level,
    // we just verify that both syntaxes compile correctly
    let _symbol = Symbol::<Cartesian>::new().x_with(col("x"), |c| {
        c.scale(|s| s.domain((0.0, 100.0)))
            .axis(|a| a.title("Channel Level X"))
    });

    // In practice, "Channel Level X" should be used since marks are processed after plot axes
}
