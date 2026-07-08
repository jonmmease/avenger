#[cfg(not(target_arch = "wasm32"))]
fn main() {
    pollster::block_on(chart_app_nyc_taxi_raster::run());
}

#[cfg(target_arch = "wasm32")]
fn main() {}
