#![doc = include_str!("../README.md")]

pub mod condition;
pub mod properties;
pub mod selector;
pub mod source;
pub mod stylesheet;
pub mod values;
pub mod vocab;

pub use properties::{Property, Shorthand};
pub use source::{Diagnostic, Severity, SourceId, Span};
pub use stylesheet::{
    Declaration, Declared, Import, Interpolation, LayerDeclaration, LayerName, Name, PaletteKind,
    PaletteRule, ParseOptions, StyleRule, Stylesheet,
};
pub use values::{Color, Syntax, Value, Wide};
pub use vocab::Element;
