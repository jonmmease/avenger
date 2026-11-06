#![doc = include_str!("../README.md")]

mod cascade;
mod computed;
pub mod condition;
mod conditions;
mod host;
pub mod palette;
pub mod properties;
pub mod selector;
pub mod source;
pub mod stylesheet;
pub mod table;
mod tree;
pub mod values;
pub mod vocab;

pub use cascade::Cascade;
pub use computed::{Computed, ComputedStyle, ComputedStyles, Paint, StyleWarning};
pub use host::{ColorScheme, Contrast, HostInputs, HostTheme, Preferences};
pub use palette::{Palette, ScaleKind};
pub use table::{Env, Loaded, Loader, Origin, Sheet, StyleTable};
pub use tree::{AttributeValue, Node, NodeId, StyleTree};

pub use properties::{Property, Shorthand};
pub use source::{Diagnostic, Severity, SourceId, Span};
pub use stylesheet::{
    Declaration, Declared, Import, Interpolation, LayerDeclaration, LayerName, Name, PaletteKind,
    PaletteRule, ParseOptions, StyleRule, Stylesheet,
};
pub use values::{Color, Syntax, Value, Wide};
pub use vocab::Element;
