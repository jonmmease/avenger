//! Compile-time checks for using built-in coordinate crates directly.

use avenger_chart::plot::Plot;
use avenger_chart_marks::Symbol;
use avenger_chart_polar::{Polar, PolarSymbolPositionChannels};

#[test]
fn direct_polar_crate_imports_support_symbol_authoring() {
    let _plot = Plot::<Polar>::new().mark(Symbol::<Polar>::new().r("radius").theta("angle"));
}
