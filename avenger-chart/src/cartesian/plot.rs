use crate::cartesian::Cartesian;
use crate::coords::CoordinateSystem;
use crate::plot::{AxisSpec, Plot, ScaleSpec};
use crate::scales::Scale;
use std::sync::Arc;

impl Plot<Cartesian> {
    pub fn scale_x<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale) -> Scale + Send + Sync + 'static,
    {
        self.scale_specs
            .insert("x".to_string(), ScaleSpec::Local(Arc::new(f)));
        self
    }

    /// Reference a named scale from parent layout for x channel
    pub fn scale_x_ref<S: Into<String>>(mut self, name: S) -> Self {
        self.scale_specs
            .insert("x".to_string(), ScaleSpec::Reference(name.into()));
        self
    }

    pub fn scale_y<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale) -> Scale + Send + Sync + 'static,
    {
        self.scale_specs
            .insert("y".to_string(), ScaleSpec::Local(Arc::new(f)));
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
        F: Fn(<Cartesian as CoordinateSystem>::Axis) -> <Cartesian as CoordinateSystem>::Axis
            + Send
            + Sync
            + 'static,
    {
        self.axis_specs
            .insert("x".to_string(), AxisSpec::Local(Arc::new(f)));
        self
    }

    pub fn axis_y<F>(mut self, f: F) -> Self
    where
        F: Fn(<Cartesian as CoordinateSystem>::Axis) -> <Cartesian as CoordinateSystem>::Axis
            + Send
            + Sync
            + 'static,
    {
        self.axis_specs
            .insert("y".to_string(), AxisSpec::Local(Arc::new(f)));
        self
    }

    /// Add an alternative y-axis scale with a custom name
    pub fn scale_y_alt<S: Into<String>, F>(mut self, name: S, f: F) -> Self
    where
        F: Fn(Scale) -> Scale + Send + Sync + 'static,
    {
        let name = name.into();
        self.scale_specs
            .insert(name.clone(), ScaleSpec::Local(Arc::new(f)));
        // Map this scale to the y coordinate channel
        self.scale_to_coord_channel.insert(name, "y".to_string());
        self
    }

    /// Add an alternative x-axis scale with a custom name
    pub fn scale_x_alt<S: Into<String>, F>(mut self, name: S, f: F) -> Self
    where
        F: Fn(Scale) -> Scale + Send + Sync + 'static,
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
        F: Fn(<Cartesian as CoordinateSystem>::Axis) -> <Cartesian as CoordinateSystem>::Axis
            + Send
            + Sync
            + 'static,
    {
        let scale_name = scale_name.into();
        self.axis_specs
            .insert(scale_name, AxisSpec::Local(Arc::new(f)));
        self
    }

    /// Configure an axis for a named x scale
    pub fn axis_x_alt<S: Into<String>, F>(mut self, scale_name: S, f: F) -> Self
    where
        F: Fn(<Cartesian as CoordinateSystem>::Axis) -> <Cartesian as CoordinateSystem>::Axis
            + Send
            + Sync
            + 'static,
    {
        let scale_name = scale_name.into();
        self.axis_specs
            .insert(scale_name, AxisSpec::Local(Arc::new(f)));
        self
    }
}