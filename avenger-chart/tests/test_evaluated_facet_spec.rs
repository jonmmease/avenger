//! Integration tests for EvaluatedFacetSpec::from_compiled_plot

use avenger_chart::channel::config_traits::ScaleSharing;
use avenger_chart::facet::computed_facet_spec::EvaluatedFacetSpec;
use avenger_chart::guide::FacetDirection;
use avenger_chart::prelude::*;
use datafusion::common::ScalarValue;
use datafusion::prelude::*;

/// Helper to create a simple test DataFrame with species and region columns
async fn create_test_data(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    ctx.read_batch(
        datafusion::arrow::record_batch::RecordBatch::try_from_iter(vec![
            (
                "species",
                std::sync::Arc::new(datafusion::arrow::array::StringArray::from(vec![
                    "setosa", "setosa", "versicolor", "versicolor", "virginica", "virginica",
                ])) as datafusion::arrow::array::ArrayRef,
            ),
            (
                "region",
                std::sync::Arc::new(datafusion::arrow::array::StringArray::from(vec![
                    "east", "west", "east", "west", "east", "west",
                ])) as datafusion::arrow::array::ArrayRef,
            ),
            (
                "value",
                std::sync::Arc::new(datafusion::arrow::array::Float64Array::from(vec![
                    1.0, 2.0, 3.0, 4.0, 5.0, 6.0,
                ])) as datafusion::arrow::array::ArrayRef,
            ),
        ])
        .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn test_from_compiled_plot_no_facets() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx).await;

    // Simple plot without faceting
    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(Symbol::new().x(col("value")).y(col("value")));

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetSpec::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    assert!(spec.root().is_none());
    assert_eq!(spec.depth(), 0);
}

#[tokio::test]
async fn test_from_compiled_plot_single_row_facet() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx).await;

    // Single-level FacetRow by species
    let plot = Plot::<FacetRow>::new().data(df).mark(
        Facet::new().row(col("species")).subplot(
            Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))),
        ),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetSpec::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    // Should have a root node
    let root = spec.root().expect("should have root");

    // Check structure
    assert_eq!(root.direction, FacetDirection::Row);
    assert_eq!(root.field, "species");
    assert!(root.is_leaf()); // No nested facets

    // Check values (should be sorted)
    let values: Vec<_> = root.values().collect();
    assert_eq!(values.len(), 3); // setosa, versicolor, virginica

    assert_eq!(spec.depth(), 1);
}

#[tokio::test]
async fn test_from_compiled_plot_single_col_facet() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx).await;

    // Single-level FacetColumn by region
    let plot = Plot::<FacetColumn>::new().data(df).mark(
        Facet::new().column(col("region")).subplot(
            Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))),
        ),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetSpec::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    let root = spec.root().expect("should have root");

    assert_eq!(root.direction, FacetDirection::Column);
    assert_eq!(root.field, "region");
    assert!(root.is_leaf());

    let values: Vec<_> = root.values().collect();
    assert_eq!(values.len(), 2); // east, west

    assert_eq!(spec.depth(), 1);
}

#[tokio::test]
async fn test_from_compiled_plot_nested_col_row() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx).await;

    // Nested: FacetColumn (region) > FacetRow (species)
    let plot = Plot::<FacetColumn>::new().data(df).mark(
        Facet::new().column(col("region")).subplot(
            Plot::<FacetRow>::new().mark(
                Facet::new().row(col("species")).subplot(
                    Plot::<Cartesian>::new()
                        .mark(Symbol::new().x(col("value")).y(col("value"))),
                ),
            ),
        ),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetSpec::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    // Should have depth 2
    assert_eq!(spec.depth(), 2);

    let root = spec.root().expect("should have root");

    // Outer level: Column by region
    assert_eq!(root.direction, FacetDirection::Column);
    assert_eq!(root.field, "region");
    assert!(!root.is_leaf()); // Has children

    let outer_values: Vec<_> = root.values().collect();
    assert_eq!(outer_values.len(), 2); // east, west

    // Check children (inner level: Row by species)
    for value in root.values() {
        let child = root.child(value).expect("should have child");
        assert_eq!(child.direction, FacetDirection::Row);
        assert_eq!(child.field, "species");
        assert!(child.is_leaf()); // Innermost level

        let inner_values: Vec<_> = child.values().collect();
        assert_eq!(inner_values.len(), 3); // setosa, versicolor, virginica
    }
}

#[tokio::test]
async fn test_from_compiled_plot_nested_row_row() {
    // Test same-type nesting: Row > Row
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx).await;

    // Nested: FacetRow (region) > FacetRow (species)
    let plot = Plot::<FacetRow>::new().data(df).mark(
        Facet::new().row(col("region")).subplot(
            Plot::<FacetRow>::new().mark(
                Facet::new().row(col("species")).subplot(
                    Plot::<Cartesian>::new()
                        .mark(Symbol::new().x(col("value")).y(col("value"))),
                ),
            ),
        ),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetSpec::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    assert_eq!(spec.depth(), 2);

    let root = spec.root().expect("should have root");

    // Both levels are Row
    assert_eq!(root.direction, FacetDirection::Row);
    assert_eq!(root.field, "region");

    for value in root.values() {
        let child = root.child(value).expect("should have child");
        assert_eq!(child.direction, FacetDirection::Row);
        assert_eq!(child.field, "species");
    }
}

/// Create test data where inner domain varies by parent
/// - region "east" has species: setosa, versicolor
/// - region "west" has species: versicolor, virginica
/// With Free sharing, each child gets different domain
/// With Shared sharing, all children get [setosa, versicolor, virginica]
async fn create_varying_domain_data(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    ctx.read_batch(
        datafusion::arrow::record_batch::RecordBatch::try_from_iter(vec![
            (
                "region",
                std::sync::Arc::new(datafusion::arrow::array::StringArray::from(vec![
                    "east", "east", "west", "west",
                ])) as datafusion::arrow::array::ArrayRef,
            ),
            (
                "species",
                std::sync::Arc::new(datafusion::arrow::array::StringArray::from(vec![
                    "setosa", "versicolor", "versicolor", "virginica",
                ])) as datafusion::arrow::array::ArrayRef,
            ),
            (
                "value",
                std::sync::Arc::new(datafusion::arrow::array::Float64Array::from(vec![
                    1.0, 2.0, 3.0, 4.0,
                ])) as datafusion::arrow::array::ArrayRef,
            ),
        ])
        .unwrap(),
    )
    .unwrap()
}

fn scalar(s: &str) -> ScalarValue {
    ScalarValue::Utf8(Some(s.to_string()))
}

#[tokio::test]
async fn test_free_sharing_nested_row_row() {
    // With Free sharing (default), inner domain varies by parent
    let ctx = SessionContext::new();
    let df = create_varying_domain_data(&ctx).await;

    // Nested Row > Row with Free sharing (default)
    let plot = Plot::<FacetRow>::new().data(df).mark(
        Facet::new().row(col("region")).subplot(
            Plot::<FacetRow>::new().mark(
                Facet::new().row(col("species")).subplot(
                    Plot::<Cartesian>::new()
                        .mark(Symbol::new().x(col("value")).y(col("value"))),
                ),
            ),
        ),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetSpec::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    let root = spec.root().expect("should have root");

    // Check that children have DIFFERENT domains (Free sharing)
    let east_child = root.child(&scalar("east")).expect("east child");
    let west_child = root.child(&scalar("west")).expect("west child");

    let east_values: Vec<_> = east_child.values().cloned().collect();
    let west_values: Vec<_> = west_child.values().cloned().collect();

    // east has setosa, versicolor
    assert_eq!(east_values.len(), 2);
    assert!(east_values.contains(&scalar("setosa")));
    assert!(east_values.contains(&scalar("versicolor")));

    // west has versicolor, virginica
    assert_eq!(west_values.len(), 2);
    assert!(west_values.contains(&scalar("versicolor")));
    assert!(west_values.contains(&scalar("virginica")));

    // They're different!
    assert_ne!(east_values, west_values);
}

#[tokio::test]
async fn test_shared_sharing_nested_row_row() {
    // With Shared sharing, inner domain is the same for all parents
    let ctx = SessionContext::new();
    let df = create_varying_domain_data(&ctx).await;

    // Nested Row > Row with Shared sharing on inner facet
    let plot = Plot::<FacetRow>::new().data(df).mark(
        Facet::new().row(col("region")).subplot(
            Plot::<FacetRow>::new().mark(
                Facet::new()
                    .row_with(col("species"), |c| {
                        c.facet(|f| f.with_scale_sharing(ScaleSharing::Shared))
                    })
                    .subplot(
                        Plot::<Cartesian>::new()
                            .mark(Symbol::new().x(col("value")).y(col("value"))),
                    ),
            ),
        ),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetSpec::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    let root = spec.root().expect("should have root");

    // Check that children have THE SAME domain (Shared sharing)
    let east_child = root.child(&scalar("east")).expect("east child");
    let west_child = root.child(&scalar("west")).expect("west child");

    let east_values: Vec<_> = east_child.values().cloned().collect();
    let west_values: Vec<_> = west_child.values().cloned().collect();

    // Both should have all three species
    assert_eq!(east_values.len(), 3);
    assert_eq!(west_values.len(), 3);

    // They should be the same!
    assert_eq!(east_values, west_values);

    // And contain all species
    assert!(east_values.contains(&scalar("setosa")));
    assert!(east_values.contains(&scalar("versicolor")));
    assert!(east_values.contains(&scalar("virginica")));
}

#[tokio::test]
async fn test_sharing_level_stored_in_node() {
    // Verify that the sharing level is correctly stored in the partition node
    let ctx = SessionContext::new();
    let df = create_varying_domain_data(&ctx).await;

    // Outer: Free (default), Inner: Level(2)
    let plot = Plot::<FacetRow>::new().data(df).mark(
        Facet::new().row(col("region")).subplot(
            Plot::<FacetRow>::new().mark(
                Facet::new()
                    .row_with(col("species"), |c| {
                        c.facet(|f| f.with_scale_sharing(ScaleSharing::Level(2)))
                    })
                    .subplot(
                        Plot::<Cartesian>::new()
                            .mark(Symbol::new().x(col("value")).y(col("value"))),
                    ),
            ),
        ),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetSpec::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    let root = spec.root().expect("should have root");

    // Outer level should have sharing = 0 (Free)
    assert_eq!(root.sharing, 0);

    // Inner level should have sharing = 2 (Level(2))
    let child = root.child(&scalar("east")).expect("child");
    assert_eq!(child.sharing, 2);
}
