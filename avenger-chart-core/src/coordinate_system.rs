use std::{collections::HashMap, sync::Arc};

use crate::{
    AvengerChartError, Axis, CoordinateGuide, CoordinateSystemTransform, MarkState, RepeatContext,
};

/// Core-safe coordinate-system authoring contract.
///
/// This trait contains only the coordinate-system metadata needed by generic
/// mark authoring. The top-level chart crate layers guide construction,
/// serializable transform creation, and layout measurement on top.
pub trait CoordinateSystemCore: Sized + Clone + Send + Sync + 'static {
    /// Get the names of position channels required by this coordinate system.
    fn required_channels(&self) -> &'static [&'static str];

    /// Validate coordinate-system authoring state before compilation.
    fn validate(&self) -> Result<(), AvengerChartError> {
        Ok(())
    }

    /// Resolve repeat placeholders in coordinate-owned authoring state.
    ///
    /// Most coordinate systems have no expressions on the coordinate itself and
    /// can use the default clone. Coordinates with expression-bearing frame or
    /// guide metadata can override this; data-bearing position expressions
    /// should usually live on mark channels.
    fn resolve_repeat(&self, _ctx: &RepeatContext) -> Result<Self, AvengerChartError> {
        Ok(self.clone())
    }

    /// Resolve coordinate-system state that depends on authored mark channels.
    ///
    /// Most coordinate systems have fixed position channels and can use the
    /// default clone. Coordinates with open-ended position families, such as
    /// parallel coordinates, can scan mark states to discover the frame they
    /// should render.
    fn resolve_from_mark_states(&self, _states: &[MarkState]) -> Result<Self, AvengerChartError> {
        Ok(self.clone())
    }

    /// Axis configurations owned by the resolved coordinate frame.
    ///
    /// This is for frame metadata and coordinate-level guide overrides only;
    /// data-bearing position channels should still live on marks.
    fn coordinate_axis_configs(&self) -> HashMap<String, Arc<dyn Axis>> {
        HashMap::new()
    }
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
