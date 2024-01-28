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
