#[cfg(test)]
mod tests {
    use avenger_chart::cartesian::Cartesian;
    use avenger_chart::marks::symbol::Symbol;
    use avenger_chart::prelude::*;
    use datafusion::prelude::SessionContext;

    #[tokio::test]
    async fn test_cartesian_symbol_mark_renderer() {
        // Create a simple plot with a CartesianSymbol mark
        let plot = Plot::<Cartesian>::new().mark(Symbol::new().x("x").y("y"));

        // Compile the plot to get mark renderers
        let ctx = SessionContext::new();
        let compiled = plot.compile(&ctx).await.unwrap();

        // Verify that we have mark renderers
        assert_eq!(compiled.marks().len(), 1);

        // Verify the mark renderer was created successfully
        let renderer = &compiled.marks()[0];
        assert_eq!(renderer.mark_type(), "symbol");
    }
}
