//! A chart script's own tables of files: its `CREATE TEMP EXTERNAL TABLE` and `LOAD DATA ...
//! TEMP TABLE` statements, analyzed as a data script's are, over `MemoryCatalog`s without I/O.

use std::sync::{Arc, OnceLock};

use avenger_sql::{
    analysis::{Analysis, Options, StatementPlan},
    analyze,
    catalog::{references, MemoryCatalog},
    data::{file_reads, DefinitionKind, LoadMode, ScriptTable},
    library::Library,
    types::TypeOptions,
};
use avenger_sql_syntax::parse;
use datafusion::{
    arrow::datatypes::{DataType, Field, Schema, SchemaRef},
    common::tree_node::{TreeNode, TreeNodeRecursion},
    datasource::source_as_provider,
    logical_expr::LogicalPlan,
};

fn library() -> &'static Library {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY.get_or_init(Library::new)
}

/// The schema of the files of the tests: a region, a month, and an amount.
fn sales() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, true),
        Field::new("month", DataType::Int64, true),
        Field::new("amount", DataType::Float64, true),
    ]))
}

/// The analysis of the chart script `text`, which parses without errors, with a table `sales`
/// of the catalog and a table of [`sales`] for each of its file reads.
fn analyze_chart(text: &str) -> Analysis {
    let parse = parse(text);
    assert!(parse.ok(), "{:?}", parse.diagnostics());
    let root = parse.syntax();
    let catalog = (file_reads(&root, TypeOptions::default()).into_iter()).fold(
        MemoryCatalog::new().with_schema(&["sales"], sales()),
        |catalog, read| catalog.with_files_schema(read.files, sales()),
    );
    analyze(&root, &catalog, library(), &Options::default())
}

/// The errors of `analysis`, each as the text at its range and its message.
fn errors<'a>(text: &'a str, analysis: &Analysis) -> Vec<(&'a str, String)> {
    (analysis.diagnostics.iter())
        .map(|diagnostic| (&text[diagnostic.range], diagnostic.message.clone()))
        .collect()
}

/// The placeholders that `plan` scans, as their paths and definitions.
fn placeholders(plan: &LogicalPlan) -> Vec<(String, usize)> {
    let mut found = Vec::new();
    plan.apply(|node| {
        if let LogicalPlan::TableScan(scan) = node {
            let provider = source_as_provider(&scan.source).unwrap();
            if let Some(table) = provider.downcast_ref::<ScriptTable>() {
                found.push((table.names().join("."), table.definition()));
            }
        }
        Ok(TreeNodeRecursion::Continue)
    })
    .unwrap();
    found
}

const CHART: &str = "DECLARE least FLOAT64 DEFAULT 0;
CREATE TEMP EXTERNAL TABLE orders OPTIONS (format = 'CSV', uris = ['data/files/orders.csv']);
LOAD DATA OVERWRITE TEMP TABLE rates FROM FILES (format = 'CSV', uris = ['data/files/rates/*.csv']);
LOAD DATA INTO TEMP TABLE rates FROM FILES (format = 'CSV', uris = ['data/files/more-rates.csv']);
FROM orders JOIN rates USING (region, month)
|> WHERE orders.amount > least
|> AGGREGATE SUM(orders.amount * rates.amount) AS total GROUP BY region";

#[test]
fn a_chart_defines_its_tables_of_files_and_reads_them_by_placeholders() {
    let analysis = analyze_chart(CHART);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let kinds: Vec<(String, bool, Option<LoadMode>)> = (analysis.file_tables.iter())
        .map(|definition| {
            let mode = match &definition.kind {
                DefinitionKind::LoadData { mode, .. } => Some(*mode),
                _ => None,
            };
            (definition.names.join("."), definition.temp, mode)
        })
        .collect();
    assert_eq!(
        kinds,
        [
            ("orders".to_string(), true, None),
            ("rates".to_string(), true, Some(LoadMode::Overwrite)),
            ("rates".to_string(), true, Some(LoadMode::Into)),
        ]
    );
    assert!(matches!(
        analysis.file_tables[0].kind,
        DefinitionKind::ExternalTable { files: Some(_) }
    ));
    assert_eq!(analysis.file_tables[2].appends(), Some(1));
    // The query reads the placeholders, and a read of the appended table sees the last append.
    let plan = analysis.plan.as_ref().expect("a plan of the query");
    assert_eq!(
        placeholders(plan),
        [("orders".to_string(), 0), ("rates".to_string(), 2)]
    );
    // Each statement of files is a definition of the script.
    let definitions = (analysis.statements.iter())
        .filter(|statement| matches!(statement.plan, StatementPlan::Definition))
        .count();
    assert_eq!(definitions, 3);
}

#[test]
fn the_references_of_a_script_leave_out_its_own_tables() {
    let text = "FROM sales |> LIMIT 1;
CREATE TEMP EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['sales.csv']);
CREATE TEMP TABLE recent AS (FROM sales |> WHERE month > 6);
FROM recent JOIN sales USING (region) |> JOIN other USING (region)";
    let tables: Vec<String> = (references(&parse(text).syntax()).tables.into_iter())
        .map(|reference| reference.names.join("."))
        .collect();
    // The read before the definition is a reference too, though the analysis reports it as an
    // error, since a later statement defines the table.
    assert_eq!(tables, ["sales", "other"]);
}

#[test]
fn a_table_of_files_reads_only_the_tables_before_it_and_takes_a_free_name() {
    // A read of a table of files before its statement is an error, as in a data script.
    let text = "DECLARE x INT64;
SET x = (FROM orders |> AGGREGATE COUNT(*));
CREATE TEMP EXTERNAL TABLE orders OPTIONS (format = 'CSV', uris = ['orders.csv']);
FROM orders";
    let found = errors(text, &analyze_chart(text));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "orders");
    assert!(found[0].1.contains("later statement"), "{found:?}");
    // A temporary table and a table of files of one script have two names.
    for text in [
        "CREATE TEMP TABLE t AS (FROM sales);
CREATE TEMP EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['t.csv']);
FROM t",
        "CREATE TEMP EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['t.csv']);
CREATE TEMP TABLE t AS (FROM sales);
FROM t",
    ] {
        let found = errors(text, &analyze_chart(text));
        assert_eq!(
            found,
            [("t", "Table t already exists".to_string())],
            "{text}"
        );
    }
}

#[test]
fn a_module_reads_the_tables_of_the_catalog_and_not_those_of_the_chart() {
    let text = "IMPORT MODULE m;
CREATE TEMP EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['sales.csv']);
FROM m.recent(5)";
    let parse = parse(text);
    assert!(parse.ok(), "{:?}", parse.diagnostics());
    let root = parse.syntax();
    let catalog = (file_reads(&root, TypeOptions::default()).into_iter()).fold(
        MemoryCatalog::new()
            .with_schema(&["sales"], sales())
            .with_module(
            &["m"],
            "MODULE m; CREATE PUBLIC TABLE FUNCTION recent(n INT64) AS (FROM sales |> LIMIT n);",
        ),
        |catalog, read| catalog.with_files_schema(read.files, sales()),
    );
    let analysis = analyze(&root, &catalog, library(), &Options::default());
    assert_eq!(errors(text, &analysis), []);
    // The call plans a scan of the catalog's table, not of the chart's table of files.
    let plan = analysis.plan.as_ref().unwrap();
    assert_eq!(placeholders(plan), []);
    assert!(plan
        .display_indent()
        .to_string()
        .contains("TableScan: sales"));
}

/// The columns of a chart's temporary table take one name each, as those of its tables of files
/// do, since both read their column lists with the same code.
#[test]
fn a_column_of_a_temporary_table_takes_one_name() {
    let text = "CREATE TEMP TABLE w (a INT64, A STRING); FROM w";
    let analysis = analyze(
        &parse(text).syntax(),
        &MemoryCatalog::new(),
        library(),
        &Options::default(),
    );
    assert_eq!(
        errors(text, &analysis),
        [("A", "Column A is named twice".to_string())]
    );
}

#[test]
fn a_chart_without_tables_of_files_has_no_file_tables() {
    let analysis = analyze_chart("FROM sales |> WHERE amount > 0");
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert!(analysis.file_tables.is_empty());
}
