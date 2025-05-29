use arrow::array::{ArrayRef, Float32Array};
use avenger_scales::scales::{log::LogScale, pow::PowScale, symlog::SymlogScale};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Logarithmic and Power Scale Examples ===\n");

    // Example 1: Basic Log Scale
    println!("1. Log Scale (base 10, domain [1, 1000]):");

    let log_scale = LogScale::new((1.0, 1000.0), (0.0, 300.0)).with_option("base", 10.0);

    let test_values = vec![1.0, 10.0, 100.0, 1000.0];
    let values_array = Arc::new(Float32Array::from(test_values.clone())) as ArrayRef;

    let scaled_result = log_scale.scale_to_numeric(&values_array)?;
    let scaled_values = scaled_result.as_vec(test_values.len(), None);

    println!("Log10 scale mapping:");
    for (input, output) in test_values.iter().zip(scaled_values.iter()) {
        println!("  {:.0} → {:.1}", input, output);
    }

    // Test log scale ticks
    println!("\nLog scale ticks (10 ticks requested):");
    let ticks = log_scale.ticks(Some(10.0))?;
    let tick_array = ticks.as_any().downcast_ref::<Float32Array>().unwrap();
    let tick_values: Vec<f32> = tick_array.values().to_vec();

    println!("Tick values: {:?}", tick_values);

    // Example 2: Log Scale with Different Base
    println!("\n2. Log Scale (base 2, for computer science applications):");

    let log2_scale = LogScale::new((1.0, 64.0), (0.0, 200.0)).with_option("base", 2.0);

    let powers_of_2 = vec![1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
    let pow2_array = Arc::new(Float32Array::from(powers_of_2.clone())) as ArrayRef;

    let log2_result = log2_scale.scale_to_numeric(&pow2_array)?;
    let log2_values = log2_result.as_vec(powers_of_2.len(), None);

    println!("Log2 scale mapping (powers of 2):");
    for (input, output) in powers_of_2.iter().zip(log2_values.iter()) {
        println!("  {:.0} → {:.1}", input, output);
    }

    // Example 3: Symlog Scale (handles zero and negative values)
    println!("\n3. Symlog Scale (handles negative values and zero):");

    let symlog_scale =
        SymlogScale::new((-1000.0, 1000.0), (0.0, 400.0)).with_option("constant", 100.0); // Linear region around zero: [-100, 100]

    let symlog_values = vec![-1000.0, -100.0, -10.0, 0.0, 10.0, 100.0, 1000.0];
    let symlog_array = Arc::new(Float32Array::from(symlog_values.clone())) as ArrayRef;

    let symlog_result = symlog_scale.scale_to_numeric(&symlog_array)?;
    let symlog_scaled = symlog_result.as_vec(symlog_values.len(), None);

    println!("Symlog scale mapping (linear around zero, log elsewhere):");
    for (input, output) in symlog_values.iter().zip(symlog_scaled.iter()) {
        println!("  {:.0} → {:.1}", input, output);
    }

    // Example 4: Power Scale (square root, good for area mappings)
    println!("\n4. Power Scale (square root, for area-based visualizations):");

    let sqrt_scale = PowScale::new((0.0, 100.0), (0.0, 200.0)).with_option("exponent", 0.5); // Square root

    let area_values = vec![
        0.0, 1.0, 4.0, 9.0, 16.0, 25.0, 36.0, 49.0, 64.0, 81.0, 100.0,
    ];
    let area_array = Arc::new(Float32Array::from(area_values.clone())) as ArrayRef;

    let sqrt_result = sqrt_scale.scale_to_numeric(&area_array)?;
    let sqrt_scaled = sqrt_result.as_vec(area_values.len(), None);

    println!("Square root scale mapping (good for circle areas):");
    for (input, output) in area_values.iter().zip(sqrt_scaled.iter()) {
        println!("  {:.0} → {:.1}", input, output);
    }

    // Example 5: Scale Inversion
    println!("\n5. Scale Inversion Example:");

    let range_values = vec![0.0, 100.0, 200.0, 300.0];
    let range_array = Arc::new(Float32Array::from(range_values.clone())) as ArrayRef;

    let inverted_result = log_scale.invert_from_numeric(&range_array)?;
    let inverted_values = inverted_result.as_vec(range_values.len(), None);

    println!("Log scale inversion (range → domain):");
    for (range_val, domain_val) in range_values.iter().zip(inverted_values.iter()) {
        println!("  {:.1} → {:.2}", range_val, domain_val);
    }

    Ok(())
}
