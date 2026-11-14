//! The tables and modules that a script names, and the catalogs that look up its tables.

use std::sync::Arc;

use avenger_sql::catalog::{references, Catalog, MemoryCatalog, SessionCatalog};
use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    catalog::{MemorySchemaProvider, SchemaProvider},
    datasource::empty::EmptyTable,
    prelude::SessionContext,
};

fn names(path: &str) -> Vec<String> {
    path.split('.').map(str::to_string).collect()
}

#[test]
fn references_hold_the_paths_of_tables_and_modules() {
    let text = "IMPORT MODULE charts.scales;
FROM sales.orders AS o JOIN `Customers` USING (id) |> CALL f(TABLE refunds) |> WHERE id IN (FROM ok);
FROM my-project.ds.events";
    let parse = avenger_sql_syntax::parse(text);
    assert!(parse.ok(), "{:?}", parse.diagnostics());
    let found = references(&parse.syntax());
    let tables: Vec<Vec<String>> = found.tables.iter().map(|r| r.names.clone()).collect();
    assert_eq!(
        tables,
        [
            names("sales.orders"),
            names("Customers"),
            names("refunds"),
            names("ok"),
            names("my-project.ds.events"),
        ]
    );
    assert_eq!(&text[found.tables[0].range], "sales.orders");
    let modules: Vec<Vec<String>> = found.modules.iter().map(|r| r.names.clone()).collect();
    assert_eq!(modules, [names("charts.scales")]);
}

fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]))
}

#[test]
fn a_memory_catalog_finds_tables_without_regard_to_case() {
    let catalog = MemoryCatalog::new().with_schema(&["Sales", "Orders"], schema());
    let table = catalog.table(&names("sales.ORDERS")).expect("a table");
    assert_eq!(table.names, ["Sales", "Orders"]);
    assert_eq!(table.schema(), schema());
    assert!(catalog.table(&names("orders")).is_none());
    let tables: Vec<Vec<String>> = catalog.tables().into_iter().map(|t| t.names).collect();
    assert_eq!(tables, [["Sales", "Orders"]]);
}

#[tokio::test]
async fn a_session_catalog_loads_the_tables_that_a_script_names() {
    let ctx = SessionContext::new();
    ctx.register_table("Orders", Arc::new(EmptyTable::new(schema())))
        .unwrap();
    let sales = Arc::new(MemorySchemaProvider::new());
    sales
        .register_table("q1".to_string(), Arc::new(EmptyTable::new(schema())))
        .unwrap();
    ctx.catalog("datafusion")
        .unwrap()
        .register_schema("sales", sales)
        .unwrap();
    let text = "FROM orders; FROM SALES.Q1; FROM datafusion.public.ORDERS; FROM missing";
    let parse = avenger_sql_syntax::parse(text);
    let catalog = SessionCatalog::load(&ctx, &references(&parse.syntax()))
        .await
        .unwrap();
    for path in ["orders", "ORDERS", "sales.q1", "datafusion.public.orders"] {
        let table = catalog
            .table(&names(path))
            .unwrap_or_else(|| panic!("{path}"));
        assert_eq!(table.schema(), schema(), "{path}");
    }
    assert!(catalog.table(&names("missing")).is_none());
    // Its tables are those that it loaded, under the paths that the script writes.
    let tables: Vec<String> = (catalog.tables().into_iter())
        .map(|table| table.names.join("."))
        .collect();
    assert_eq!(tables, ["orders", "SALES.Q1", "datafusion.public.ORDERS"]);
    // DataFusion's sessions map strings to views by default.
    assert!(catalog.type_options().utf8_view);
}
