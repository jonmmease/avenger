
use avenger_chart::prelude::*;
use datafusion::prelude::*;
use avenger_chart::facet::marks::facet_evaluation::evaluate_facet;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting nested facet reproduction...");
    let ctx = SessionContext::new();
    
    // Create some dummy data
    let df = ctx.read_csv("tests/data/iris.csv", CsvReadOptions::new()).await?;

    // Define nested facet chart
    // Col facet (Species) -> Row facet (Petal Width) -> Point mark
    let chart = Facet::new()
        .col_with(col("species"), |c| c.facet(|f| f.title("Species")))
        .subplot(
            Plot::<FacetRow>::new()
                .mark(
                    Facet::new()
                        .row_with(col("petal_width"), |c| {
                            c.facet(|f| f.title("Petal Width"))
                        })
                        .subplot(
                            Plot::new().mark(Mark::Point(PointMark::default()))
                        )
                )
        );

    // This is a high level API construction.
    // I need to compile it to get `CompiledPlot`.
    // But `avenger_chart` doesn't expose compilation easily in public API?
    // It's usually `chart.build(&ctx).await`.
    
    // Let's try to build/render it which should trigger the recursion.
    // We can't easily hook into the internal functions from an example.
    // So I will rely on the existing test `tests/visual_tests/test_nested_facets.rs` 
    // and run it with cargo test while adding print statements in the library code.
    
    Ok(())
}
