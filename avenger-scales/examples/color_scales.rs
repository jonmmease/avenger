use arrow::array::{ArrayRef, Float32Array};
use avenger_common::types::ColorOrGradient;
use avenger_scales::scales::{linear::LinearScale, log::LogScale};
use datafusion_common::utils::arrays_into_list_array;
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Color Scale Examples ===\n");

    // Example 1: Linear scale with color interpolation
    println!("1. Linear Scale with Color Gradient (Blue → Red):");

    let blue = [0.0, 0.0, 1.0, 1.0]; // Blue
    let red = [1.0, 0.0, 0.0, 1.0]; // Red

    let color_arrays = vec![
        Arc::new(Float32Array::from(Vec::from(blue))) as ArrayRef,
        Arc::new(Float32Array::from(Vec::from(red))) as ArrayRef,
    ];
    let color_range = Arc::new(arrays_into_list_array(color_arrays)?) as ArrayRef;

    let linear_color_scale = LinearScale::new((0.0, 10.0), (0.0, 1.0)).with_range(color_range);

    let test_values = vec![0.0, 2.5, 5.0, 7.5, 10.0];
    let values_array = Arc::new(Float32Array::from(test_values.clone())) as ArrayRef;

    let color_result = linear_color_scale.scale_to_color(&values_array)?;
    let colors = color_result.as_vec(test_values.len(), None);

    for (value, color) in test_values.iter().zip(colors.iter()) {
        match color {
            ColorOrGradient::Color(rgba) => {
                println!(
                    "  {:.1} → RGBA({:.2}, {:.2}, {:.2}, {:.2})",
                    value, rgba[0], rgba[1], rgba[2], rgba[3]
                );
            }
            _ => println!("  {:.1} → {:?}", value, color),
        }
    }

    println!("\n2. Log Scale with 3-Color Gradient (Red → Yellow → Blue):");

    let red = [1.0, 0.0, 0.0, 1.0]; // Red
    let yellow = [1.0, 1.0, 0.0, 1.0]; // Yellow
    let blue = [0.0, 0.0, 1.0, 1.0]; // Blue

    let color_arrays = vec![
        Arc::new(Float32Array::from(Vec::from(red))) as ArrayRef,
        Arc::new(Float32Array::from(Vec::from(yellow))) as ArrayRef,
        Arc::new(Float32Array::from(Vec::from(blue))) as ArrayRef,
    ];
    let three_color_range = Arc::new(arrays_into_list_array(color_arrays)?) as ArrayRef;

    let log_color_scale = LogScale::new((1.0, 100.0), (0.0, 1.0)).with_range(three_color_range);

    let log_test_values = vec![1.0, 3.16, 10.0, 31.6, 100.0]; // Evenly spaced in log space
    let log_values_array = Arc::new(Float32Array::from(log_test_values.clone())) as ArrayRef;

    let log_color_result = log_color_scale.scale_to_color(&log_values_array)?;
    let log_colors = log_color_result.as_vec(log_test_values.len(), None);

    for (value, color) in log_test_values.iter().zip(log_colors.iter()) {
        match color {
            ColorOrGradient::Color(rgba) => {
                println!(
                    "  {:.2} → RGBA({:.2}, {:.2}, {:.2}, {:.2})",
                    value, rgba[0], rgba[1], rgba[2], rgba[3]
                );
            }
            _ => println!("  {:.2} → {:?}", value, color),
        }
    }

    println!("\n3. Generating Gradient Stops for CSS/SVG:");

    let gradient_stops = log_color_scale.color_range_as_gradient_stops(5)?;
    println!("Gradient stops for log scale:");
    for stop in gradient_stops {
        println!(
            "  offset: {:.2}, color: RGBA({:.2}, {:.2}, {:.2}, {:.2})",
            stop.offset, stop.color[0], stop.color[1], stop.color[2], stop.color[3]
        );
    }

    Ok(())
}
