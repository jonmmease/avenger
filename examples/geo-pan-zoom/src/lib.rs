//! A globe and a Web Mercator world map of the same earthquakes, which pan and zoom on their own
//! in two `avenger_geo::PanMode`s, over countries or over raster tiles that load as they come into
//! view, with the layers' legends. See the README for the controls.

mod app;
mod maps;
mod rest;

pub use app::{run, Basemap, OSM_TILES};
