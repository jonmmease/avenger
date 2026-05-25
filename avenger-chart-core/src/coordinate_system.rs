use crate::{CoordinateGuide, CoordinateSystemTransform};

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
}
