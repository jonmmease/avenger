use arrow::array::{ArrayRef, Float32Array, Float64Array};
use avenger_scales::scales::linear::LinearScale;
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Tick Generation and Scale Options ===\n");

    // Example 1: Tick Generation
    println!("1. Tick Generation:");

    // Linear scale ticks
    let tick_scale = LinearScale::configured((0.0, 100.0), (0.0, 400.0));

    println!("Linear scale ticks (different counts):");
    for tick_count in [5.0, 10.0, 15.0] {
        let ticks = tick_scale.ticks(Some(tick_count))?;
        let tick_array = ticks.as_any().downcast_ref::<Float64Array>().unwrap();
        let tick_values: Vec<f64> = tick_array.values().to_vec();
        println!("  {} ticks: {:?}", tick_count, tick_values);
    }

    // Example 2: Scale with Custom Options
    println!("\n2. Scale with Custom Options:");

    let custom_scale = LinearScale::configured((0.0, 1000.0), (0.0, 500.0))
        .with_option("clamp", true)
        .with_option("round", true)
        .with_option("nice", true);

    let test_values = vec![-100.0, 500.0, 1200.0]; // Values outside domain
    let test_array = Arc::new(Float32Array::from(test_values.clone())) as ArrayRef;

    let clamped_result = custom_scale.scale_to_numeric(&test_array)?;
    let clamped_values = clamped_result.as_vec(test_values.len(), None);

    println!("Scale with clamping and rounding:");
    for (input, output) in test_values.iter().zip(clamped_values.iter()) {
        println!("  {:.1} → {:.1}", input, output);
    }

    // Show domain and range info
    let (domain_start, domain_end) = custom_scale.numeric_interval_domain()?;
    let (range_start, range_end) = custom_scale.numeric_interval_range()?;

    println!("\nScale configuration:");
    println!("  Domain: [{}, {}]", domain_start, domain_end);
    println!("  Range: [{}, {}]", range_start, range_end);
    println!("  Clamp: {}", custom_scale.option_boolean("clamp", false));
    println!("  Round: {}", custom_scale.option_boolean("round", false));
    println!("  Nice: {}", custom_scale.option_boolean("nice", false));

    Ok(())
}
