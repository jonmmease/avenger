pub mod group;
pub mod mark;
pub mod rect;
pub mod symbol;
pub mod dims;

use crate::specs::group::GroupItemSpec;
use crate::specs::mark::MarkContainerSpec;

pub type SceneGraphSpec = MarkContainerSpec<GroupItemSpec>;
