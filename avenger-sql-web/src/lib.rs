#![doc = include_str!("../README.md")]

// The libraries of avenger-query-web and avenger-chart-web, whose exports the module holds too.
pub use avenger_query_web::*;

pub mod project;
#[cfg(target_arch = "wasm32")]
mod web;

pub use project::SqlProject;
