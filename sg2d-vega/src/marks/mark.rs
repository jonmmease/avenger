use crate::marks::arc::VegaArcItem;
use crate::marks::area::VegaAreaItem;
use crate::marks::group::VegaGroupItem;
use crate::marks::image::VegaImageItem;
use crate::marks::line::VegaLineItem;
use crate::marks::path::VegaPathItem;
use crate::marks::rect::VegaRectItem;
use crate::marks::rule::VegaRuleItem;
use crate::marks::shape::VegaShapeItem;
use crate::marks::symbol::VegaSymbolItem;
use crate::marks::text::VegaTextItem;
use crate::marks::trail::VegaTrailItem;
use serde::{Deserialize, Serialize};

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
    role: Option<String>,
    zindex: Option<i64>,
}
