use serde::{Deserialize, Serialize};

use crate::marks::{
    arc::VegaArcItem, area::VegaAreaItem, group::VegaGroupItem, image::VegaImageItem,
    line::VegaLineItem, path::VegaPathItem, rect::VegaRectItem, rule::VegaRuleItem,
    shape::VegaShapeItem, symbol::VegaSymbolItem, text::VegaTextItem, trail::VegaTrailItem,
};

pub trait VegaMarkItem {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "marktype")]
pub enum VegaMark {
    Arc(VegaMarkContainer<VegaArcItem>),
    Area(VegaMarkContainer<VegaAreaItem>),
    Image(VegaMarkContainer<VegaImageItem>),
    Group(VegaMarkContainer<VegaGroupItem>),
    Line(VegaMarkContainer<VegaLineItem>),
    Path(VegaMarkContainer<VegaPathItem>),
    Rect(VegaMarkContainer<VegaRectItem>),
    Rule(VegaMarkContainer<VegaRuleItem>),
    Shape(VegaMarkContainer<VegaShapeItem>),
    Symbol(VegaMarkContainer<VegaSymbolItem>),
    Text(VegaMarkContainer<VegaTextItem>),
    Trail(VegaMarkContainer<VegaTrailItem>),
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VegaMarkContainer<T: VegaMarkItem> {
    #[serde(default)]
    pub clip: bool,
    pub interactive: bool,
    #[serde(default)]
    pub items: Vec<T>,
    pub name: Option<String>,
    pub role: Option<String>,
    pub zindex: Option<i32>,
}
