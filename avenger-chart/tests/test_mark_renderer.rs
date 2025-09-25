#[cfg(test)]
mod tests {
    use avenger_chart::prelude::*;
    use avenger_chart::cartesian::Cartesian;
    use avenger_chart::marks::symbol::Symbol;
    use std::sync::Arc;

    #[test]
    fn test_cartesian_symbol_mark_renderer() {
        // Create a simple plot with a CartesianSymbol mark
        let plot = Plot::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .x("x")
                    .y("y")
            );

        // Verify that we have marks
        assert_eq!(plot.marks().len(), 1);

        // Verify the mark was created successfully
        let mark = &plot.marks()[0];
        assert_eq!(mark.mark_type(), "symbol");

        // Test that we can build a MarkRenderer from the mark
        let renderer = mark.build();
        assert_eq!(renderer.mark_type(), "symbol");
    }
}