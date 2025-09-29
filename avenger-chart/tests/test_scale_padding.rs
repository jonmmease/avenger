use avenger_chart::prelude::*;
use datafusion::logical_expr::lit;

#[test]
fn test_scale_padding_builders() {
    // Test padding with numeric value
    let _scale = Scale::<Linear>::new().padding(10.0);

    // Test padding with different numeric value
    let _scale = Scale::<Linear>::new().padding(15.5);
}

#[test]
fn test_scale_padding_default() {
    // Test that default scale compiles
    let _scale = Scale::<Linear>::new();
}

#[tokio::test]
async fn test_scale_padding_normalization() -> Result<(), Box<dyn std::error::Error>> {
    use datafusion::prelude::SessionContext;

    // Test that explicit padding is applied during normalization
    let ctx = SessionContext::new();
    let scale = Scale::<Linear>::new()
        .domain_interval(lit(0.0), lit(100.0))
        .range_interval(lit(0.0), lit(400.0))
        .padding(20.0);

    // Create configured scale to test padding option is set
    let empty_params = indexmap::IndexMap::new();
    let configured = scale.create_configured_scale(400.0, 300.0, &ctx, &empty_params).await?;

    // Check that clip_padding options were added
    let clip_padding_lower = configured.config.options.get("clip_padding_lower");
    let clip_padding_upper = configured.config.options.get("clip_padding_upper");
    assert!(
        clip_padding_lower.is_some(),
        "clip_padding_lower option should be set"
    );
    assert!(
        clip_padding_upper.is_some(),
        "clip_padding_upper option should be set"
    );

    Ok(())
}
