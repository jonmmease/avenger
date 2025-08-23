//! Example of a custom Cartesian axis implementation from an external crate
//! This demonstrates the current limitations and required workarounds

use avenger_chart::{
    axis::{Axis as AxisBase, AxisPosition},
    cartesian::CartesianAxis,
};
use std::any::Any;

/// A logarithmic axis with custom formatting options
#[derive(Clone, Debug)]
pub struct LogarithmicAxis {
    pub visible: bool,
    pub position: Option<AxisPosition>,
    pub title: Option<String>,
    pub grid: bool,
    pub tick_count: Option<usize>,
    pub label_angle: f32,
    pub format_number: Option<String>,
    // Custom fields for logarithmic axis
    pub base: f64,
    pub show_minor_ticks: bool,
    pub minor_tick_count: usize,
}

impl Default for LogarithmicAxis {
    fn default() -> Self {
        // NOTE: This is where we have to set defaults that would normally
        // be set by create_default_axes. This is a current limitation.
        Self {
            visible: true,
            position: None, // Will be set based on channel name (x -> Bottom, y -> Left)
            title: None,    // Will be extracted from mark encodings
            grid: true,     // Numeric scales default to grid=true
            tick_count: None,
            label_angle: 0.0,
            format_number: None,
            // Custom defaults
            base: 10.0,
            show_minor_ticks: false,
            minor_tick_count: 4,
        }
    }
}

impl AxisBase for LogarithmicAxis {
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

use avenger_chart::render::Padding;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_chart::error::AvengerChartError;

impl CartesianAxis for LogarithmicAxis {
    // === Getters ===
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
        self.format_number.as_deref()
    }

    // === Setters ===
    fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    fn with_position(mut self, position: AxisPosition) -> Self {
        self.position = Some(position);
        self
    }

    fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    fn with_grid(mut self, grid: bool) -> Self {
        self.grid = grid;
        self
    }

    fn with_tick_count(mut self, count: usize) -> Self {
        self.tick_count = Some(count);
        self
    }

    fn with_label_angle(mut self, angle: f32) -> Self {
        self.label_angle = angle;
        self
    }

    fn with_format_number(mut self, format: impl Into<String>) -> Self {
        self.format_number = Some(format.into());
        self
    }

    fn render(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
        plot_width: f32,
        plot_height: f32,
        padding: &Padding,
    ) -> Result<SceneMark, AvengerChartError> {
        // For this example, we'll delegate to the same rendering logic as DefaultCartesianAxis
        // In a real implementation, this could have custom rendering for logarithmic scales
        use avenger_guides::axis::{
            numeric::make_numeric_axis_marks,
            opts::{AxisConfig, AxisOrientation},
        };

        // Skip if invisible
        if !self.visible {
            return Ok(SceneMark::Group(avenger_scenegraph::marks::group::SceneGroup {
                marks: vec![],
                ..Default::default()
            }));
        }

        // Determine axis position
        let position = self.position.unwrap_or_else(|| {
            match channel {
                "x" => AxisPosition::Bottom,
                "y" => AxisPosition::Left,
                _ => AxisPosition::Bottom,
            }
        });

        // Convert position to orientation
        let orientation = match position {
            AxisPosition::Top => AxisOrientation::Top,
            AxisPosition::Bottom => AxisOrientation::Bottom,
            AxisPosition::Left => AxisOrientation::Left,
            AxisPosition::Right => AxisOrientation::Right,
        };

        let axis_origin = [padding.left, padding.top];

        let axis_config = AxisConfig {
            orientation,
            dimensions: [plot_width, plot_height],
            grid: self.grid,
            format_number: self.format_number.clone(),
            title_font_size: None,
        };

        // For logarithmic axis, always use numeric rendering
        // In a real implementation, this could generate log-spaced ticks
        let axis_group = make_numeric_axis_marks(
            scale,
            self.title.as_deref().unwrap_or(""),
            axis_origin,
            &axis_config,
        )?;

        Ok(SceneMark::Group(axis_group))
    }
}

impl LogarithmicAxis {
    /// Builder method for axis visibility
    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    /// Builder method for axis position
    pub fn position(mut self, position: AxisPosition) -> Self {
        self.position = Some(position);
        self
    }

    /// Builder method for axis title
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Builder method for grid lines
    pub fn grid(mut self, grid: bool) -> Self {
        self.grid = grid;
        self
    }

    /// Builder method for logarithm base
    pub fn base(mut self, base: f64) -> Self {
        self.base = base;
        self
    }

    /// Builder method for showing minor ticks
    pub fn show_minor_ticks(mut self, show: bool) -> Self {
        self.show_minor_ticks = show;
        self
    }

    /// Builder method for number of minor ticks
    pub fn minor_tick_count(mut self, count: usize) -> Self {
        self.minor_tick_count = count;
        self
    }

    /// Custom method to format numbers in logarithmic notation
    pub fn format_log_value(&self, value: f64) -> String {
        if self.base == 10.0 {
            // Use scientific notation for base 10
            format!("10^{:.0}", value.log10())
        } else {
            // Use base notation for other bases
            format!("{:.0}^{:.0}", self.base, value.log(self.base))
        }
    }
}

/// Temperature axis with Celsius/Fahrenheit conversion
#[derive(Clone, Debug)]
pub struct TemperatureAxis {
    pub visible: bool,
    pub position: Option<AxisPosition>,
    pub title: Option<String>,
    pub grid: bool,
    pub tick_count: Option<usize>,
    pub label_angle: f32,
    pub format_number: Option<String>,
    // Custom fields
    pub unit: TemperatureUnit,
    pub show_both_units: bool,
}

impl Default for TemperatureAxis {
    fn default() -> Self {
        Self {
            visible: true,
            position: None,
            title: None,
            grid: true,
            tick_count: None,
            label_angle: 0.0,
            format_number: None,
            unit: TemperatureUnit::Celsius,
            show_both_units: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum TemperatureUnit {
    #[default]
    Celsius,
    Fahrenheit,
    Kelvin,
}

impl AxisBase for TemperatureAxis {
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

impl CartesianAxis for TemperatureAxis {
    // === Getters ===
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
        self.format_number.as_deref()
    }

    // === Setters ===
    fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    fn with_position(mut self, position: AxisPosition) -> Self {
        self.position = Some(position);
        self
    }

    fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    fn with_grid(mut self, grid: bool) -> Self {
        self.grid = grid;
        self
    }

    fn with_tick_count(mut self, count: usize) -> Self {
        self.tick_count = Some(count);
        self
    }

    fn with_label_angle(mut self, angle: f32) -> Self {
        self.label_angle = angle;
        self
    }

    fn with_format_number(mut self, format: impl Into<String>) -> Self {
        self.format_number = Some(format.into());
        self
    }

    fn render(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
        plot_width: f32,
        plot_height: f32,
        padding: &Padding,
    ) -> Result<SceneMark, AvengerChartError> {
        use avenger_guides::axis::{
            numeric::make_numeric_axis_marks,
            opts::{AxisConfig, AxisOrientation},
        };

        // Skip if invisible
        if !self.visible {
            return Ok(SceneMark::Group(avenger_scenegraph::marks::group::SceneGroup {
                marks: vec![],
                ..Default::default()
            }));
        }

        let position = self.position.unwrap_or_else(|| {
            match channel {
                "x" => AxisPosition::Bottom,
                "y" => AxisPosition::Left,
                _ => AxisPosition::Bottom,
            }
        });

        let orientation = match position {
            AxisPosition::Top => AxisOrientation::Top,
            AxisPosition::Bottom => AxisOrientation::Bottom,
            AxisPosition::Left => AxisOrientation::Left,
            AxisPosition::Right => AxisOrientation::Right,
        };

        let axis_origin = [padding.left, padding.top];

        let axis_config = AxisConfig {
            orientation,
            dimensions: [plot_width, plot_height],
            grid: self.grid,
            format_number: self.format_number.clone(),
            title_font_size: None,
        };

        // For temperature axis, use numeric rendering
        // A real implementation could show dual scales or unit conversions
        let axis_group = make_numeric_axis_marks(
            scale,
            self.title.as_deref().unwrap_or(""),
            axis_origin,
            &axis_config,
        )?;

        Ok(SceneMark::Group(axis_group))
    }
}

impl TemperatureAxis {
    pub fn unit(mut self, unit: TemperatureUnit) -> Self {
        self.unit = unit;
        self
    }

    pub fn show_both_units(mut self, show: bool) -> Self {
        self.show_both_units = show;
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn grid(mut self, grid: bool) -> Self {
        self.grid = grid;
        self
    }

    /// Convert temperature value based on current unit
    pub fn format_temperature(&self, value: f64) -> String {
        match self.unit {
            TemperatureUnit::Celsius => {
                if self.show_both_units {
                    let fahrenheit = value * 9.0 / 5.0 + 32.0;
                    format!("{:.1}°C ({:.1}°F)", value, fahrenheit)
                } else {
                    format!("{:.1}°C", value)
                }
            }
            TemperatureUnit::Fahrenheit => {
                if self.show_both_units {
                    let celsius = (value - 32.0) * 5.0 / 9.0;
                    format!("{:.1}°F ({:.1}°C)", value, celsius)
                } else {
                    format!("{:.1}°F", value)
                }
            }
            TemperatureUnit::Kelvin => {
                if self.show_both_units {
                    let celsius = value - 273.15;
                    format!("{:.1}K ({:.1}°C)", value, celsius)
                } else {
                    format!("{:.1}K", value)
                }
            }
        }
    }
}