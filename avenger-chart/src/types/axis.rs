use avenger_guides::axis::{
    numeric::make_numeric_axis_marks,
    opts::{AxisConfig, AxisOrientation},
};
use avenger_scenegraph::marks::mark::SceneMark;

use crate::error::AvengerChartError;

use super::{
    guide::{Guide, GuideCompilationContext},
    scales::Scale,
};

#[derive(Debug, Clone)]
pub struct Axis {
    pub scale: Vec<Scale>,
    pub orientation: AxisOrientation,
    pub title: String,
    pub ticks: Option<bool>,
    pub grid: Option<bool>,
}

impl Axis {
    pub fn new(scale: &Scale) -> Self {
        Self {
            scale: vec![scale.clone()],
            orientation: AxisOrientation::Left,
            title: "".to_string(),
            ticks: None,
            grid: None,
        }
    }

    pub fn ticks(self, ticks: bool) -> Self {
        Self {
            ticks: Some(ticks),
            ..self
        }
    }

    pub fn get_ticks(&self) -> Option<bool> {
        self.ticks
    }

    pub fn title(self, title: String) -> Self {
        Self { title, ..self }
    }

    pub fn get_title(&self) -> &str {
        self.title.as_str()
    }

    pub fn orientation(self, orientation: AxisOrientation) -> Self {
        Self {
            orientation,
            ..self
        }
    }

    pub fn get_orientation(&self) -> AxisOrientation {
        self.orientation
    }

    pub fn grid(self, grid: bool) -> Self {
        Self {
            grid: Some(grid),
            ..self
        }
    }

    pub fn get_grid(&self) -> Option<bool> {
        self.grid
    }
}

impl Guide for Axis {
    fn scales(&self) -> &[Scale] {
        &self.scale
    }

    fn compile(
        &self,
        context: &GuideCompilationContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let configured_scale = context.scales.get(0).unwrap();

        let axis_config = AxisConfig {
            orientation: self.orientation,
            dimensions: context.size,
            grid: self.get_grid().unwrap_or(false),
        };

        let marks = SceneMark::Group(make_numeric_axis_marks(
            configured_scale,
            self.get_title(),
            context.origin,
            &axis_config,
        )?);

        Ok(vec![marks])
    }
}
