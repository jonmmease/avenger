use std::any::Any;

use crate::CoordinatedOverflow;

/// Coordinate-system-specific measurement data computed during the measure phase.
///
/// Coordinate systems can compute layout data once during measurement and make
/// it available to guides and marks during rendering. Top-level layout
/// containers may downcast this trait to their own concrete measurement types
/// for core-owned facet/concat behavior.
pub trait CoordMeasurement: Send + Sync + 'static {
    /// Downcast support for accessing concrete measurement types.
    fn as_any(&self) -> &dyn Any;

    /// Mutable downcast support for coordination phases.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Get coordinated overflow after coordination phase.
    ///
    /// Returns `None` for non-coordinatable measurements.
    fn coordinated_overflow(&self) -> Option<&CoordinatedOverflow> {
        None
    }
}

/// Empty measurement for coordinate systems that don't need measurement data.
#[derive(Debug, Clone, Default)]
pub struct EmptyCoordMeasurement;

impl CoordMeasurement for EmptyCoordMeasurement {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
