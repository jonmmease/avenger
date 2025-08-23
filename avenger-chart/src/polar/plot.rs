use crate::coords::CoordinateSystem;
use crate::plot::{AxisSpec, Plot, ScaleSpec};
use crate::polar::Polar;
use crate::scales::{Auto, Scale, ScaleSpec as ScaleTypeSpec};
use std::sync::Arc;

impl Plot<Polar> {
    /// Configure r scale with inferred type
    pub fn scale_r<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        self.scale_specs
            .insert("r".to_string(), ScaleSpec::Local(Arc::new(f)));
        self
    }

    /// Configure r scale with explicit type
    pub fn scale_r_with<S: ScaleTypeSpec>(
        mut self,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        self.scale_specs.insert(
            "r".to_string(),
            ScaleSpec::Local(Arc::new(move |default_scale| {
                let typed_scale = default_scale.into_type::<S>();
                f(typed_scale).into_auto()
            })),
        );
        self
    }

    /// Reference a named scale from parent layout for r channel
    pub fn scale_r_ref<S: Into<String>>(mut self, name: S) -> Self {
        self.scale_specs
            .insert("r".to_string(), ScaleSpec::Reference(name.into()));
        self
    }

    /// Configure theta scale with inferred type
    pub fn scale_theta<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        self.scale_specs
            .insert("theta".to_string(), ScaleSpec::Local(Arc::new(f)));
        self
    }

    /// Configure theta scale with explicit type
    pub fn scale_theta_with<S: ScaleTypeSpec>(
        mut self,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        self.scale_specs.insert(
            "theta".to_string(),
            ScaleSpec::Local(Arc::new(move |default_scale| {
                let typed_scale = default_scale.into_type::<S>();
                f(typed_scale).into_auto()
            })),
        );
        self
    }

    /// Reference a named scale from parent layout for theta channel
    pub fn scale_theta_ref<S: Into<String>>(mut self, name: S) -> Self {
        self.scale_specs
            .insert("theta".to_string(), ScaleSpec::Reference(name.into()));
        self
    }

    /// Configure the radial axis
    pub fn axis_r<F>(mut self, f: F) -> Self
    where
        F: Fn(<Polar as CoordinateSystem>::Axis) -> <Polar as CoordinateSystem>::Axis
            + Send
            + Sync
            + 'static,
    {
        self.axis_specs
            .insert("r".to_string(), AxisSpec::Local(Arc::new(f)));
        self
    }

    /// Configure the angular axis
    pub fn axis_theta<F>(mut self, f: F) -> Self
    where
        F: Fn(<Polar as CoordinateSystem>::Axis) -> <Polar as CoordinateSystem>::Axis
            + Send
            + Sync
            + 'static,
    {
        self.axis_specs
            .insert("theta".to_string(), AxisSpec::Local(Arc::new(f)));
        self
    }
}
