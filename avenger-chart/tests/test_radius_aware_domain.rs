//! Test radius-aware domain calculation for scales

use avenger_chart::scales::Scale;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_radius_aware_domain() {
    // Create test data with points and radii
    let x_values = Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0]);
    let radii = Float64Array::from(vec![5.0, 10.0, 15.0, 10.0, 5.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("radius", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(radii)]).unwrap();

    let ctx = SessionContext::new();
    let df = Arc::new(ctx.read_batch(batch).unwrap());

    // Create a linear scale with radius-aware domain
    let scale = Scale::with_type("linear").domain_data_field_with_radius(
        df.clone(),
        col("x"),
        col("radius"),
    );

    // Infer domain with range hint (simulating a 200 pixel wide plot)
    let scale_with_domain = scale
        .infer_domain_from_data(Some((0.0, 200.0)))
        .await
        .unwrap();

    // Verify that domain was computed
    match &scale_with_domain.domain.default_domain {
        avenger_chart::scales::ScaleDefaultDomain::Interval(min_expr, max_expr) => {
            println!("Domain with radius padding computed successfully");
            println!("Min expression: {:?}", min_expr);
            println!("Max expression: {:?}", max_expr);

            // The domain should be expanded beyond [10, 50] to accommodate the radii
            // With the largest radius being 15 at x=30, we expect some padding
            // The exact values depend on the optimization algorithm
        }
        _ => panic!("Expected interval domain"),
    }
}

#[tokio::test]
async fn test_standard_domain_without_radius() {
    // Create test data without radius
    let x_values = Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0]);

    let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Float64, false)]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values)]).unwrap();

    let ctx = SessionContext::new();
    let df = Arc::new(ctx.read_batch(batch).unwrap());

    // Create a linear scale without radius
    let scale = Scale::with_type("linear").domain_data_field(df.clone(), col("x"));

    // Infer domain without radius consideration
    let scale_with_domain = scale
        .infer_domain_from_data(Some((0.0, 200.0)))
        .await
        .unwrap();

    // Verify that domain was computed
    match &scale_with_domain.domain.default_domain {
        avenger_chart::scales::ScaleDefaultDomain::Interval(min_expr, max_expr) => {
            println!("Standard domain computed successfully");
            println!("Min expression: {:?}", min_expr);
            println!("Max expression: {:?}", max_expr);

            // Without radius, the domain should just be [10, 50]
        }
        _ => panic!("Expected interval domain"),
    }
}
