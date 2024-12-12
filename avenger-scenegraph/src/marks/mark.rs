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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}
