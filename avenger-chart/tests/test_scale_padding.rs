use avenger_chart::scales::Scale;
use avenger_scales::scales::linear::LinearScale;
use datafusion::prelude::*;

#[test]
fn test_scale_padding_builders() {
    // Test padding with literal
    let scale = Scale::new(LinearScale).padding(lit(10.0));

    assert!(scale.has_explicit_padding());
    assert!(scale.get_padding().is_some());

    // Test padding with numeric value wrapped in lit
    let scale = Scale::new(LinearScale).padding(lit(15.5));

    assert!(scale.has_explicit_padding());
    assert!(scale.get_padding().is_some());

    // Test no padding
    let scale = Scale::new(LinearScale).padding_none();

    assert!(!scale.has_explicit_padding());
    assert!(scale.get_padding().is_none());
}

#[test]
fn test_scale_padding_default() {
    // Test that default scale has no padding
    let scale = Scale::new(LinearScale);

    assert!(!scale.has_explicit_padding());
    assert!(scale.get_padding().is_none());
}

#[tokio::test]
async fn test_scale_padding_normalization() -> Result<(), Box<dyn std::error::Error>> {
    // Test that explicit padding is applied during normalization
    let scale = Scale::new(LinearScale)
        .domain_interval(lit(0.0), lit(100.0))
        .range_interval(lit(0.0), lit(400.0))
        .padding(lit(20.0));

    // Create configured scale to test padding option is set
    let configured = scale.create_configured_scale(400.0, 300.0).await?;

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
