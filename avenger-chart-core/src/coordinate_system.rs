/// Core-safe coordinate-system authoring contract.
///
/// This trait contains only the coordinate-system metadata needed by generic
/// mark authoring. The top-level chart crate layers guide construction,
/// serializable transform creation, and layout measurement on top.
pub trait CoordinateSystemCore: Sized + Send + Sync + 'static {
    /// Get the names of position channels required by this coordinate system.
    fn required_channels(&self) -> &'static [&'static str];
}
