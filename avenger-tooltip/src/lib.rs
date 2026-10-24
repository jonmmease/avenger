//! Tooltips: a layer that describes the data under the pointer, from the rows behind the marks of
//! the layers below it.

mod content;
mod draw;
mod error;
mod target;
mod tooltips;

pub use content::TooltipField;
pub use draw::TooltipStyle;
pub use target::{InstanceTarget, TooltipAnchor};
pub use tooltips::Tooltips;
