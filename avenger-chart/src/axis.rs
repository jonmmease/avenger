//! Axis trait for update functionality

/// Trait for axis types that support update operations
pub trait AxisUpdate: Clone + Default {
    /// Update this axis with another axis configuration
    fn update(self, other: Self) -> Self;
}

// Implement for unit type (used by coordinate systems without axes like ZeroD)
impl AxisUpdate for () {
    fn update(self, _other: Self) -> Self {
        ()
    }
}
