//! A globe and a Web Mercator world map of the same earthquakes, which pan and zoom on their own
//! in two `avenger_geo::PanMode`s, with the layers' legends. See the README for the controls.

mod app;
mod maps;

pub use app::run;
