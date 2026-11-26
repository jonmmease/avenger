#![doc = include_str!("../README.md")]

// The library of avenger-chart-web, whose exports the module holds too.
pub use avenger_chart_web::*;

#[cfg(target_arch = "wasm32")]
mod graph;

#[cfg(target_arch = "wasm32")]
pub use graph::ChartGraph;
