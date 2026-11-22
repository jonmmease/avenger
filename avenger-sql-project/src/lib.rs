#![doc = include_str!("../README.md")]

pub mod add;
mod live;
mod load;
mod manifest;
mod path;
mod project;
mod source;
mod style;
#[cfg(not(target_arch = "wasm32"))]
pub mod watch;

pub use live::{charts_to_run, Analysis, LoadState, Loader, ScriptState, Status, Table};
pub use load::{
    Change, ChartError, FileReaders, FilesReload, LoadedProject, LoadedScript, ProjectDiagnostic,
    ScriptLoad, ScriptStatus, TableChange, UnloadedRead,
};
pub use manifest::{
    ChartsSection, DataSection, Manifest, ManifestError, ModulesSection, Pattern, ProjectSection,
    Role, RoleConflict, StyleSection, FORMAT, MANIFEST,
};
#[cfg(not(target_arch = "wasm32"))]
pub use project::find_root;
pub use project::{ModuleConflict, ModuleFile, ProjectError, ProjectFiles};
pub use source::{FileSource, Listing, Skipped, SourceError};
pub use style::ProjectStylesheet;
