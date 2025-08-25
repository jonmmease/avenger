use crate::coords::CoordinateSystem;
use crate::plot::{Plot, ScaleSpec};
use crate::scales::{Auto, Scale, ScaleSpec as ScaleTypeSpec};
use std::sync::Arc;

/// Methods for adding scales to Plot
impl<C: CoordinateSystem> Plot<C> {
    /// Configure a scale by channel name
    ///
    /// This is an internal method. Use position-specific methods like `scale_x`, `scale_y`
    /// for position channels, or channel-specific scale methods on marks for other channels.
    #[doc(hidden)]
    pub fn _scale<F>(mut self, channel: &str, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        self.scale_specs
            .insert(channel.to_string(), ScaleSpec::Local(Arc::new(f)));
        self
    }

    /// Configure a scale with explicit type
    ///
    /// This is an internal method. Use position-specific methods like `scale_x_with`, `scale_y_with`
    /// for position channels, or channel-specific scale methods on marks for other channels.
    #[doc(hidden)]
    pub fn _scale_with<S: ScaleTypeSpec>(
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
