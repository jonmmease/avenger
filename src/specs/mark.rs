use crate::specs::group::GroupItemSpec;
use crate::specs::rect::RectItemSpec;
use crate::specs::symbol::SymbolItemSpec;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

pub trait MarkItemSpec {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "marktype")]
pub enum MarkSpec {
    Arc,
    Area,
    Image,
    Group(MarkContainerSpec<GroupItemSpec>),
    Line,
    Path,
    Rect(MarkContainerSpec<RectItemSpec>),
    Rule,
    Shape,
    Symbol(MarkContainerSpec<SymbolItemSpec>),
    Text,
    Trail,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarkContainerSpec<T: MarkItemSpec> {
    #[serde(default)]
    pub clip: bool,
    interactive: bool,
    #[serde(default)]
    pub items: Vec<T>,
    name: Option<String>,
    role: Option<String>,
    zindex: Option<i64>,
}
