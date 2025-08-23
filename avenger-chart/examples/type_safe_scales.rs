//! Example demonstrating the new type-safe scale API

use avenger_chart::scales::{Band, Linear, Ordinal, Scale};
use datafusion::logical_expr::lit;

fn main() {
    // Create a linear scale with type-specific methods
    let linear_scale = Scale::<Linear>::new()
        .domain_interval(lit(0.0), lit(100.0))
        .range_interval(lit(0.0), lit(500.0))
        .nice(true) // ✓ Linear has nice()
        .zero(true) // ✓ Linear has zero()
        .clamp(false) // ✓ Linear has clamp()
        .padding(0.1); // ✓ Linear has padding()
    // .padding_inner(0.5) // ✗ Won't compile - Linear doesn't have padding_inner()

    println!(
        "Created linear scale with domain: {:?}",
        linear_scale.get_domain()
    );

    // Create a band scale with different type-specific methods
    let band_scale = Scale::<Band>::new()
        .domain_discrete(vec![lit("A"), lit("B"), lit("C"), lit("D")])
        .range_interval(lit(0.0), lit(500.0))
        .padding_inner(0.1) // ✓ Band has padding_inner()
        .padding_outer(0.05) // ✓ Band has padding_outer()
        .align(0.5) // ✓ Band has align()
        .round(true); // ✓ Band has round()
    // .nice(true)          // ✗ Won't compile - Band doesn't have nice()

    println!(
        "Created band scale with domain: {:?}",
        band_scale.get_domain()
    );

    // Create an ordinal scale for colors
    let color_scale = Scale::<Ordinal>::new()
        .domain_discrete(vec![lit("cat"), lit("dog"), lit("bird")])
        .range_discrete(vec!["red", "blue", "green"])
        .unknown(lit("gray")); // ✓ Ordinal has unknown()

    println!("Created ordinal scale for categorical colors");

    // Convert to Auto type for storage (type erasure)
    use avenger_chart::scales::Auto;
    let scales: Vec<Scale<Auto>> = vec![
        linear_scale.into_auto(),
        band_scale.into_auto(),
        color_scale.into_auto(),
    ];

    println!("Stored {} scales in a collection", scales.len());

    // All scales can report their type
    for scale in &scales {
        println!("Scale type: {}", scale.get_scale_type());
    }
}
