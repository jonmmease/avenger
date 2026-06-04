//! Axis trait for update functionality

use std::any::Any;

use datafusion::{logical_expr::Expr, prelude::SessionContext};

/// Trait for axis types that support update operations

#[typetag::serde(tag = "type")]
pub trait Axis: Send + Sync {
    /// Update this axis with another axis configuration
    fn update(&mut self, other: &dyn Axis);

    fn as_any(&self) -> &dyn Any;

    fn box_clone(&self) -> Box<dyn Axis>;

    /// Return expressions referenced by this axis configuration.
    fn all_exprs(&self, _ctx: &SessionContext) -> Vec<Expr> {
        Vec::new()
    }
}

impl Clone for Box<dyn Axis> {
    fn clone(&self) -> Self {
        self.box_clone()
    }
}

// Implement for unit type (used by coordinate systems without axes like ZeroD)
#[typetag::serde(name = "empty")]
impl Axis for () {
    fn update(&mut self, _other: &dyn Axis) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn box_clone(&self) -> Box<dyn Axis> {
        Box::new(())
    }
}
