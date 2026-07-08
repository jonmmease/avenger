#[cfg(not(target_arch = "wasm32"))]
fn main() {
    pollster::block_on(chart_app_large_scatter::run());
}

#[cfg(target_arch = "wasm32")]
fn main() {}
