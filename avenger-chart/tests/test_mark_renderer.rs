#[cfg(test)]
mod tests {
    use avenger_chart::cartesian::Cartesian;
    use avenger_chart::marks::symbol::Symbol;
    use avenger_chart::prelude::*;
    use std::sync::Arc;

    #[test]
    fn test_cartesian_symbol_mark_renderer() {
        // Create a simple plot with a CartesianSymbol mark
        let plot = Plot::<Cartesian>::new().mark(Symbol::new().x("x").y("y"));

        // Verify that we have mark_renderers
        assert_eq!(plot.mark_renderers().len(), 1);

        // Verify the mark renderer was created successfully
        let renderer = &plot.mark_renderers()[0];
        assert_eq!(renderer.mark_type(), "symbol");
    }
}
