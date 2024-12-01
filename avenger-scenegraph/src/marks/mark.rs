use crate::marks::arc::ArcMark;
use crate::marks::area::AreaMark;
use crate::marks::group::SceneGroup;
use crate::marks::image::ImageMark;
use crate::marks::line::LineMark;
use crate::marks::path::PathMark;
use crate::marks::rect::RectMark;
use crate::marks::rule::RuleMark;
use crate::marks::symbol::SymbolMark;
use crate::marks::text::TextMark;
use crate::marks::trail::TrailMark;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SceneMark {
    Arc(ArcMark),
    Area(AreaMark),
    Path(PathMark),
    Symbol(SymbolMark),
    Line(LineMark),
    Trail(TrailMark),
    Rect(RectMark),
    Rule(RuleMark),
    Text(Box<TextMark>),
    Image(Box<ImageMark>),
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
}
