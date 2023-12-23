use serde::{Deserialize, Serialize};
use crate::specs::mark::MarkItemSpec;


#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolItemSpec {
    x: f32,
    y: f32,
    fill: Option<String>,
    fill_opacity: Option<f32>,
    size: Option<f32>,
    shape: Option<SymbolShape>
}

impl MarkItemSpec for SymbolItemSpec {}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
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

