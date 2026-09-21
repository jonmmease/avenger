#![doc = include_str!("../README.md")]

mod bin;
mod data;
mod error;
mod presence;
mod spec;
mod transform;
mod validate;

pub use bin::{Bin, BinOutput, BinParams};
pub use data::{Data, DataFormat, FormatType, InlineDataset};
pub use error::SpecError;
pub use presence::MissingNullOrValue;
pub use spec::{
    Axis, BarMark, Encoding, FieldType, Mark, MarkType, Orient, PositionFieldDef, Scale, ScaleType,
    SecondaryFieldDef, SortOrder, Text, UnitSpec,
};
pub use transform::{AggregateOp, AggregateTransform, AggregatedFieldDef, BinTransform, Transform};
