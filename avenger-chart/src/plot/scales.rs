//! Scale configuration methods for Plot

use crate::{
    coords::CoordinateSystem,
    plot::Plot,
    scales::{PlotScaleSpec as ScaleSpec, ScaleSpec as ScaleTypeSpec},
};
use avenger_chart_core::{Auto, Scale};

/// Methods for adding scales to Plot
impl<C: CoordinateSystem> Plot<C> {
    /// Configure a scale by channel name
    pub fn scale<F>(mut self, channel: &str, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        // Create a Scale and apply the configuration
        let scale_changes = f(Scale::new()).into_config();
        self.scale_specs
            .insert(channel.to_string(), ScaleSpec::Local(scale_changes));
        self
    }

    /// Configure a scale with explicit scale type
    pub fn scale_with<S: ScaleTypeSpec + Default>(
        mut self,
        channel: &str,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        // Create a typed scale and apply the configuration
        let typed_scale = Scale::<S>::new();
        let scale_changes = f(typed_scale).into_auto().into_config();
        self.scale_specs
            .insert(channel.to_string(), ScaleSpec::Local(scale_changes));
        self
    }
}
