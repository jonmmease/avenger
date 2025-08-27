use avenger_chart::axis::AxisPosition;
use avenger_chart::cartesian::Cartesian;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;

#[test]
fn test_cartesian_axis_inference_works() {
    // This test verifies that type inference works without explicit type annotations
    // on the axis closure parameter for Cartesian coordinates
    let _plot = Plot::<Cartesian>::new().mark(
        Symbol::<Cartesian>::new()
            .x_with("x", |c| {
                c.axis(|a| {
                    // No type annotation needed on 'a' anymore!
                    // The compiler can infer that 'a' is CartesianAxis
                    a.title("X Axis").grid(true).position(AxisPosition::Bottom)
                })
            })
            .y_with("y", |c| {
                c.axis(|a| {
                    // No type annotation needed on 'a' anymore!
                    // The compiler can infer that 'a' is CartesianAxis
                    a.title("Y Axis").grid(false).position(AxisPosition::Left)
                })
            }),
    );

    // If this compiles, our struct-based axis design works!
}
