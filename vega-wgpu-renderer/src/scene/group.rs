use crate::error::VegaWgpuError;
use crate::scene::{
    rect::RectMark,
    rule::RuleMark,
    scene_graph::SceneMark,
    symbol::SymbolMark,
    text::TextMark,
};
use crate::specs::group::GroupItemSpec;
use crate::specs::mark::{MarkContainerSpec, MarkSpec};

#[derive(Debug, Clone, Copy)]
pub struct GroupBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone)]
pub struct SceneGroup {
    pub bounds: GroupBounds,
    pub marks: Vec<SceneMark>,
}

impl SceneGroup {
    pub fn from_spec(
        spec: &MarkContainerSpec<GroupItemSpec>,
        origin: [f32; 2],
    ) -> Result<Vec<Self>, VegaWgpuError> {
        let mut scene_groups: Vec<Self> = Vec::new();
        for group_item_spec in &spec.items {
            let new_origin = [group_item_spec.x + origin[0], group_item_spec.y + origin[1]];

            let mut group_marks: Vec<SceneMark> = Vec::new();
            for item in &group_item_spec.items {
                let item_marks: Vec<_> = match item {
                    MarkSpec::Group(group) => SceneGroup::from_spec(group, new_origin)?
                        .into_iter()
                        .map(SceneMark::Group)
                        .collect(),
                    MarkSpec::Rect(mark) => {
                        vec![SceneMark::Rect(RectMark::from_spec(mark, new_origin)?)]
                    }
                    MarkSpec::Rule(mark) => {
                        vec![SceneMark::Rule(RuleMark::from_spec(mark, new_origin)?)]
                    }
                    MarkSpec::Symbol(mark) => {
                        vec![SceneMark::Symbol(SymbolMark::from_spec(mark, new_origin)?)]
                    }
                    MarkSpec::Text(mark) => {
                        vec![SceneMark::Text(TextMark::from_spec(mark, new_origin)?)]
                    }
                    _ => {
                        println!("Mark type not yet supported: {:?}", item);
                        continue;
                    }
                };
                group_marks.extend(item_marks);
            }
            scene_groups.push(Self {
                bounds: GroupBounds {
                    x: group_item_spec.x,
                    y: group_item_spec.y,
                    width: group_item_spec.width.unwrap_or(0.0), // What should happen here?
                    height: group_item_spec.height.unwrap_or(0.0), // What should happen here?
                },
                marks: group_marks,
            })
        }
        Ok(scene_groups)
    }
}
