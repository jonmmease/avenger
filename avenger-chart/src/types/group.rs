use std::sync::Arc;

use crate::{guides::axis::Axis, runtime::controller::Controller};

use super::mark::Mark;
use super::scales::Scale;

use crate::guides::Guide;
use crate::param::Param;
use datafusion::prelude::{DataFrame, Expr};
use indexmap::IndexMap;

#[derive(Debug, Clone)]
pub struct Group {
    pub x: f32,
    pub y: f32,
    pub size: [f32; 2],
    pub name: Option<String>,
    pub datasets: IndexMap<String, DataFrame>,
    pub scales: IndexMap<String, Scale>,
    pub params: Vec<Param>,
    pub controllers: Vec<Arc<dyn Controller>>,

    pub guides: Vec<Arc<dyn Guide>>,

    pub marks: Vec<MarkOrGroup>,

    pub title: Option<String>,
    pub subtitle: Option<String>,
}

impl Group {
    pub fn new() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            size: [200.0, 200.0],
            name: None,
            datasets: Default::default(),
            scales: Default::default(),
            params: Default::default(),
            controllers: vec![],
            guides: vec![],
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

    pub fn size(self, width: f32, height: f32) -> Self {
        Self {
            size: [width, height],
            ..self
        }
    }

    pub fn get_size(&self) -> [f32; 2] {
        self.size
    }

    pub fn guide<G: Guide>(self, guide: G) -> Self {
        let mut guides = self.guides;
        guides.push(Arc::new(guide));
        Self { guides, ..self }
    }

    pub fn get_guides(&self) -> &Vec<Arc<dyn Guide>> {
        &self.guides
    }

    /// Add an axis to the chart.
    pub fn axis(self, axis: Axis) -> Self {
        let mut guides = self.guides;
        guides.push(Arc::new(axis));
        Self { guides, ..self }
    }

    // /// Add a legend to the chart.
    // pub fn legend(self, legend: Legend) -> Self {
    //     todo!()
    //     // let mut legends = self.legends;
    //     // legends.push(legend);
    //     // Self { legends, ..self }
    // }

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

    pub fn controller(self, controller: Arc<dyn Controller>) -> Self {
        let mut controllers = self.controllers;
        controllers.push(controller);
        Self {
            controllers,
            ..self
        }
    }

    pub fn param(self, param: Param) -> Self {
        let mut params = self.params;
        params.push(param);
        Self { params, ..self }
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
