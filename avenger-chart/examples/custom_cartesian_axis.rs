//! Example demonstrating custom axis implementation for Cartesian coordinates

use avenger_chart::{
    axis::{Axis as AxisBase, AxisPosition},
    cartesian::{Cartesian, CartesianAxis},
    marks::symbol::Symbol,
    plot::Plot,
};
use std::any::Any;

/// A custom axis that displays values in scientific notation
#[derive(Clone, Default)]
struct ScientificAxis {
    visible: bool,
    position: Option<AxisPosition>,
    title: Option<String>,
    grid: bool,
    tick_count: Option<usize>,
    label_angle: f32,
    format_number: Option<String>,
    // Custom fields
    exponent_threshold: i32,
    show_mantissa: bool,
}

impl AxisBase for ScientificAxis {
    fn clone_box(&self) -> Box<dyn AxisBase> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

impl CartesianAxis for ScientificAxis {
    fn visible(&self) -> bool {
        self.visible
    }

    fn position(&self) -> Option<AxisPosition> {
        self.position
    }

    fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    fn grid(&self) -> bool {
        self.grid
    }

    fn tick_count(&self) -> Option<usize> {
        self.tick_count
    }

    fn label_angle(&self) -> f32 {
        self.label_angle
    }

    fn format_number(&self) -> Option<&str> {
        // In a real implementation, this would format numbers in scientific notation
        self.format_number.as_deref()
    }
}

impl ScientificAxis {
    // Custom builder methods
    pub fn exponent_threshold(mut self, threshold: i32) -> Self {
        self.exponent_threshold = threshold;
        self
    }

    pub fn show_mantissa(mut self, show: bool) -> Self {
        self.show_mantissa = show;
        self
    }

    // Standard builder methods (needed because they're not in the trait)
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn grid(mut self, grid: bool) -> Self {
        self.grid = grid;
        self
    }
}

fn main() {
    // Example 1: Using default Cartesian axes
    // Type is Plot<Cartesian> which is Plot<Cartesian<DefaultCartesianAxis>>
    let _plot_default = Plot::<Cartesian>::new()
        .mark(Symbol::new().x("mass").y("energy"))
        .axis_x(|axis| axis.title("Mass (kg)").grid(true))
        .axis_y(|axis| axis.title("Energy (J)"));

    // Example 2: Using custom scientific axes
    // Need to specify the full type for custom axes
    let _plot_scientific = Plot::<Cartesian<ScientificAxis>>::new()
        .mark(Symbol::new().x("wavelength").y("frequency"))
        .axis_x(|axis| {
            axis.title("Wavelength (m)")
                .grid(true)
                .exponent_threshold(3)
                .show_mantissa(true)
        })
        .axis_y(|axis| axis.title("Frequency (Hz)").exponent_threshold(6));

    println!("Successfully created plots with default and custom axes!");
}
