use avenger_chart::cartesian::Cartesian;
use avenger_chart::legend::LegendPosition;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use datafusion::prelude::{col, lit};

#[test]
fn test_symbol_with_typed_legend() {
    let _plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .fill_with(col("temperature"), |c| {
                c.legend(|l| {
                    l.title("Temperature (°C)")
                        .gradient_length(200.0) // ColorLegendBuilder specific method
                        .gradient_thickness(20.0)
                        .position(LegendPosition::Right)
                })
                .scale(|s| s.option("nice", lit(true)))
            })
            .size_with(col("population"), |c| {
                c.legend(|l| {
                    l.title("Population")
                        .symbol_size(15.0) // SizeLegendBuilder specific method
                        .columns(2)
                })
            })
            .shape_with("type", |c| {
                c.legend(|l| {
                    l.title("Type")
                        .columns(3) // ShapeLegendBuilder specific method
                        .symbol_size(20.0)
                })
            }),
    );
}

#[test]
fn test_legend_builder_methods() {
    use avenger_chart::legend_builder::{ColorLegendBuilder, LegendBuilder, SizeLegendBuilder};

    // Test that ColorLegendBuilder has gradient methods
    let color_legend = ColorLegendBuilder::new()
        .title("Color Legend")
        .gradient_length(150.0)
        .gradient_thickness(10.0)
        .build();

    assert_eq!(color_legend.title, Some("Color Legend".to_string()));
    assert_eq!(color_legend.gradient_length, Some(150.0));
    assert_eq!(color_legend.gradient_thickness, Some(10.0));

    // Test that SizeLegendBuilder has symbol_size method
    let size_legend = SizeLegendBuilder::new()
        .title("Size Legend")
        .symbol_size(25.0)
        .build();

    assert_eq!(size_legend.title, Some("Size Legend".to_string()));
    assert_eq!(size_legend.symbol_size, Some(25.0));
}

#[test]
fn test_channel_config_legend() {
    use avenger_chart::channel_configs::ColorChannelConfig;
    use avenger_chart::marks::ChannelValue;
    use datafusion::prelude::lit;

    // Create a channel config and configure its legend
    let value: ChannelValue = lit("red").into();
    let config = ColorChannelConfig::new(value);

    let configured = config.legend(|l| l.title("Colors").gradient_length(100.0).visible(true));

    // Verify we can get the channel value back
    let _channel_value = configured.into_inner();
}
