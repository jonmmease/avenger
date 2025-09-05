use crate::axis::{Axis, AxisPosition};
use crate::error::AvengerChartError;
use avenger_scenegraph::marks::mark::SceneMark;
use std::any::Any;

/// Concrete struct for Cartesian axes
/// Using a struct instead of a trait enables type inference in closure parameters
#[derive(Clone, Debug)]
pub struct CartesianAxis {
    pub visible: bool,
    pub position: Option<AxisPosition>,
    pub title: Option<String>,
    pub grid: bool,
    pub tick_count: Option<usize>,
    pub label_angle: f32,
    pub format_number: Option<String>,
}

impl CartesianAxis {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn position(mut self, position: AxisPosition) -> Self {
        self.position = Some(position);
        self
    }

    pub fn title<S: Into<String>>(mut self, title: S) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn grid(mut self, grid: bool) -> Self {
        self.grid = grid;
        self
    }

    pub fn tick_count(mut self, count: usize) -> Self {
        self.tick_count = Some(count);
        self
    }

    pub fn label_angle(mut self, angle: f32) -> Self {
        self.label_angle = angle;
        self
    }

    pub fn format(mut self, format: impl Into<String>) -> Self {
        self.format_number = Some(format.into());
        self
    }

    /// Render this axis to scene marks
    pub fn render(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
        plot_width: f32,
        plot_height: f32,
        padding: &crate::render::Padding,
    ) -> Result<SceneMark, AvengerChartError> {
        use avenger_guides::axis::{
            band::make_band_axis_marks,
            numeric::make_numeric_axis_marks,
            opts::{AxisConfig, AxisOrientation},
        };

        // Skip if invisible
        if !self.visible {
            return Ok(SceneMark::Group(
                avenger_scenegraph::marks::group::SceneGroup {
                    marks: vec![],
                    ..Default::default()
                },
            ));
        }

        // Determine axis position
        let position = self.position.unwrap_or(
            // Default positions based on channel name
            match channel {
                "x" => AxisPosition::Bottom,
                "y" => AxisPosition::Left,
                _ => AxisPosition::Bottom,
            },
        );

        // Convert position to orientation
        let orientation = match position {
            AxisPosition::Top => AxisOrientation::Top,
            AxisPosition::Bottom => AxisOrientation::Bottom,
            AxisPosition::Left => AxisOrientation::Left,
            AxisPosition::Right => AxisOrientation::Right,
        };

        // Axis origin is always the top-left corner of the plot area
        let axis_origin = [padding.left, padding.top];

        // Create axis config with plot dimensions
        let axis_config = AxisConfig {
            orientation,
            dimensions: [plot_width, plot_height],
            grid: self.grid,
            format_number: self.format_number.clone(),
            title_font_size: None, // Use default for regular axes
        };

        // Generate axis marks based on scale type
        // Check if scale has "band" option (band and point scales have this)
        let has_band_option = scale
            .scale_impl
            .option_definitions()
            .iter()
            .any(|def| def.name == "band");
        
        let axis_group = if has_band_option {
            make_band_axis_marks(
                scale,
                self.title.as_deref().unwrap_or(""),
                axis_origin,
                &axis_config,
            )?
        } else {
            // Default to numeric axis for linear and other continuous scales
            make_numeric_axis_marks(
                scale,
                self.title.as_deref().unwrap_or(""),
                axis_origin,
                &axis_config,
            )?
        };

        Ok(SceneMark::Group(axis_group))
    }
}

impl Default for CartesianAxis {
    fn default() -> Self {
        Self {
            visible: true,
            position: None,
            title: None,
            grid: false,
            tick_count: None,
            label_angle: 0.0,
            format_number: None,
        }
    }
}

impl Axis for CartesianAxis {
    fn clone_box(&self) -> Box<dyn Axis> {
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
