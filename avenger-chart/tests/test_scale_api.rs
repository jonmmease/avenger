// Test that the new scale API works correctly
use avenger_chart::prelude::*;
use palette::rgb::Srgba;

#[test]
fn test_typed_scale_with_linear() {
    // Test that we can specify Linear type and get Linear-specific methods
    let _plot =
        Plot::<Cartesian>::new().mark(Rect::new().x(col("category")).y(col("value")).fill_with(
            col("value"),
            |c| {
                c.scale_with::<Linear>(|s| {
                    s.range_colors(vec![
                        Srgba::new(0.97, 0.96, 0.89, 1.0),
                        Srgba::new(0.96, 0.64, 0.38, 1.0),
                        Srgba::new(0.84, 0.19, 0.11, 1.0),
                    ])
                    .zero(true) // Linear-specific method
                    .nice(true) // Linear-specific method
                })
            },
        ));
}

#[test]
fn test_typed_scale_with_band() {
    // Test that we can specify Band type and get Band-specific methods
    let _plot = Plot::<Cartesian>::new().mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale_with::<Band>(|s| {
                    s.domain_discrete(vec![lit("A"), lit("B"), lit("C")])
                        .padding_inner(0.1) // Band-specific method
                        .padding_outer(0.05) // Band-specific method
                        .align(0.5) // Band-specific method
                })
            })
            .y(col("value")),
    );
}

#[test]
fn test_auto_scale_preserves_type() {
    // Test that Auto scale preserves the inferred type
    let _plot =
        Plot::<Cartesian>::new().mark(Rect::new().x(col("category")).y(col("value")).fill_with(
            col("value"),
            |c| {
                c.scale(|s| {
                    // s is Scale<Auto> which uses the inferred type
                    s.range_colors(vec![
                        Srgba::new(0.0, 0.0, 1.0, 1.0),
                        Srgba::new(1.0, 0.0, 0.0, 1.0),
                    ])
                    // Can't use type-specific methods here without knowing the type
                })
            },
        ));
}

#[test]
fn test_scale_preserves_domain_from_data() {
    // Test that the typed scale preserves domain configuration from data
    let _plot =
        Plot::<Cartesian>::new().mark(Rect::new().x(col("category")).y_with(col("value"), |c| {
            c.scale_with::<Linear>(|s| {
                // The scale should already have domain expressions from the mark's y channel
                // We're just adding configuration on top
                s.nice(true).zero(true)
            })
        }));
}
