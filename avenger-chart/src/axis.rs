use std::any::Any;

/// Trait for all axis types
///
/// The clone_box, as_any, and into_any methods enable storing different
/// axis types in a type-erased collection while preserving the ability
/// to downcast back to concrete types when needed.
pub trait AxisTrait: Send + Sync {
    fn clone_box(&self) -> Box<dyn AxisTrait>;
    fn as_any(&self) -> &dyn Any;
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

/// Position for axes (common across coordinate systems)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AxisPosition {
    Top,
    Right,
    Bottom,
    Left,
}

/// Specification for an axis in a plot (generic over coordinate system axis types)
#[derive(Clone, Debug)]
pub struct AxisSpec<A: AxisTrait> {
    pub axis: A,
}
