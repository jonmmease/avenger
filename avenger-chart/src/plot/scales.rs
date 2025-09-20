//! Scale configuration methods for Plot

use crate::coords::CoordinateSystem;
use crate::plot::Plot;
use crate::scales::{Auto, Scale, ScaleSpec as ScaleTypeSpec};
use std::sync::Arc;

use super::specs::ScaleSpec;

/// Methods for adding scales to Plot
impl<C: CoordinateSystem> Plot<C> {
    /// Configure a scale by channel name
    pub fn scale<F>(mut self, channel: &str, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        self.scale_specs
            .insert(channel.to_string(), ScaleSpec::Local(Arc::new(f)));
        self
    }

    /// Configure a scale with explicit scale type
    pub fn scale_with<S: ScaleTypeSpec>(
        mut self,
        channel: &str,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        self.scale_specs.insert(
            channel.to_string(),
            ScaleSpec::Local(Arc::new(move |default_scale| {
                let typed_scale = default_scale.into_type::<S>();
                f(typed_scale).into_auto()
            })),
        );
        self
    }
}
