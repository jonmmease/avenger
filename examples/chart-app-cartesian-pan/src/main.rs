#[cfg(not(target_arch = "wasm32"))]
fn main() {
    pollster::block_on(chart_app_cartesian_pan::run());
}

#[cfg(target_arch = "wasm32")]
fn main() {}
