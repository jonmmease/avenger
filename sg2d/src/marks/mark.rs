use crate::marks::arc::ArcMark;
use crate::marks::group::SceneGroup;
use crate::marks::rect::RectMark;
use crate::marks::rule::RuleMark;
use crate::marks::symbol::SymbolMark;
use crate::marks::text::TextMark;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SceneMark {
    Arc(ArcMark),
    Symbol(SymbolMark),
    Rect(RectMark),
    Rule(RuleMark),
    Text(Box<TextMark>),
    Group(SceneGroup),
}
