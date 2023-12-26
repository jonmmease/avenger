use crate::scene::rect::RectMark;
use crate::scene::symbol::SymbolMark;
use crate::specs::group::GroupItemSpec;
use crate::specs::mark::{MarkContainerSpec, MarkSpec};
use crate::specs::rect::RectItemSpec;
use crate::specs::symbol::SymbolItemSpec;

pub trait SceneVisitor {
    type Error;
    fn visit_group(&mut self, group: &SceneGroup, bounds: GroupBounds) -> Result<(), Self::Error>;

    fn visit_symbol_mark(&mut self, mark: &SymbolMark, bounds: GroupBounds) -> Result<(), Self::Error>;

    fn visit_rect_mark(&mut self, mark: &RectMark, bounds: GroupBounds) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone)]
pub struct SceneGraph {
    pub(crate) groups: Vec<SceneGroup>,
    pub(crate) origin: [f32; 2],
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl SceneGraph {
    pub fn walk<E>(&self, visitor: &mut dyn SceneVisitor<Error=E>) -> Result<(), E> {
        for group in &self.groups {
            visitor.visit_group(
                group,
                GroupBounds {
                    x: 0.0,
                    y: 0.0,
                    width: self.width,
                    height: self.height,
                }
            )?
        }
        Ok(())
    }

    pub fn from_spec(spec: &MarkContainerSpec<GroupItemSpec>, origin: [f32; 2], width: f32, height: f32) -> Self {
        Self {
            groups: SceneGroup::from_spec(spec),
            origin,
            width,
            height,
        }
    }
}


#[derive(Debug, Clone)]
pub struct SceneGroup {
    pub bounds: GroupBounds,
    pub marks: Vec<SceneMark>,
}

impl SceneGroup {
    pub fn walk<E>(&self, visitor: &mut dyn SceneVisitor<Error=E>) -> Result<(), E> {
        for mark in &self.marks {
            match mark {
                SceneMark::Symbol(mark) => visitor.visit_symbol_mark(mark, self.bounds)?,
                SceneMark::Rect(mark) => visitor.visit_rect_mark(mark, self.bounds)?,
                SceneMark::Group(group) => visitor.visit_group(group, self.bounds)?,
            }
        }
        Ok(())
    }

    pub fn from_spec(spec: &MarkContainerSpec<GroupItemSpec>) -> Vec<Self> {
        let mut scene_groups: Vec<Self> = Vec::new();
        for group_item_spec in &spec.items {
            let mut group_marks: Vec<SceneMark> = Vec::new();
            for item in &group_item_spec.items {
                let item_marks: Vec<_> = match item {
                    MarkSpec::Group(group) => {
                        SceneGroup::from_spec(group).into_iter().map(SceneMark::Group).collect()
                    }
                    MarkSpec::Rect(mark) => {
                        vec![SceneMark::Rect(RectMark::from_spec(mark))]
                    }
                    MarkSpec::Symbol(mark) => {
                        vec![SceneMark::Symbol(SymbolMark::from_spec(mark))]
                    }
                    _ => unimplemented!()
                };
                group_marks.extend(item_marks);
            }
            scene_groups.push(Self {
                bounds: GroupBounds {
                    x: group_item_spec.x,
                    y: group_item_spec.y,
                    width: group_item_spec.width.unwrap_or(0.0),  // What should happen here?
                    height: group_item_spec.height.unwrap_or(0.0),  // What should happen here?
                },
                marks: group_marks,
            })
        }
        scene_groups
    }
}

#[derive(Debug, Clone)]
pub enum SceneMark {
    Symbol(SymbolMark),
    Rect(RectMark),
    Group(SceneGroup)
}


#[derive(Debug, Clone, Copy)]
pub struct GroupBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32
}
