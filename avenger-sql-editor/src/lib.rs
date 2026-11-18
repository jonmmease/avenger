//! A lightweight editor of avenger-sql queries. CSV and Parquet files become tables of a
//! DataFusion session with the functions of avenger-sql, and the result of each query shows as a
//! chart, or as a table whose chart columns show the chart of each row.

pub mod app;
pub mod assist;
mod cards;
pub mod charts;
pub mod comment;
pub mod document;
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
pub mod sources;
mod table;
#[cfg(target_arch = "wasm32")]
mod web;

pub use app::{EditorApp, Options};
