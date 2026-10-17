//! Diagnostic images and a chart example for `avenger-layout`, kept out of that crate so that its
//! tests build without a renderer.
//!
//! `tests/gallery.rs` draws one diagnostic image per concept of the layout model and compares each
//! with its baseline in `tests/baselines/`. The `chart_grid` example lays out three plots with
//! measured axes and a legend and renders the page through SVG.
