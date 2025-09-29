//! Default scale creation using theme and trait-based type detection

use crate::error::AvengerChartError;
use crate::render::RenderContext;
use crate::scales::spec::ScaleSpec;
use crate::scales::{Auto, Scale};

/// Create a default scale for a channel using theme and scale traits
pub fn create_default_scale_for_channel(
    channel: &str,
    scale_spec: Box<dyn ScaleSpec>,
    context: &RenderContext,
) -> Result<Scale<Auto>, AvengerChartError> {
    let range_kind = scale_spec.range_kind();

    let mut scale = Scale::<Auto>::from_spec(scale_spec);

    // Use generic "mark" type for global scales
    // Individual marks will override with their specific type in default_channel_range
    let mark_type = "mark";

    // Use the new get_range_for_channel method which supports CSS discrete/continuous properties
    let range = context
        .theme
        .get_range_for_channel(mark_type, channel, range_kind, None);
    scale = scale.range(range);

    Ok(scale)
}
