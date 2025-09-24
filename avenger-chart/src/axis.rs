//! Axis trait for update functionality

/// Trait for axis types that support update operations
pub trait Axis: Clone + Default + Send + Sync + 'static {
    /// Update this axis with another axis configuration
    fn update(self, other: Self) -> Self;
}

// Implement for unit type (used by coordinate systems without axes like ZeroD)
impl Axis for () {
    fn update(self, _other: Self) -> Self {
        ()
    }
}
