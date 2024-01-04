use crate::error::VegaWgpuError;
use crate::scene::{
    group::SceneGroup,
    rect::RectMark,
    rule::RuleMark,
    symbol::SymbolMark,
    text::TextMark
};
use crate::specs::group::GroupItemSpec;
use crate::specs::mark::MarkContainerSpec;

#[derive(Debug, Clone)]
pub enum SceneMark {
    Symbol(SymbolMark),
    Rect(RectMark),
    Rule(RuleMark),
    Text(TextMark),
    Group(SceneGroup),
}

#[derive(Debug, Clone)]
pub struct SceneGraph {
    pub groups: Vec<SceneGroup>,
    pub width: f32,
    pub height: f32,
}

impl SceneGraph {
    pub fn from_spec(
        spec: &MarkContainerSpec<GroupItemSpec>,
        origin: [f32; 2],
        width: f32,
        height: f32,
    ) -> Result<Self, VegaWgpuError> {
        Ok(Self {
            groups: SceneGroup::from_spec(spec, origin)?,
            width,
            height,
        })
    }
}
