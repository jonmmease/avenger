//! Integration tests for EvaluatedFacetSpec::from_compiled_plot

use avenger_chart::channel::config_traits::CoordinationScope;
use avenger_chart::facet::FacetDirection;
use avenger_chart::facet::evaluated_facet_tree::EvaluatedFacetTree;
use avenger_chart::param::Param;
use avenger_chart::prelude::*;
use datafusion::common::ScalarValue;
use datafusion::functions_aggregate::min_max::max;
use datafusion::prelude::*;

/// Helper to create a simple test DataFrame with species and region columns
async fn create_test_data(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    ctx.read_batch(
        datafusion::arrow::record_batch::RecordBatch::try_from_iter(vec![
            (
                "species",
                std::sync::Arc::new(datafusion::arrow::array::StringArray::from(vec![
                    "setosa",
                    "setosa",
                    "versicolor",
                    "versicolor",
                    "virginica",
                    "virginica",
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
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
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
        Subplot::new(Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))))
            .row(col("species")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
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
        Subplot::new(Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))))
            .column(col("region")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
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
        Subplot::new(
            Plot::<FacetRow>::new().mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))),
                )
                .row(col("species")),
            ),
        )
        .column(col("region")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
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
        Subplot::new(
            Plot::<FacetRow>::new().mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))),
                )
                .row(col("species")),
            ),
        )
        .row(col("region")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
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
///
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
                    "setosa",
                    "versicolor",
                    "versicolor",
                    "virginica",
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

fn int_scalar(v: i64) -> ScalarValue {
    ScalarValue::Int64(Some(v))
}

#[tokio::test]
async fn test_facet_wrap_default_columns_and_predicate_skip_structural_row() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx).await;

    let plot = Plot::<FacetWrap>::new().data(df.clone()).mark(
        Subplot::new(Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))))
            .wrap(col("species")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    let root = spec.root().expect("wrap root");
    assert_eq!(root.direction, FacetDirection::Row);
    assert_eq!(
        root.values().cloned().collect::<Vec<_>>(),
        vec![int_scalar(0), int_scalar(1)]
    );

    let first_row = root.child(&int_scalar(0)).expect("first wrap row");
    assert_eq!(first_row.direction, FacetDirection::Column);
    assert_eq!(
        first_row.values().cloned().collect::<Vec<_>>(),
        vec![scalar("setosa"), scalar("versicolor")]
    );

    let path = vec![int_scalar(0), scalar("setosa")];
    let predicate = spec.cell_predicate(&path, 0).expect("wrap value predicate");
    let filtered = df
        .filter(predicate)
        .expect("filter wrap value")
        .collect()
        .await
        .unwrap();
    let row_count: usize = filtered.iter().map(|batch| batch.num_rows()).sum();
    assert_eq!(row_count, 2);

    assert!(
        spec.cell_predicate(&path, 1).is_none(),
        "Level(1) sharing should own the whole wrap, not one synthetic row"
    );
}

#[tokio::test]
async fn test_facet_wrap_columns_and_order_by_aggregate() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx).await;

    let plot = Plot::<FacetWrap>::new().data(df).mark(
        Subplot::new(Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))))
            .wrap_with(col("species"), |c| {
                c.columns(1).order_by(max(col("value"))).order_desc()
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    let root = spec.root().expect("wrap root");
    assert_eq!(
        root.values().cloned().collect::<Vec<_>>(),
        vec![int_scalar(0), int_scalar(1), int_scalar(2)]
    );
    let ordered_values = root
        .values()
        .flat_map(|row| root.child(row).expect("wrap row").values().cloned())
        .collect::<Vec<_>>();
    assert_eq!(
        ordered_values,
        vec![scalar("virginica"), scalar("versicolor"), scalar("setosa")]
    );
}

#[tokio::test]
async fn test_facet_wrap_columns_accepts_param() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx).await;
    let columns = Param::new("wrap_columns", ScalarValue::Int64(Some(2)));

    let plot = Plot::<FacetWrap>::new()
        .data(df)
        .add_param(columns.clone())
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))),
            )
            .wrap_with(col("species"), |c| c.columns(columns.expr())),
        );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot_with_params(
        &compiled,
        &ctx,
        compiled.get_default_params(),
    )
    .await
    .expect("build spec");

    let root = spec.root().expect("wrap root");
    assert_eq!(
        root.values().cloned().collect::<Vec<_>>(),
        vec![int_scalar(0), int_scalar(1)]
    );
}

#[tokio::test]
async fn test_facet_wrap_columns_accepts_aggregate() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx).await;

    let plot = Plot::<FacetWrap>::new().data(df).mark(
        Subplot::new(Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))))
            .wrap_with(col("species"), |c| c.columns(max(col("value")))),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    let root = spec.root().expect("wrap root");
    assert_eq!(
        root.values().cloned().collect::<Vec<_>>(),
        vec![int_scalar(0)]
    );
}

#[tokio::test]
async fn test_facet_wrap_columns_rejects_non_aggregate_column_expr() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx).await;

    let plot = Plot::<FacetWrap>::new().data(df).mark(
        Subplot::new(Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))))
            .wrap_with(col("species"), |c| c.columns(col("value"))),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let error = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
        .await
        .expect_err("non-aggregate column expression should fail");
    assert!(
        error
            .to_string()
            .contains("constant, parameter, or aggregate expression"),
        "{error}"
    );
}

#[tokio::test]
async fn test_free_sharing_nested_row_row() {
    // With Free sharing (default), inner domain varies by parent
    let ctx = SessionContext::new();
    let df = create_varying_domain_data(&ctx).await;

    // Nested Row > Row with Free sharing (default)
    let plot = Plot::<FacetRow>::new().data(df).mark(
        Subplot::new(
            Plot::<FacetRow>::new().mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))),
                )
                .row(col("species")),
            ),
        )
        .row(col("region")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
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
        Subplot::new(
            Plot::<FacetRow>::new().mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))),
                )
                .row_with(col("species"), |c| {
                    c.with_slot_sharing(CoordinationScope::Shared)
                }),
            ),
        )
        .row(col("region")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
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
        Subplot::new(
            Plot::<FacetRow>::new().mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))),
                )
                .row_with(col("species"), |c| {
                    c.with_slot_sharing(CoordinationScope::Level(2))
                }),
            ),
        )
        .row(col("region")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    let root = spec.root().expect("should have root");

    // Outer level should have sharing = 0 (Free)
    assert_eq!(root.sharing, 0);

    // Inner level should have sharing = 2 (Level(2))
    let child = root.child(&scalar("east")).expect("child");
    assert_eq!(child.sharing, 2);
}

#[tokio::test]
async fn test_row_facet_order_by_aggregate_descending() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx).await;

    let plot = Plot::<FacetRow>::new().data(df).mark(
        Subplot::new(Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))))
            .row_with(col("species"), |c| {
                c.order_by(max(col("value"))).order_desc()
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    let root = spec.root().expect("should have root");
    let values: Vec<_> = root.values().cloned().collect();
    assert_eq!(
        values,
        vec![scalar("virginica"), scalar("versicolor"), scalar("setosa")]
    );
}

#[tokio::test]
async fn test_facet_order_by_rejects_non_aggregate_non_partition_column() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx).await;

    let plot = Plot::<FacetRow>::new().data(df).mark(
        Subplot::new(Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))))
            .row_with(col("species"), |c| c.order_by(col("region"))),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let err = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
        .await
        .expect_err("invalid facet order expression");

    assert!(matches!(
        err,
        avenger_chart::error::AvengerChartError::InvalidArgument(message)
            if message.contains("Facet order_by expression")
    ));
}

#[tokio::test]
async fn test_free_nested_facet_ordering_is_parent_scoped() {
    let ctx = SessionContext::new();
    let df = create_varying_domain_data(&ctx).await;

    let plot = Plot::<FacetRow>::new().data(df).mark(
        Subplot::new(
            Plot::<FacetRow>::new().mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))),
                )
                .row_with(col("species"), |c| {
                    c.order_by(max(col("value"))).order_desc()
                }),
            ),
        )
        .row(col("region")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    let root = spec.root().expect("should have root");
    let east_child = root.child(&scalar("east")).expect("east child");
    let west_child = root.child(&scalar("west")).expect("west child");

    assert_eq!(
        east_child.values().cloned().collect::<Vec<_>>(),
        vec![scalar("versicolor"), scalar("setosa")]
    );
    assert_eq!(
        west_child.values().cloned().collect::<Vec<_>>(),
        vec![scalar("virginica"), scalar("versicolor")]
    );
}

#[tokio::test]
async fn test_shared_nested_facet_ordering_uses_global_scope() {
    let ctx = SessionContext::new();
    let df = create_varying_domain_data(&ctx).await;

    let plot = Plot::<FacetRow>::new().data(df).mark(
        Subplot::new(
            Plot::<FacetRow>::new().mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))),
                )
                .row_with(col("species"), |c| {
                    c.order_by(max(col("value"))).order_desc().share_slots()
                }),
            ),
        )
        .row(col("region")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    let root = spec.root().expect("should have root");
    let east_child = root.child(&scalar("east")).expect("east child");
    let west_child = root.child(&scalar("west")).expect("west child");
    let expected = vec![scalar("virginica"), scalar("versicolor"), scalar("setosa")];

    assert_eq!(east_child.values().cloned().collect::<Vec<_>>(), expected);
    assert_eq!(west_child.values().cloned().collect::<Vec<_>>(), expected);
}

async fn create_three_level_ordering_data(
    ctx: &SessionContext,
) -> datafusion::dataframe::DataFrame {
    ctx.read_batch(
        datafusion::arrow::record_batch::RecordBatch::try_from_iter(vec![
            (
                "division",
                std::sync::Arc::new(datafusion::arrow::array::StringArray::from(vec![
                    "Eng", "Eng", "Eng", "Eng", "Ops", "Ops", "Ops", "Ops",
                ])) as datafusion::arrow::array::ArrayRef,
            ),
            (
                "department",
                std::sync::Arc::new(datafusion::arrow::array::StringArray::from(vec![
                    "Platform", "Platform", "Apps", "Apps", "Field", "Field", "Support", "Support",
                ])) as datafusion::arrow::array::ArrayRef,
            ),
            (
                "team",
                std::sync::Arc::new(datafusion::arrow::array::StringArray::from(vec![
                    "Alpha", "Beta", "Alpha", "Beta", "Alpha", "Beta", "Alpha", "Beta",
                ])) as datafusion::arrow::array::ArrayRef,
            ),
            (
                "value",
                std::sync::Arc::new(datafusion::arrow::array::Float64Array::from(vec![
                    10.0, 1.0, 1.0, 5.0, 1.0, 10.0, 5.0, 1.0,
                ])) as datafusion::arrow::array::ArrayRef,
            ),
        ])
        .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn test_level1_nested_facet_ordering_uses_ancestor_scope() {
    let ctx = SessionContext::new();
    let df = create_three_level_ordering_data(&ctx).await;

    let plot = Plot::<FacetColumn>::new().data(df).mark(
        Subplot::new(
            Plot::<FacetColumn>::new().mark(
                Subplot::new(
                    Plot::<FacetRow>::new().mark(
                        Subplot::new(
                            Plot::<Cartesian>::new()
                                .mark(Symbol::new().x(col("value")).y(col("value"))),
                        )
                        .row_with(col("team"), |c| {
                            c.order_by(max(col("value")))
                                .order_desc()
                                .with_slot_sharing(CoordinationScope::Level(1))
                        }),
                    ),
                )
                .col_with(col("department"), |c| c.free_slots()),
            ),
        )
        .col_with(col("division"), |c| c.free_slots()),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    let eng_platform = spec
        .node_at_path(&[scalar("Eng"), scalar("Platform")])
        .expect("Eng Platform teams");
    let eng_apps = spec
        .node_at_path(&[scalar("Eng"), scalar("Apps")])
        .expect("Eng Apps teams");
    let ops_field = spec
        .node_at_path(&[scalar("Ops"), scalar("Field")])
        .expect("Ops Field teams");
    let ops_support = spec
        .node_at_path(&[scalar("Ops"), scalar("Support")])
        .expect("Ops Support teams");

    let eng_expected = vec![scalar("Alpha"), scalar("Beta")];
    let ops_expected = vec![scalar("Beta"), scalar("Alpha")];
    assert_eq!(
        eng_platform.values().cloned().collect::<Vec<_>>(),
        eng_expected
    );
    assert_eq!(eng_apps.values().cloned().collect::<Vec<_>>(), eng_expected);
    assert_eq!(
        ops_field.values().cloned().collect::<Vec<_>>(),
        ops_expected
    );
    assert_eq!(
        ops_support.values().cloned().collect::<Vec<_>>(),
        ops_expected
    );
}

#[tokio::test]
async fn test_enumerate_values_for_facet_shared_returns_union() {
    // Production behavior: enumerate values for inner facet under shared semantics.
    let ctx = SessionContext::new();
    let df = create_varying_domain_data(&ctx).await;

    let plot = Plot::<FacetRow>::new().data(df).mark(
        Subplot::new(
            Plot::<FacetRow>::new().mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))),
                )
                .row(col("species")),
            ),
        )
        .row(col("region")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    // For the inner facet (species), shared enumeration should return union of all species.
    let east_union = spec
        .enumerate_values_for_facet(&[scalar("east")], 255)
        .expect("east values");
    let west_union = spec
        .enumerate_values_for_facet(&[scalar("west")], 255)
        .expect("west values");

    assert_eq!(east_union.len(), 3);
    assert_eq!(west_union.len(), 3);
    assert_eq!(east_union, west_union);
    assert!(east_union.contains(&scalar("setosa")));
    assert!(east_union.contains(&scalar("versicolor")));
    assert!(east_union.contains(&scalar("virginica")));
}

#[tokio::test]
async fn test_node_at_path_root_values() {
    // Production behavior: query root node values via node_at_path.
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx).await;

    let plot = Plot::<FacetRow>::new().data(df).mark(
        Subplot::new(Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))))
            .row(col("species")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");

    let root = spec.node_at_path(&[]).expect("root node");
    let domain: Vec<_> = root.values().cloned().collect();
    assert_eq!(domain.len(), 3);
    assert!(domain.contains(&scalar("setosa")));
    assert!(domain.contains(&scalar("versicolor")));
    assert!(domain.contains(&scalar("virginica")));
}

#[tokio::test]
async fn test_facet_presence_via_root_and_depth() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx).await;

    // No facets
    let plot_no_facets = Plot::<Cartesian>::new()
        .data(df.clone())
        .mark(Symbol::new().x(col("value")).y(col("value")));

    let compiled = plot_no_facets.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");
    assert!(spec.root().is_none());
    assert_eq!(spec.depth(), 0);

    // Single facet
    let plot_single = Plot::<FacetRow>::new().data(df.clone()).mark(
        Subplot::new(Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))))
            .row(col("species")),
    );

    let compiled = plot_single.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");
    assert!(spec.root().is_some());
    assert_eq!(spec.depth(), 1);

    // Nested facets
    let plot_nested = Plot::<FacetRow>::new().data(df).mark(
        Subplot::new(
            Plot::<FacetRow>::new().mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value"))),
                )
                .row(col("species")),
            ),
        )
        .row(col("region")),
    );

    let compiled = plot_nested.compile(&ctx).await.expect("compile");
    let spec = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx)
        .await
        .expect("build spec");
    assert!(spec.root().is_some());
    assert_eq!(spec.depth(), 2);
}
