use super::axis::Axis;
use super::legend::Legend;
use super::mark::Mark;
use super::scales::Scale;

use datafusion::prelude::{DataFrame, Expr};
use indexmap::IndexMap;
#[derive(Debug, Clone)]
pub struct Group {
    pub x: f32,
    pub y: f32,
    pub name: Option<String>,
    pub datasets: IndexMap<String, DataFrame>,
    pub scales: IndexMap<String, Scale>,
    pub params: IndexMap<String, Expr>,

    pub axes: Vec<Axis>,
    pub legends: Vec<Legend>,
    pub marks: Vec<MarkOrGroup>,

    pub title: Option<String>,
    pub subtitle: Option<String>,
}

impl Group {
    pub fn new() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            name: None,
            datasets: Default::default(),
            scales: Default::default(),
            params: Default::default(),
            axes: vec![],
            legends: vec![],
            marks: vec![],
            title: None,
            subtitle: None,
        }
    }

    pub fn name(self, name: String) -> Self {
        Self {
            name: Some(name),
            ..self
        }
    }

    pub fn get_name(&self) -> Option<&String> {
        self.name.as_ref()
    }

    pub fn x(self, x: f32) -> Self {
        Self { x, ..self }
    }

    pub fn get_x(&self) -> f32 {
        self.x
    }

    pub fn y(self, y: f32) -> Self {
        Self { y, ..self }
    }

    pub fn get_y(&self) -> f32 {
        self.y
    }

    /// Add an axis to the chart.
    pub fn axis(self, axis: Axis) -> Self {
        let mut axes = self.axes;
        axes.push(axis);
        Self { axes, ..self }
    }

    /// Get the axes of the chart.
    pub fn get_axes(&self) -> &Vec<Axis> {
        &self.axes
    }

    /// Add a legend to the chart.
    pub fn legend(self, legend: Legend) -> Self {
        let mut legends = self.legends;
        legends.push(legend);
        Self { legends, ..self }
    }

    /// Get the legends of the chart.
    pub fn get_legends(&self) -> &Vec<Legend> {
        &self.legends
    }

    /// Add a mark to the chart.
    pub fn mark(self, mark: Mark) -> Self {
        let mut marks = self.marks;
        marks.push(MarkOrGroup::Mark(mark));
        Self { marks, ..self }
    }

    /// Add a group to the chart.
    pub fn group(self, group: Group) -> Self {
        let mut marks = self.marks;
        marks.push(MarkOrGroup::Group(group));
        Self { marks, ..self }
    }

    /// Get the marks of the chart.
    pub fn get_marks_and_groups(&self) -> &Vec<MarkOrGroup> {
        &self.marks
    }

    /// Set the title of the chart.
    pub fn title(self, title: String) -> Self {
        Self {
            title: Some(title),
            ..self
        }
    }

    /// Get the title of the chart.
    pub fn get_title(&self) -> Option<&String> {
        self.title.as_ref()
    }

    /// Set the subtitle of the chart.
    pub fn subtitle(self, subtitle: String) -> Self {
        Self {
            subtitle: Some(subtitle),
            ..self
        }
    }

    /// Get the subtitle of the chart.
    pub fn get_subtitle(&self) -> Option<&String> {
        self.subtitle.as_ref()
    }
}

#[derive(Debug, Clone)]
pub enum MarkOrGroup {
    Mark(Mark),
    Group(Group),
}
