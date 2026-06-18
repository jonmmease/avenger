use crate::{CoordinateGuide, CoordinateScaleSource, CoordinateSystemTransform};

/// Core-safe coordinate-system authoring contract.
///
/// This trait contains only the coordinate-system metadata needed by generic
/// mark authoring. The top-level chart crate layers guide construction,
/// serializable transform creation, and layout measurement on top.
pub trait CoordinateSystemCore: Sized + Send + Sync + 'static {
    /// Get the names of position channels required by this coordinate system.
    fn required_channels(&self) -> &'static [&'static str];
}

/// Core coordinate-system authoring contract.
///
/// Coordinate-system crates implement this trait to provide their guide type
/// and serializable transform. Top-level layout measurement remains a facade
/// concern and is dispatched separately for built-in facet/concat/container
/// coordinates.
pub trait CoordinateSystem: CoordinateSystemCore {
    /// The guide type for this coordinate system.
    type Guide: CoordinateGuide;

    /// Create a boxed coordinate-system transform for scale building and mark
    /// rendering.
    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform>;

    /// Non-rendered scale/domain sources owned by this coordinate system.
    ///
    /// Most coordinates return no sources because rendered marks declare all
    /// scales through their channels. Coordinates with dynamic scale families
    /// can return sources here so scale building, guide extraction, and domain
    /// coordination see those scales before rendering.
    fn coordinate_scale_sources(&self) -> Vec<CoordinateScaleSource> {
        Vec::new()
    }
}
