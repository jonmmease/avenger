//! Axis trait for update functionality

/// Trait for axis types that support update operations

#[typetag::serde(tag = "type")]
pub trait Axis: Send + Sync {
    /// Update this axis with another axis configuration
    fn update(&mut self, other: &dyn Axis);

    fn as_any(&self) -> &dyn std::any::Any;

    fn box_clone(&self) -> Box<dyn Axis>;
}

// Implement for unit type (used by coordinate systems without axes like ZeroD)
#[typetag::serde(name = "empty")]
impl Axis for () {
    fn update(&mut self, _other: &dyn Axis) {}

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn box_clone(&self) -> Box<dyn Axis> {
        Box::new(())
    }
}
