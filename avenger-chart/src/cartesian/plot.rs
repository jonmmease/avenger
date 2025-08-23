use crate::cartesian::{Cartesian, CartesianAxis};
use crate::plot::{AxisSpec, Plot, ScaleSpec};
use crate::scales::{Auto, Scale, ScaleSpec as ScaleTypeSpec};
use std::sync::Arc;

// Generic implementation for any CartesianAxis type
impl<A> Plot<Cartesian<A>>
where
    A: CartesianAxis + Default + 'static,
{
    /// Configure x scale with inferred type
    pub fn scale_x<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        self.scale_specs
            .insert("x".to_string(), ScaleSpec::Local(Arc::new(f)));
        self
    }

    /// Configure x scale with explicit type
    pub fn scale_x_with<S: ScaleTypeSpec>(
        mut self,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        self.scale_specs.insert(
            "x".to_string(),
            ScaleSpec::Local(Arc::new(move |default_scale| {
                // Convert the default scale to the requested type
                // This changes the scale_impl to match type S while preserving domain/range/options
                let typed_scale = default_scale.into_type::<S>();
                f(typed_scale).into_auto()
            })),
        );
        self
    }

    /// Reference a named scale from parent layout for x channel
    pub fn scale_x_ref<S: Into<String>>(mut self, name: S) -> Self {
        self.scale_specs
            .insert("x".to_string(), ScaleSpec::Reference(name.into()));
        self
    }

    /// Configure y scale with inferred type
    pub fn scale_y<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        self.scale_specs
            .insert("y".to_string(), ScaleSpec::Local(Arc::new(f)));
        self
    }

    /// Configure y scale with explicit type
    pub fn scale_y_with<S: ScaleTypeSpec>(
        mut self,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        self.scale_specs.insert(
            "y".to_string(),
            ScaleSpec::Local(Arc::new(move |default_scale| {
                // Convert the default scale to the requested type
                // This changes the scale_impl to match type S while preserving domain/range/options
                let typed_scale = default_scale.into_type::<S>();
                f(typed_scale).into_auto()
            })),
        );
        self
    }

    /// Reference a named scale from parent layout for y channel
    pub fn scale_y_ref<S: Into<String>>(mut self, name: S) -> Self {
        self.scale_specs
            .insert("y".to_string(), ScaleSpec::Reference(name.into()));
        self
    }

    pub fn axis_x<F>(mut self, f: F) -> Self
    where
        F: Fn(A) -> A + Send + Sync + 'static,
    {
        self.axis_specs
            .insert("x".to_string(), AxisSpec::Local(Arc::new(f)));
        self
    }

    pub fn axis_y<F>(mut self, f: F) -> Self
    where
        F: Fn(A) -> A + Send + Sync + 'static,
    {
        self.axis_specs
            .insert("y".to_string(), AxisSpec::Local(Arc::new(f)));
        self
    }

    /// Add an alternative y-axis scale with a custom name
    pub fn scale_y_alt<N: Into<String>, F>(mut self, name: N, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        let name = name.into();
        self.scale_specs
            .insert(name.clone(), ScaleSpec::Local(Arc::new(f)));
        // Map this scale to the y coordinate channel
        self.scale_to_coord_channel.insert(name, "y".to_string());
        self
    }

    /// Add an alternative x-axis scale with a custom name
    pub fn scale_x_alt<N: Into<String>, F>(mut self, name: N, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        let name = name.into();
        self.scale_specs
            .insert(name.clone(), ScaleSpec::Local(Arc::new(f)));
        // Map this scale to the x coordinate channel
        self.scale_to_coord_channel.insert(name, "x".to_string());
        self
    }

    /// Configure an axis for a named y scale
    pub fn axis_y_alt<S: Into<String>, F>(mut self, scale_name: S, f: F) -> Self
    where
        F: Fn(A) -> A + Send + Sync + 'static,
    {
        let scale_name = scale_name.into();
        self.axis_specs
            .insert(scale_name, AxisSpec::Local(Arc::new(f)));
        self
    }

    /// Configure an axis for a named x scale
    pub fn axis_x_alt<S: Into<String>, F>(mut self, scale_name: S, f: F) -> Self
    where
        F: Fn(A) -> A + Send + Sync + 'static,
    {
        let scale_name = scale_name.into();
        self.axis_specs
            .insert(scale_name, AxisSpec::Local(Arc::new(f)));
        self
    }
}
