use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::marks::{
    arc::SceneArcMark, area::SceneAreaMark, group::SceneGroup, image::SceneImageMark,
    line::SceneLineMark, mesh_image::SceneMeshImageMark, path::ScenePathMark, rect::SceneRectMark,
    rule::SceneRuleMark, symbol::SceneSymbolMark, text::SceneTextMark, trail::SceneTrailMark,
};

#[derive(Debug, Clone, PartialEq, Hash, Serialize, Deserialize)]
pub enum SceneMark {
    Arc(SceneArcMark),
    Area(SceneAreaMark),
    Path(ScenePathMark),
    Symbol(SceneSymbolMark),
    Line(SceneLineMark),
    Trail(SceneTrailMark),
    Rect(SceneRectMark),
    Rule(SceneRuleMark),
    Text(Arc<SceneTextMark>),
    Image(Arc<SceneImageMark>),
    MeshImage(Arc<SceneMeshImageMark>),
    Group(SceneGroup),
}

impl SceneMark {
    pub fn zindex(&self) -> Option<i32> {
        match self {
            Self::Arc(mark) => mark.zindex,
            Self::Area(mark) => mark.zindex,
            Self::Path(mark) => mark.zindex,
            Self::Symbol(mark) => mark.zindex,
            Self::Line(mark) => mark.zindex,
            Self::Trail(mark) => mark.zindex,
            Self::Rect(mark) => mark.zindex,
            Self::Rule(mark) => mark.zindex,
            Self::Text(mark) => mark.zindex,
            Self::Image(mark) => mark.zindex,
            Self::MeshImage(mark) => mark.zindex,
            Self::Group(mark) => mark.zindex,
        }
    }

    /// The mark's own interactive setting. `None` takes the enclosing group's.
    pub fn interactive(&self) -> Option<bool> {
        match self {
            Self::Arc(mark) => mark.interactive,
            Self::Area(mark) => mark.interactive,
            Self::Path(mark) => mark.interactive,
            Self::Symbol(mark) => mark.interactive,
            Self::Line(mark) => mark.interactive,
            Self::Trail(mark) => mark.interactive,
            Self::Rect(mark) => mark.interactive,
            Self::Rule(mark) => mark.interactive,
            Self::Text(mark) => mark.interactive,
            Self::Image(mark) => mark.interactive,
            Self::MeshImage(mark) => mark.interactive,
            Self::Group(mark) => mark.interactive,
        }
    }

    /// Sets whether hits find the mark, overriding the enclosing group's setting.
    pub fn set_interactive(&mut self, interactive: bool) {
        match self {
            Self::Arc(mark) => mark.interactive = Some(interactive),
            Self::Area(mark) => mark.interactive = Some(interactive),
            Self::Path(mark) => mark.interactive = Some(interactive),
            Self::Symbol(mark) => mark.interactive = Some(interactive),
            Self::Line(mark) => mark.interactive = Some(interactive),
            Self::Trail(mark) => mark.interactive = Some(interactive),
            Self::Rect(mark) => mark.interactive = Some(interactive),
            Self::Rule(mark) => mark.interactive = Some(interactive),
            Self::Text(mark) => Arc::make_mut(mark).interactive = Some(interactive),
            Self::Image(mark) => Arc::make_mut(mark).interactive = Some(interactive),
            Self::MeshImage(mark) => Arc::make_mut(mark).interactive = Some(interactive),
            Self::Group(mark) => mark.interactive = Some(interactive),
        }
    }

    pub fn with_interactive(mut self, interactive: bool) -> Self {
        self.set_interactive(interactive);
        self
    }

    pub fn children(&self) -> &[SceneMark] {
        match self {
            Self::Group(mark) => &mark.marks,
            _ => &[],
        }
    }

    pub fn mark_type(&self) -> SceneMarkType {
        match self {
            Self::Arc(..) => SceneMarkType::Arc,
            Self::Area(..) => SceneMarkType::Area,
            Self::Path(..) => SceneMarkType::Path,
            Self::Symbol(..) => SceneMarkType::Symbol,
            Self::Line(..) => SceneMarkType::Line,
            Self::Trail(..) => SceneMarkType::Trail,
            Self::Rect(..) => SceneMarkType::Rect,
            Self::Rule(..) => SceneMarkType::Rule,
            Self::Text(..) => SceneMarkType::Text,
            Self::Image(..) => SceneMarkType::Image,
            Self::MeshImage(..) => SceneMarkType::MeshImage,
            Self::Group(..) => SceneMarkType::Group,
        }
    }

    pub fn mark_name(&self) -> &str {
        match self {
            Self::Text(mark) => &mark.name,
            Self::Arc(mark) => &mark.name,
            Self::Area(mark) => &mark.name,
            Self::Path(mark) => &mark.name,
            Self::Symbol(mark) => &mark.name,
            Self::Line(mark) => &mark.name,
            Self::Trail(mark) => &mark.name,
            Self::Rect(mark) => &mark.name,
            Self::Rule(mark) => &mark.name,
            Self::Image(mark) => &mark.name,
            Self::MeshImage(mark) => &mark.name,
            Self::Group(mark) => &mark.name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Hash)]
pub enum SceneMarkType {
    Arc,
    Area,
    Path,
    Symbol,
    Line,
    Trail,
    Rect,
    Rule,
    Text,
    Image,
    MeshImage,
    Group,
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct MarkInstance {
    pub name: String,
    pub mark_path: Vec<usize>,
    pub instance_index: Option<usize>,
}
