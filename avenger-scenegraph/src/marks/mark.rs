use std::sync::Arc;

use crate::marks::arc::SceneArcMark;
use crate::marks::area::SceneAreaMark;
use crate::marks::group::SceneGroup;
use crate::marks::image::SceneImageMark;
use crate::marks::line::SceneLineMark;
use crate::marks::path::ScenePathMark;
use crate::marks::rect::SceneRectMark;
use crate::marks::rule::SceneRuleMark;
use crate::marks::symbol::SceneSymbolMark;
use crate::marks::text::SceneTextMark;
use crate::marks::trail::SceneTrailMark;
use serde::{Deserialize, Serialize};

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
            Self::Group(mark) => mark.zindex,
        }
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
            Self::Group(mark) => &mark.name,
        }
    }
}

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
    Group,
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct MarkInstance {
    pub name: String,
    pub mark_path: Vec<usize>,
    pub instance_index: Option<usize>,
}
