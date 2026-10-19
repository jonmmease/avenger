//! The maps over raster tiles: OpenStreetMap's, or those at the URL template that
//! `--tiles <TEMPLATE>` names.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let template = args
        .iter()
        .position(|arg| arg == "--tiles")
        .and_then(|index| args.get(index + 1))
        .cloned()
        .unwrap_or_else(|| geo_pan_zoom::OSM_TILES.to_string());
    pollster::block_on(geo_pan_zoom::run(geo_pan_zoom::Basemap::Tiles(template)));
}
