//! The maps over countries.

fn main() {
    pollster::block_on(geo_pan_zoom::run(geo_pan_zoom::Basemap::Countries));
}
