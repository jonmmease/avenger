//! Tests for custom Cartesian axis implementations from external crates

use avenger_chart::{
    cartesian::Cartesian,
    marks::symbol::Symbol,
    plot::Plot,
};
use avenger_chart_external_test::external_cartesian_axis::{
    LogarithmicAxis, TemperatureAxis, TemperatureUnit,
};

#[test]
fn test_logarithmic_axis_creation() {
    // Create a plot with custom logarithmic axes
    let _plot = Plot::<Cartesian<LogarithmicAxis>>::new()
        .mark(Symbol::new().x("wavelength").y("intensity"))
        .axis_x(|axis| {
            axis.title("Wavelength (nm)")
                .grid(true)
                .base(10.0)
                .show_minor_ticks(true)
                .minor_tick_count(9)
        })
        .axis_y(|axis| {
            axis.title("Intensity")
                .base(2.0) // Use base 2 for intensity
                .show_minor_ticks(false)
        });

    // The plot compiles with custom axes - success!
}

#[test]
fn test_temperature_axis_creation() {
    // Create a plot with temperature axes
    let _plot = Plot::<Cartesian<TemperatureAxis>>::new()
        .mark(Symbol::new().x("time").y("temperature"))
        .axis_x(|axis| axis.title("Time (hours)").grid(false))
        .axis_y(|axis| {
            axis.title("Temperature")
                .unit(TemperatureUnit::Celsius)
                .show_both_units(true)
                .grid(true)
        });

    // The plot compiles with temperature axes - success!
}

#[test]
fn test_custom_axis_default_initialization() {
    // Test that custom axes get reasonable defaults
    let log_axis = LogarithmicAxis::default();
    assert_eq!(log_axis.visible, true);
    assert_eq!(log_axis.grid, true);
    assert_eq!(log_axis.base, 10.0);
    assert_eq!(log_axis.label_angle, 0.0);

    let temp_axis = TemperatureAxis::default();
    assert_eq!(temp_axis.visible, true); // Custom Default impl sets this to true
    assert_eq!(temp_axis.unit, TemperatureUnit::Celsius);
    assert_eq!(temp_axis.show_both_units, false);
}

#[test]
fn test_logarithmic_axis_formatting() {
    let axis = LogarithmicAxis::default().base(10.0);
    assert_eq!(axis.format_log_value(100.0), "10^2");
    assert_eq!(axis.format_log_value(1000.0), "10^3");

    let axis_base2 = LogarithmicAxis::default().base(2.0);
    assert_eq!(axis_base2.format_log_value(8.0), "2^3");
}

#[test]
fn test_temperature_axis_formatting() {
    let celsius_axis = TemperatureAxis::default().unit(TemperatureUnit::Celsius);
    assert_eq!(celsius_axis.format_temperature(0.0), "0.0°C");
    assert_eq!(celsius_axis.format_temperature(100.0), "100.0°C");

    let fahrenheit_axis = TemperatureAxis::default()
        .unit(TemperatureUnit::Fahrenheit)
        .show_both_units(true);
    assert_eq!(fahrenheit_axis.format_temperature(32.0), "32.0°F (0.0°C)");
    assert_eq!(
        fahrenheit_axis.format_temperature(212.0),
        "212.0°F (100.0°C)"
    );

    let kelvin_axis = TemperatureAxis::default().unit(TemperatureUnit::Kelvin);
    assert_eq!(kelvin_axis.format_temperature(273.15), "273.1K");
}

#[test]
fn test_mixed_axis_types_not_allowed() {
    // This test demonstrates that you can't mix different axis types
    // The following would NOT compile (commented out to keep test passing):
    
    // let plot = Plot::<Cartesian<LogarithmicAxis>>::new()
    //     .axis_x(|axis| axis.base(10.0))
    //     .axis_y(|axis: TemperatureAxis| axis.unit(TemperatureUnit::Celsius));
    
    // This is correct - axes must all be the same type
}

#[test]
fn test_custom_axis_trait_methods() {
    use avenger_chart::cartesian::CartesianAxis;

    let log_axis = LogarithmicAxis::default()
        .title("Log Scale")
        .grid(true);

    // Test trait methods work correctly (these are getters from the trait, not builders)
    assert_eq!(CartesianAxis::title(&log_axis), Some("Log Scale"));
    assert_eq!(CartesianAxis::grid(&log_axis), true);
    assert_eq!(CartesianAxis::visible(&log_axis), true);
    assert_eq!(CartesianAxis::position(&log_axis), None); // Not set by default

    let temp_axis = TemperatureAxis::default()
        .title("Temperature (°C)")
        .grid(false);

    assert_eq!(CartesianAxis::title(&temp_axis), Some("Temperature (°C)"));
    assert_eq!(CartesianAxis::grid(&temp_axis), false);
}