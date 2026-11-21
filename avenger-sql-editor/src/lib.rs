//! A lightweight editor of avenger-sql projects: a project of avenger-sql-project opens with its
//! files by role, each open file in a tab, and the gallery of its charts. The result of each chart
//! script shows as a chart, or as a table whose chart columns show the chart of each row. The
//! editor embeds a sample project, which it opens when it starts.

pub mod add_file;
pub mod app;
pub mod assist;
mod buffer;
mod cards;
pub mod charts;
pub mod comment;
mod editor;
mod exec;
pub mod format;
pub mod gallery;
mod gpu;
pub mod highlight;
pub mod indent;
mod input;
mod live;
pub mod query;
pub mod result;
pub mod samples;
mod sidebar;
mod table;
#[cfg(not(target_arch = "wasm32"))]
pub mod watch;
#[cfg(target_arch = "wasm32")]
mod web;
pub mod workspace;

pub use app::{EditorApp, Options};

/// The name of the native app, which names the folder of its settings and its images.
pub const APP_ID: &str = "avenger-sql editor";
