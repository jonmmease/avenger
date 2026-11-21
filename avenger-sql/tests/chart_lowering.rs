//! The tables of a chart script's own tables of files, which `ScriptTables` builds over an
//! `InMemory` object store without registering them, and which the chart's plans read in place
//! of their placeholders.

use std::sync::{Arc, OnceLock};

use arrow::util::pretty::pretty_format_batches;
use avenger_sql::{
    analysis::{Analysis, Options},
    analyze,
    catalog::{references, Catalog, SessionCatalog},
    data::{file_reads, file_table, ScriptTables},
    library::Library,
    project::ProjectCatalog,
    ScriptError,
};
use avenger_sql_syntax::parse;
use datafusion::{
    common::DataFusionError,
    execution::object_store::ObjectStoreUrl,
    object_store::{memory::InMemory, path::Path, ObjectStoreExt, PutPayload},
    prelude::SessionContext,
};

/// The URL of the project's root, at the object store of each test's files.
const BASE: &str = "memory://project/";

const SALES_CSV: &[u8] = include_bytes!("data/files/sales.csv");

fn library() -> &'static Library {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY.get_or_init(Library::new)
}

/// A session that reads the files of an `InMemory` store at [`BASE`], with `sales.csv` and a
/// copy of it in `more/`.
async fn session() -> SessionContext {
    let ctx = SessionContext::new();
    avenger_sql::register(&ctx).unwrap();
    let store = Arc::new(InMemory::new());
    let url = ObjectStoreUrl::parse("memory://project").unwrap();
    ctx.register_object_store(url.as_ref(), store.clone());
    for path in ["sales.csv", "more/sales.csv"] {
        let payload = PutPayload::from(SALES_CSV.to_vec());
        store
            .put(&Path::parse(path).unwrap(), payload)
            .await
            .unwrap();
    }
    ctx
}

/// The analysis of the chart script `text` in `ctx`, whose catalog holds the tables of the
/// session that it reads and the table of each of its file reads, and that catalog.
async fn analyze_chart(ctx: &SessionContext, text: &str) -> (Analysis, ProjectCatalog) {
    let parse = parse(text);
    assert!(parse.ok(), "{:?}", parse.diagnostics());
    let root = parse.syntax();
    let base = SessionCatalog::load(ctx, &references(&root)).await.unwrap();
    let types = base.type_options();
    let mut catalog = ProjectCatalog::new(Arc::new(base));
    let state = ctx.state();
    for read in file_reads(&root, types) {
        let table = file_table(&state, &read.files, BASE).await;
        catalog = catalog.with_files(read.files, table);
    }
    let analysis = analyze(&root, &catalog, library(), &Options::default());
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    (analysis, catalog)
}

/// The rows of the chart script `text` in `ctx`, with the tables of its files, as a table.
async fn rows(ctx: &SessionContext, text: &str) -> String {
    let (analysis, catalog) = analyze_chart(ctx, text).await;
    let tables = ScriptTables::build(ctx, &analysis.file_tables, &catalog)
        .await
        .unwrap();
    let frame = analysis.with_tables(&tables).unwrap().dataframe(ctx).await;
    let batches = frame.unwrap().collect().await.unwrap();
    pretty_format_batches(&batches).unwrap().to_string()
}

#[tokio::test]
async fn a_chart_reads_its_external_table_without_registering_it() {
    let ctx = session().await;
    let text = "CREATE TEMP EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['sales.csv']);
FROM sales |> AGGREGATE SUM(amount) AS total GROUP BY region |> ORDER BY region";
    let expected = "\
+--------+-------+
| region | total |
+--------+-------+
| east   | 17.75 |
| west   | 20.0  |
+--------+-------+";
    assert_eq!(rows(&ctx, text).await, expected);
    // The session holds no table of the chart's.
    assert!(!ctx.table_exist("sales").unwrap());
}

#[tokio::test]
async fn a_chart_loads_its_files_into_memory_and_appends() {
    let ctx = session().await;
    let text = "DECLARE least FLOAT64 DEFAULT 8;
LOAD DATA OVERWRITE TEMP TABLE sales FROM FILES (format = 'CSV', uris = ['sales.csv']);
LOAD DATA INTO TEMP TABLE sales FROM FILES (format = 'CSV', uris = ['more/']);
SET least = (FROM sales |> AGGREGATE MIN(amount) + 1);
FROM sales |> WHERE amount > least |> AGGREGATE COUNT(*) AS n";
    let expected = "\
+---+
| n |
+---+
| 4 |
+---+";
    assert_eq!(rows(&ctx, text).await, expected);
    assert!(!ctx.table_exist("sales").unwrap());
}

#[tokio::test]
async fn a_failing_load_is_the_error_of_its_definition() {
    let ctx = session().await;
    // The files have three columns, and the column list two, so their rows fail to load.
    let text = "LOAD DATA OVERWRITE TEMP TABLE sales (region STRING, amount INT64)
FROM FILES (format = 'CSV', uris = ['sales.csv'], skip_leading_rows = 1);
FROM sales";
    let (analysis, catalog) = analyze_chart(&ctx, text).await;
    let errors = ScriptTables::build(&ctx, &analysis.file_tables, &catalog)
        .await
        .unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].definition, 0);
}

#[tokio::test]
async fn a_chart_with_tables_of_files_runs_only_in_a_project() {
    let ctx = session().await;
    let text = "CREATE TEMP EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['sales.csv']);
LOAD DATA OVERWRITE TEMP TABLE more FROM FILES (format = 'CSV', uris = ['more/']);
FROM sales";
    let error = avenger_sql::sql(&ctx, text).await.unwrap_err();
    let DataFusionError::External(error) = error else {
        panic!("{error}");
    };
    let error = error.downcast_ref::<ScriptError>().unwrap();
    let at: Vec<&str> = (error.diagnostics.iter())
        .map(|diagnostic| &text[diagnostic.range][..9])
        .collect();
    assert_eq!(at, ["CREATE TE", "LOAD DATA"]);
    assert!(error.diagnostics[0].message.contains("project"));
    let lifted = avenger_sql::lift(&ctx, text).await.unwrap_err();
    assert!(lifted.to_string().contains("project"), "{lifted}");
}

#[tokio::test]
async fn a_table_of_files_without_temp_is_an_error_of_the_parse() {
    let ctx = session().await;
    let text = "CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['sales.csv']);
FROM sales";
    let error = avenger_sql::sql(&ctx, text).await.unwrap_err();
    let DataFusionError::External(error) = error else {
        panic!("{error}");
    };
    let error = error.downcast_ref::<ScriptError>().unwrap();
    // The parse's error names a data script, and the chart has no error that a project runs it.
    assert!(!error.diagnostics.is_empty());
    for diagnostic in &error.diagnostics {
        assert!(
            !diagnostic.message.contains("a project runs the chart"),
            "{}",
            diagnostic.message
        );
    }
}
