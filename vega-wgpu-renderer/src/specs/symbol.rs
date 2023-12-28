use crate::specs::mark::MarkItemSpec;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolItemSpec {
    pub x: f32,
    pub y: f32,
    pub fill: Option<String>,
    pub fill_opacity: Option<f32>,
    pub size: Option<f32>,
    pub shape: Option<SymbolShape>,
}

impl MarkItemSpec for SymbolItemSpec {}

#[derive(Default, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SymbolShape {
    #[default]
    Circle,
    Square,
    Cross,
    Diamond,
    TriangleUp,
    TriangleDown,
    TriangleRight,
    TriangleLeft,
    Arrow,
    Wedge,
    Triangle,
}
