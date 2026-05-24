/// Trait for composable guide updates.
///
/// This trait allows guides to be composed by updating one with another,
/// similar to how axes can be composed using the axis update trait.
pub trait GuideUpdate: Clone + Default {
    /// Update this guide with values from another guide.
    ///
    /// Values from `other` take precedence over values in `self`.
    fn update(self, other: Self) -> Self;
}
