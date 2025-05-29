use arrow::array::{ArrayRef, Float32Array};
use avenger_scales::scales::linear::LinearScale;
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Simple Linear Scale Example ===\n");

    // Create a linear scale that maps domain [0, 100] to range [0, 500]
    let scale = LinearScale::new((0.0, 100.0), (0.0, 500.0));

    // Test data: some values in our domain
    let test_values = vec![0.0, 25.0, 50.0, 75.0, 100.0];
    let values_array = Arc::new(Float32Array::from(test_values.clone())) as ArrayRef;

    // Scale the values
    let scaled_result = scale.scale_to_numeric(&values_array)?;
    let scaled_values = scaled_result.as_vec(test_values.len(), None);

    println!("Linear Scale Mapping (domain [0, 100] → range [0, 500]):");
    for (input, output) in test_values.iter().zip(scaled_values.iter()) {
        println!("  {:.1} → {:.1}", input, output);
    }

    println!("\n=== Inverse Scaling ===");

    // Test inverse scaling: convert range values back to domain values
    let range_values = vec![0.0, 125.0, 250.0, 375.0, 500.0];
    let range_array = Arc::new(Float32Array::from(range_values.clone())) as ArrayRef;

    let inverted_result = scale.invert_from_numeric(&range_array)?;
    let inverted_values = inverted_result.as_vec(range_values.len(), None);

    println!("Inverse Scale Mapping (range [0, 500] → domain [0, 100]):");
    for (input, output) in range_values.iter().zip(inverted_values.iter()) {
        println!("  {:.1} → {:.1}", input, output);
    }

    println!("\n=== Domain/Range Information ===");
    let (domain_start, domain_end) = scale.numeric_interval_domain()?;
    let (range_start, range_end) = scale.numeric_interval_range()?;

    println!("Domain: [{}, {}]", domain_start, domain_end);
    println!("Range: [{}, {}]", range_start, range_end);

    Ok(())
}
