#![doc = include_str!("../README.md")]

mod manifest;
mod project;
mod source;
mod style;

pub use manifest::{
    ChartsSection, DataSection, Manifest, ManifestError, ModulesSection, Pattern, ProjectSection,
    Role, RoleConflict, StyleSection, FORMAT, MANIFEST,
};
#[cfg(not(target_arch = "wasm32"))]
pub use project::find_root;
pub use project::{ModuleFile, ProjectError, ProjectFiles};
pub use source::{FileSource, SourceError};
pub use style::ProjectStylesheet;
