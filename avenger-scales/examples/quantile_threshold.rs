use arrow::array::{ArrayRef, Float32Array, StringArray};
use avenger_scales::scales::{
    quantile::QuantileScale, quantize::QuantizeScale, threshold::ThresholdScale,
};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Quantile and Threshold Scale Examples ===\n");

    // Sample dataset: test scores
    let test_scores = vec![
        45.0, 52.0, 58.0, 61.0, 67.0, 69.0, 72.0, 74.0, 76.0, 78.0, 80.0, 82.0, 84.0, 86.0, 87.0,
        89.0, 91.0, 93.0, 95.0, 98.0,
    ];

    println!("Sample dataset (test scores): {:?}\n", test_scores);

    // Example 1: Quantile Scale
    println!("1. Quantile Scale (data-driven quantiles):");

    let grade_range = Arc::new(StringArray::from(vec!["F", "D", "C", "B", "A"])) as ArrayRef;

    let quantile_scale = QuantileScale::new(test_scores.clone(), grade_range);

    // Test with some sample scores
    let sample_scores = vec![50.0, 65.0, 75.0, 85.0, 95.0];
    let sample_array = Arc::new(Float32Array::from(sample_scores.clone())) as ArrayRef;

    let quantile_result = quantile_scale.scale_to_string(&sample_array)?;
    let grades = quantile_result.as_vec(sample_scores.len(), None);

    println!("Quantile-based grading (based on actual score distribution):");
    for (score, grade) in sample_scores.iter().zip(grades.iter()) {
        println!("  {:.0} → '{}'", score, grade);
    }

    // Example 2: Quantize Scale
    println!("\n2. Quantize Scale (uniform intervals):");

    let grade_range2 = Arc::new(StringArray::from(vec!["F", "D", "C", "B", "A"])) as ArrayRef;
    let quantize_scale = QuantizeScale::new((0.0, 100.0), grade_range2).with_option("nice", true);

    let quantize_result = quantize_scale.scale_to_string(&sample_array)?;
    let quantize_grades = quantize_result.as_vec(sample_scores.len(), None);

    println!("Quantize-based grading (uniform 20-point intervals):");
    for (score, grade) in sample_scores.iter().zip(quantize_grades.iter()) {
        println!("  {:.0} → '{}'", score, grade);
    }

    // Example 3: Threshold Scale with Custom Breakpoints
    println!("\n3. Threshold Scale (custom breakpoints):");

    // Custom grade thresholds: F<60, D<70, C<80, B<90, A>=90
    let thresholds = vec![60.0, 70.0, 80.0, 90.0];
    let grade_range3 = Arc::new(StringArray::from(vec!["F", "D", "C", "B", "A"])) as ArrayRef;

    let threshold_scale = ThresholdScale::new(thresholds, grade_range3);

    let threshold_result = threshold_scale.scale_to_string(&sample_array)?;
    let threshold_grades = threshold_result.as_vec(sample_scores.len(), None);

    println!("Threshold-based grading (custom breakpoints: <60=F, <70=D, <80=C, <90=B, >=90=A):");
    for (score, grade) in sample_scores.iter().zip(threshold_grades.iter()) {
        println!("  {:.0} → '{}'", score, grade);
    }

    // Example 4: Threshold Scale for Risk Categories
    println!("\n4. Threshold Scale for Risk Assessment:");

    let risk_values = vec![0.05, 0.15, 0.35, 0.65, 0.85];
    let risk_array = Arc::new(Float32Array::from(risk_values.clone())) as ArrayRef;

    // Risk thresholds: Low<0.2, Medium<0.5, High<0.8, Critical>=0.8
    let risk_thresholds = vec![0.2, 0.5, 0.8];
    let risk_categories =
        Arc::new(StringArray::from(vec!["Low", "Medium", "High", "Critical"])) as ArrayRef;

    let risk_scale = ThresholdScale::new(risk_thresholds, risk_categories);

    let risk_result = risk_scale.scale_to_string(&risk_array)?;
    let risk_labels = risk_result.as_vec(risk_values.len(), None);

    println!("Risk assessment (thresholds: <0.2=Low, <0.5=Medium, <0.8=High, >=0.8=Critical):");
    for (risk, category) in risk_values.iter().zip(risk_labels.iter()) {
        println!("  {:.2} → '{}'", risk, category);
    }

    Ok(())
}
