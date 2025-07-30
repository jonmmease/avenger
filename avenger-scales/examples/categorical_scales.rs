use arrow::array::{Array, ArrayRef, StringArray};
use avenger_scales::scales::{band::BandScale, ordinal::OrdinalScale, point::PointScale};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Categorical Scale Examples ===\n");

    // Example 1: Band Scale for Bar Charts
    println!("1. Band Scale (for bar charts with bandwidth):");

    let categories = vec!["Category A", "Category B", "Category C", "Category D"];
    let domain = Arc::new(StringArray::from(categories.clone())) as ArrayRef;

    let band_scale = BandScale::configured(domain.clone(), (0.0, 400.0))
        .with_option("padding_inner", 0.1) // 10% padding between bands
        .with_option("padding", 0.05); // 5% padding on outer edges

    let test_array = Arc::new(StringArray::from(categories.clone())) as ArrayRef;
    let positions = band_scale.scale_to_numeric(&test_array)?;
    let position_values = positions.as_vec(categories.len(), None);

    // Get bandwidth for drawing bars
    let bandwidth = band_scale.option_f32("bandwidth", 0.0);

    println!("Category positions and bandwidth:");
    for (category, position) in categories.iter().zip(position_values.iter()) {
        println!(
            "  '{}' → position: {:.1}, bandwidth: {:.1}",
            category, position, bandwidth
        );
    }

    // Example 2: Point Scale for Scatter Plots
    println!("\n2. Point Scale (for scatter plots, no bandwidth):");

    let point_scale =
        PointScale::configured(domain.clone(), (0.0, 300.0)).with_option("padding", 0.5); // Space around the points

    let point_positions = point_scale.scale_to_numeric(&test_array)?;
    let point_values = point_positions.as_vec(categories.len(), None);

    println!("Point positions:");
    for (category, position) in categories.iter().zip(point_values.iter()) {
        println!("  '{}' → position: {:.1}", category, position);
    }

    // Example 3: Ordinal Scale for Categorical Mappings
    println!("\n3. Ordinal Scale (categorical → categorical mapping):");

    let categories = vec!["small", "medium", "large"];
    let cat_domain = Arc::new(StringArray::from(categories.clone())) as ArrayRef;

    // Map size categories to stroke cap styles (using snake_case for JSON serialization)
    let stroke_cap_values = vec!["round", "square", "butt"];
    let stroke_range = Arc::new(StringArray::from(stroke_cap_values)) as ArrayRef;

    let ordinal_scale = OrdinalScale::configured(cat_domain).with_range(stroke_range);

    let test_categories = Arc::new(StringArray::from(categories.clone())) as ArrayRef;
    let stroke_result = ordinal_scale.scale_to_stroke_cap(&test_categories)?;
    let stroke_caps_result = stroke_result.as_vec(categories.len(), None);

    println!("Size → Stroke cap mapping:");
    for (size, cap) in categories.iter().zip(stroke_caps_result.iter()) {
        println!("  '{}' → {:?}", size, cap);
    }

    // Example 4: Band Scale with Range Inversion
    println!("\n4. Band Scale Range Inversion (find category from position):");

    // Note: This would require implementing invert_from_numeric for band scale
    // For now, let's show the range interval inversion
    let range_interval = (100.0, 200.0);
    match band_scale.invert_range_interval(range_interval) {
        Ok(inverted_domain) => {
            println!("Range interval {:?} maps to domain values:", range_interval);
            // Print the resulting domain subset
            let string_array = inverted_domain
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            for i in 0..string_array.len() {
                if let Some(value) = string_array.value(i).to_string().as_str().get(..) {
                    println!("  '{}'", value);
                }
            }
        }
        Err(_) => println!("Range interval inversion not yet implemented for band scale"),
    }

    Ok(())
}
