//! The analysis of data scripts: the options of files, what each statement defines, the reads of
//! tables within a script and outside it, and the checks, over `MemoryCatalog`s without I/O.

use std::sync::{Arc, OnceLock};

use avenger_sql::{
    analysis::Options,
    analyze_data,
    catalog::{Catalog, MemoryCatalog, TableDefinition},
    data::{
        file_option, file_reads, uri_kind, DataAnalysis, DefinitionKind, FileRead, Format,
        LoadMode, OptionPlace, Partitions, ScriptTable, UriKind, FILE_OPTIONS, UNSUPPORTED_OPTIONS,
    },
    library::Library,
    types::TypeOptions,
};
use avenger_sql_syntax::{parse_data, TextRange};
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

fn reads(text: &str) -> Vec<FileRead> {
    file_reads(&parse_data(text).syntax(), TypeOptions::default())
}

/// `catalog`, with a table of the schema of [`sales`] for each file read of `text`.
fn with_files(text: &str, catalog: MemoryCatalog) -> MemoryCatalog {
    (reads(text).into_iter()).fold(catalog, |catalog, read| {
        catalog.with_files_schema(read.files, sales())
    })
}

/// The analysis of the data script `text` with `catalog`, which has no syntax errors.
fn analyze_with(text: &str, catalog: &MemoryCatalog) -> DataAnalysis {
    let parse = parse_data(text);
    assert!(parse.ok(), "{:?}", parse.diagnostics());
    analyze_data(&parse.syntax(), catalog, library(), &Options::default())
}

/// The analysis of `text` with the table `t` of [`sales`], and a table of [`sales`] for each of
/// its file reads.
fn analyze(text: &str) -> DataAnalysis {
    let catalog = MemoryCatalog::new().with_schema(&["t"], sales());
    analyze_with(text, &with_files(text, catalog))
}

/// The errors of `analysis`, each as the text at its range and its message.
fn errors<'a>(text: &'a str, analysis: &DataAnalysis) -> Vec<(&'a str, String)> {
    (analysis.diagnostics.iter())
        .map(|diagnostic| (&text[diagnostic.range], diagnostic.message.clone()))
        .collect()
}

/// The one error of `text`, analyzed with the tables of its file reads.
fn error(text: &str) -> (&str, String) {
    let found = errors(text, &analyze(text));
    assert_eq!(found.len(), 1, "{found:?}");
    found.into_iter().next().unwrap()
}

fn assert_ok(analysis: &DataAnalysis) {
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
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

fn names(path: &str) -> Vec<String> {
    path.split('.').map(str::to_string).collect()
}

#[test]
fn the_table_of_options_holds_the_options_of_files_and_of_tables() {
    let names: Vec<&str> = FILE_OPTIONS.iter().map(|option| option.name).collect();
    assert_eq!(
        names,
        [
            "format",
            "uris",
            "skip_leading_rows",
            "field_delimiter",
            "quote",
            "null_marker",
            "allow_quoted_newlines",
            "compression",
            "hive_partition_uri_prefix",
            "description",
        ]
    );
    let format = file_option("FORMAT").unwrap();
    assert!(format.required);
    assert_eq!(format.ty.to_string(), "STRING");
    assert!(format.values.contains(&"JSON"));
    assert_eq!(file_option("uris").unwrap().ty.to_string(), "ARRAY<STRING>");
    assert_eq!(
        file_option("compression").unwrap().formats,
        [Format::Csv, Format::Json]
    );
    assert_eq!(
        file_option("description").unwrap().place,
        OptionPlace::Table
    );
    for name in [
        "max_staleness",
        "metadata_cache_mode",
        "require_hive_partition_filter",
        "encoding",
        "date_format",
    ] {
        assert!(
            UNSUPPORTED_OPTIONS.iter().any(|option| option.name == name),
            "{name}"
        );
    }
    // The values that a data script takes anyway, as GoogleSQL writes them.
    let accepts = |name: &str| -> Vec<String> {
        let option = UNSUPPORTED_OPTIONS
            .iter()
            .find(|option| option.name == name);
        (option.unwrap().accepts.iter())
            .map(ToString::to_string)
            .collect()
    };
    assert_eq!(accepts("encoding"), ["'UTF8'", "'UTF-8'"]);
    assert_eq!(accepts("max_bad_records"), ["0"]);
    assert_eq!(accepts("allow_jagged_rows"), ["FALSE"]);
    assert!(accepts("date_format").is_empty());
    // No option is both supported and unsupported.
    for option in UNSUPPORTED_OPTIONS {
        assert!(file_option(option.name).is_none(), "{}", option.name);
    }
}

#[test]
fn formats_parse_without_regard_to_case_and_json_is_an_alias() {
    assert_eq!(Format::parse("csv"), Some(Format::Csv));
    assert_eq!(Format::parse("JSON"), Some(Format::Json));
    assert_eq!(Format::parse("Newline_Delimited_Json"), Some(Format::Json));
    assert_eq!(Format::parse("arrow"), Some(Format::Arrow));
    assert_eq!(Format::parse("xls"), None);
    assert_eq!(Format::Json.name(), "NEWLINE_DELIMITED_JSON");
    assert!(Format::Csv.available() && Format::Json.available() && Format::Arrow.available());
    assert_eq!(Format::Parquet.available(), cfg!(feature = "parquet"));
    assert_eq!(Format::Avro.available(), cfg!(feature = "avro"));

    // The two names of JSON read the same files.
    let text = "CREATE EXTERNAL TABLE a OPTIONS (format = 'JSON', uris = ['cars.json']);
CREATE EXTERNAL TABLE b OPTIONS (format = 'newline_delimited_json', uris = ['cars.json'])";
    let found = reads(text);
    assert_eq!(found[0].files.format, Format::Json);
    assert_eq!(found[0].files, found[1].files);
    assert_ok(&analyze(text));
}

#[test]
fn file_reads_hold_the_files_of_external_tables_and_loads() {
    let text = "CREATE EXTERNAL TABLE sales
OPTIONS (
  format = 'csv',
  uris = ['data/files/sales/*.csv', 'data/files/extra.csv'],
  field_delimiter = ';',
  quote = \"'\",
  null_marker = '',
  allow_quoted_newlines = TRUE,
  description = 'The sales'
);
LOAD DATA INTO events (id INT64 NOT NULL, kind STRING)
OPTIONS (description = 'The events')
FROM FILES (format = 'CSV', uris = ['data/files/events.csv'], skip_leading_rows = 1);
LOAD DATA OVERWRITE trips
FROM FILES (format = 'ARROW', uris = ['data/files/trips/'], hive_partition_uri_prefix = 'data/files/trips/')
WITH PARTITION COLUMNS (year INT64);
LOAD DATA OVERWRITE broken FROM FILES (format = 'CSV')";
    let found = reads(text);
    let paths: Vec<String> = found.iter().map(|read| read.names.join(".")).collect();
    assert_eq!(paths, ["sales", "events", "trips"]);

    let sales = &found[0];
    assert_eq!(sales.files.format, Format::Csv);
    assert_eq!(
        sales.files.uris,
        ["data/files/sales/*.csv", "data/files/extra.csv"]
    );
    assert!(&text[sales.uris].starts_with("['data/files/sales/*.csv'"));
    assert!(text[sales.range].starts_with("CREATE EXTERNAL TABLE sales"));
    let csv = sales.files.csv.as_ref().unwrap();
    assert!(csv.header);
    assert_eq!((csv.delimiter, csv.quote), (b';', b'\''));
    assert!(csv.quoted_newlines);
    assert_eq!(sales.files.columns, None);

    // A column list names the columns, and skip_leading_rows = 1 skips the header.
    let events = &found[1];
    let columns = events.files.columns.as_ref().unwrap();
    assert_eq!(
        columns[0].as_ref(),
        &Field::new("id", DataType::Int64, false)
    );
    assert_eq!(
        columns[1].as_ref(),
        &Field::new("kind", DataType::Utf8, true)
    );
    assert!(events.files.csv.as_ref().unwrap().header);

    let trips = &found[2];
    assert_eq!(trips.files.csv, None);
    assert_eq!(
        trips.files.hive_partition_uri_prefix.as_deref(),
        Some("data/files/trips/")
    );
    let Some(Partitions::Columns(keys)) = &trips.files.partitions else {
        panic!("{:?}", trips.files.partitions);
    };
    assert_eq!(keys[0].as_ref(), &Field::new("year", DataType::Int64, true));
}

#[test]
fn definitions_record_names_kinds_schemas_and_descriptions() {
    let text = "CREATE EXTERNAL TABLE sales
OPTIONS (format = 'CSV', uris = ['data/files/sales.csv'], description = 'Monthly sales');
CREATE VIEW totals OPTIONS (description = 'Totals by region') AS (
  FROM sales |> AGGREGATE sum(amount) AS total GROUP BY region
);
CREATE TABLE big (region STRING, total FLOAT64) AS (FROM totals |> WHERE total > 100);
CREATE SCHEMA finance OPTIONS (description = 'The books')";
    let analysis = analyze(text);
    assert_ok(&analysis);
    let definitions = &analysis.definitions;
    let paths: Vec<String> = definitions.iter().map(|d| d.names.join(".")).collect();
    assert_eq!(paths, ["sales", "totals", "big", "finance"]);

    let external = &definitions[0];
    let DefinitionKind::ExternalTable { files: Some(files) } = &external.kind else {
        panic!("{:?}", external.kind);
    };
    assert_eq!(files.uris, ["data/files/sales.csv"]);
    assert_eq!(external.schema, Some(sales()));
    assert_eq!(external.description.as_deref(), Some("Monthly sales"));
    assert_eq!(&text[external.name_range], "sales");
    assert!(text[external.range].ends_with("description = 'Monthly sales')"));
    assert!(!external.temp);

    let totals = &definitions[1];
    let DefinitionKind::View { plan: Some(plan) } = &totals.kind else {
        panic!("{:?}", totals.kind);
    };
    assert_eq!(placeholders(plan), [("sales".to_string(), 0)]);
    let schema = totals.schema.as_ref().unwrap();
    let columns: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    assert_eq!(columns, ["region", "total"]);
    assert_eq!(totals.description.as_deref(), Some("Totals by region"));

    // A column list gives the names and the types of the query's columns.
    let big = &definitions[2];
    assert!(matches!(
        big.kind,
        DefinitionKind::TableAs { plan: Some(_) }
    ));
    let schema = big.schema.as_ref().unwrap();
    assert_eq!(schema.field(1).data_type(), &DataType::Float64);

    let finance = &definitions[3];
    assert!(matches!(finance.kind, DefinitionKind::Schema));
    assert_eq!(finance.schema, None);
    assert_eq!(finance.description.as_deref(), Some("The books"));

    // The reads stay in the script.
    assert!(analysis.reads.is_empty());
    assert_eq!(analysis.exported().count(), 4);
}

#[test]
fn a_view_over_an_earlier_load_plans_a_scan_of_its_placeholder() {
    let text = "LOAD DATA OVERWRITE sales FROM FILES (format = 'CSV', uris = ['sales.csv']);
CREATE VIEW west AS (FROM sales |> WHERE region = 'west')";
    let analysis = analyze(text);
    assert_ok(&analysis);
    let DefinitionKind::LoadData {
        files: Some(_),
        mode: LoadMode::Overwrite,
        appends: None,
    } = &analysis.definitions[0].kind
    else {
        panic!("{:?}", analysis.definitions[0].kind);
    };
    let DefinitionKind::View { plan: Some(plan) } = &analysis.definitions[1].kind else {
        panic!("{:?}", analysis.definitions[1].kind);
    };
    assert_eq!(placeholders(plan), [("sales".to_string(), 0)]);
    assert_eq!(analysis.definitions[1].schema, Some(sales()));
}

#[test]
fn load_data_into_appends_to_a_table_of_the_script() {
    let text = "LOAD DATA INTO sales FROM FILES (format = 'CSV', uris = ['2024.csv']);
CREATE VIEW first AS (FROM sales);
LOAD DATA INTO sales FROM FILES (format = 'JSON', uris = ['2025.json']);
CREATE TABLE copy AS (FROM sales);
LOAD DATA INTO copy FROM FILES (format = 'CSV', uris = ['more.csv']);
CREATE VIEW both AS (FROM sales JOIN copy USING (region, month, amount))";
    let analysis = analyze(text);
    assert_ok(&analysis);
    let appends: Vec<Option<usize>> = (analysis.definitions.iter())
        .map(|definition| definition.appends())
        .collect();
    assert_eq!(appends, [None, None, Some(0), None, Some(3), None]);
    // A read sees the rows of the loads before it.
    let plan = |index: usize| match &analysis.definitions[index].kind {
        DefinitionKind::View { plan: Some(plan) }
        | DefinitionKind::TableAs { plan: Some(plan) } => plan.clone(),
        kind => panic!("{kind:?}"),
    };
    assert_eq!(placeholders(&plan(1)), [("sales".to_string(), 0)]);
    assert_eq!(placeholders(&plan(3)), [("sales".to_string(), 2)]);
    assert_eq!(
        placeholders(&plan(5)),
        [("sales".to_string(), 2), ("copy".to_string(), 4)]
    );
    let exported: Vec<String> = (analysis.exported())
        .map(|definition| definition.names.join("."))
        .collect();
    assert_eq!(exported, ["sales", "first", "copy", "both"]);
}

#[test]
fn load_data_into_appends_only_to_loaded_tables_of_the_script() {
    let (at, message) = error(
        "CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['a.csv']);
LOAD DATA INTO sales FROM FILES (format = 'CSV', uris = ['b.csv'])",
    );
    assert_eq!(at, "sales");
    assert!(message.starts_with("sales is an external table, and LOAD DATA INTO appends only"));

    let (_, message) = error(
        "LOAD DATA OVERWRITE sales FROM FILES (format = 'CSV', uris = ['a.csv']);
CREATE VIEW v AS (FROM sales);
LOAD DATA INTO v FROM FILES (format = 'CSV', uris = ['b.csv'])",
    );
    assert!(message.starts_with("v is a view"), "{message}");

    // A table of the catalog belongs to another script or to the host.
    let text = "LOAD DATA INTO orders FROM FILES (format = 'CSV', uris = ['b.csv'])";
    let catalog = with_files(text, MemoryCatalog::new().with_schema(&["orders"], sales()));
    let found = errors(text, &analyze_with(text, &catalog));
    assert_eq!(found.len(), 1);
    assert!(found[0]
        .1
        .contains("a table of another data script or of the host"));

    let (_, message) = error(
        "LOAD DATA INTO TEMP TABLE raw FROM FILES (format = 'CSV', uris = ['a.csv']);
LOAD DATA INTO raw FROM FILES (format = 'CSV', uris = ['b.csv'])",
    );
    assert!(message.starts_with("raw is a temporary table"), "{message}");

    // The files of an append have the columns of the table.
    let text = "LOAD DATA INTO sales FROM FILES (format = 'CSV', uris = ['a.csv']);
LOAD DATA INTO sales FROM FILES (format = 'CSV', uris = ['b.csv'])";
    let b = reads(text).remove(1).files;
    let narrow = Arc::new(Schema::new(vec![Field::new(
        "region",
        DataType::Utf8,
        true,
    )]));
    let catalog = (reads(text).into_iter().take(1))
        .fold(MemoryCatalog::new(), |catalog, read| {
            catalog.with_files_schema(read.files, sales())
        })
        .with_files_schema(b, narrow);
    let found = errors(text, &analyze_with(text, &catalog));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, "['b.csv']");
    assert_eq!(
        found[0].1,
        "The files lack the columns month INT64, amount FLOAT64 of sales: LOAD DATA INTO appends files whose columns match the table's by name, with values that coerce to the types of the table's"
    );

    // The files of an append have the types of the table's columns, and a name matches without
    // regard to case.
    let text = "LOAD DATA OVERWRITE t2 (a INT64) FROM FILES (format = 'CSV', uris = ['a.csv']);
LOAD DATA INTO t2 (A STRING) FROM FILES (format = 'CSV', uris = ['b.csv'])";
    let schema = |name: &str, ty: DataType| Arc::new(Schema::new(vec![Field::new(name, ty, true)]));
    let mut files = reads(text).into_iter().map(|read| read.files);
    let catalog = MemoryCatalog::new()
        .with_files_schema(files.next().unwrap(), schema("a", DataType::Int64))
        .with_files_schema(files.next().unwrap(), schema("A", DataType::Utf8));
    let found = errors(text, &analyze_with(text, &catalog));
    assert_eq!(
        found,
        [(
            "['b.csv']",
            "The files have the columns A STRING, where t2 has the columns a INT64: LOAD DATA INTO appends files whose columns match the table's by name, with values that coerce to the types of the table's".to_string()
        )]
    );
    // Values that coerce to the table's type match, since the append casts them, as INT64 values
    // to FLOAT64, and so do types that GoogleSQL names alike, but not FLOAT64 values to INT64.
    for (table, files, ok) in [
        (DataType::Float64, DataType::Int64, true),
        (DataType::Utf8, DataType::Utf8View, true),
        (DataType::Int64, DataType::Float64, false),
    ] {
        let mut reads = reads(text).into_iter().map(|read| read.files);
        let catalog = MemoryCatalog::new()
            .with_files_schema(reads.next().unwrap(), schema("a", table.clone()))
            .with_files_schema(reads.next().unwrap(), schema("A", files.clone()));
        let analysis = analyze_with(text, &catalog);
        assert_eq!(analysis.diagnostics.is_empty(), ok, "{table} {files}");
    }
}

#[test]
fn an_append_takes_the_description_of_its_table() {
    let text = "LOAD DATA INTO sales OPTIONS (description = 'Sales') FROM FILES (format = 'CSV', uris = ['a.csv']);
LOAD DATA INTO sales OPTIONS (description = 'More') FROM FILES (format = 'CSV', uris = ['b.csv'])";
    let analysis = analyze(text);
    assert_eq!(
        errors(text, &analysis),
        [(
            "description",
            "LOAD DATA INTO sales appends to the table of an earlier statement, which gives the table's description".to_string()
        )]
    );
    assert_eq!(
        analysis.definitions[1].description.as_deref(),
        Some("Sales")
    );
}

#[test]
fn load_data_overwrite_creates_its_table() {
    let (at, message) = error(
        "LOAD DATA INTO sales FROM FILES (format = 'CSV', uris = ['a.csv']);
LOAD DATA OVERWRITE sales FROM FILES (format = 'CSV', uris = ['b.csv'])",
    );
    assert_eq!(at, "sales");
    assert!(message.starts_with("LOAD DATA OVERWRITE of sales, which an earlier statement"));
}

#[test]
fn temp_tables_stay_in_their_script() {
    let text = "CREATE TEMP EXTERNAL TABLE raw_events
OPTIONS (format = 'CSV', uris = ['events/*.csv']);
LOAD DATA OVERWRITE TEMP TABLE raw_sales FROM FILES (format = 'CSV', uris = ['sales.csv']);
CREATE TEMP VIEW recent AS (FROM raw_events |> WHERE month > 6);
CREATE TABLE events AS (FROM recent JOIN raw_sales USING (region, month, amount))";
    let analysis = analyze(text);
    assert_ok(&analysis);
    let temp: Vec<bool> = analysis.definitions.iter().map(|d| d.temp).collect();
    assert_eq!(temp, [true, true, true, false]);
    let exported: Vec<String> = (analysis.exported())
        .map(|definition| definition.names.join("."))
        .collect();
    assert_eq!(exported, ["events"]);
    let DefinitionKind::TableAs { plan: Some(plan) } = &analysis.definitions[3].kind else {
        panic!();
    };
    assert_eq!(
        placeholders(plan),
        [("recent".to_string(), 2), ("raw_sales".to_string(), 1)]
    );
}

#[test]
fn schemas_hold_the_tables_of_qualified_names() {
    let text = "CREATE SCHEMA vega_datasets;
CREATE EXTERNAL TABLE vega_datasets.cars OPTIONS (format = 'JSON', uris = ['cars.json']);
CREATE VIEW vega_datasets.heavy AS (FROM vega_datasets.cars |> WHERE amount > 3000)";
    let analysis = analyze(text);
    assert_ok(&analysis);
    let DefinitionKind::View { plan: Some(plan) } = &analysis.definitions[2].kind else {
        panic!();
    };
    assert_eq!(placeholders(plan), [("vega_datasets.cars".to_string(), 1)]);

    // A schema that nothing creates.
    let (at, message) = error(
        "CREATE EXTERNAL TABLE finance.orders OPTIONS (format = 'CSV', uris = ['orders.csv'])",
    );
    assert_eq!(at, "finance");
    assert_eq!(
        message,
        "Schema finance not found: a CREATE SCHEMA creates a schema, which BigQuery calls a dataset, before the tables in it"
    );

    // A schema of the catalog, from another script or the host.
    let text = "CREATE VIEW finance.totals AS (FROM finance.orders)";
    let catalog = MemoryCatalog::new().with_schema(&["finance", "orders"], sales());
    let analysis = analyze_with(text, &catalog);
    assert_ok(&analysis);
    let tables: Vec<Vec<String>> = analysis.reads.iter().map(|r| r.names.clone()).collect();
    assert_eq!(tables, [names("finance.orders")]);
    // An empty schema of the catalog.
    let analysis = analyze_with(
        "CREATE VIEW books.v AS (FROM finance.orders)",
        &catalog.clone().with_namespace(&["Books"]),
    );
    assert_ok(&analysis);

    // A schema that a later statement creates.
    let (at, message) = error(
        "CREATE VIEW finance.v AS (FROM t);
CREATE SCHEMA finance",
    );
    assert_eq!(at, "finance");
    assert!(message.starts_with("Schema finance is created by a later statement"));

    let (at, message) = error("CREATE SCHEMA a.b");
    assert_eq!((at, message.as_str()), ("a.b", "A schema has one name"));
    let (at, _) = error("CREATE VIEW a.b.c AS (FROM t)");
    assert_eq!(at, "a.b.c");
}

#[test]
fn reads_leave_the_script_or_resolve_to_earlier_statements() {
    // A read of a table that a later statement defines.
    let text = "CREATE VIEW v AS (FROM sales);
LOAD DATA OVERWRITE sales FROM FILES (format = 'CSV', uris = ['sales.csv'])";
    let catalog = with_files(text, MemoryCatalog::new().with_schema(&["sales"], sales()));
    let found = errors(text, &analyze_with(text, &catalog));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, "sales");
    assert!(found[0]
        .1
        .starts_with("Table sales is defined by a later statement"));

    // Reads of the catalog, and of a table that nothing holds.
    let text = "CREATE VIEW v AS (FROM orders JOIN Customers USING (region, month, amount));
CREATE VIEW w AS (FROM missing)";
    let catalog = MemoryCatalog::new()
        .with_schema(&["orders"], sales())
        .with_schema(&["customers"], sales());
    let analysis = analyze_with(text, &catalog);
    assert_eq!(
        errors(text, &analysis),
        [("missing", "Table not found: missing".to_string())]
    );
    let reads: Vec<(&str, Vec<String>)> = (analysis.reads.iter())
        .map(|read| (&text[read.range], read.names.clone()))
        .collect();
    assert_eq!(
        reads,
        [
            ("orders", names("orders")),
            ("Customers", names("Customers")),
            ("missing", names("missing")),
        ]
    );
    // A view whose query has an error has no plan, and its reads add no errors.
    let DefinitionKind::View { plan } = &analysis.definitions[1].kind else {
        panic!();
    };
    assert!(plan.is_none());

    // A table whose schema is unknown has the columns that its syntax gives.
    let text = "CREATE EXTERNAL TABLE e (a INT64) OPTIONS (format = 'CSV', uris = ['e.csv']);
CREATE VIEW x AS (FROM e |> WHERE a > 1)";
    let found = errors(text, &analyze_with(text, &MemoryCatalog::new()));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "['e.csv']");
}

#[test]
fn a_name_is_defined_once_without_or_replace() {
    let (at, message) = error(
        "CREATE VIEW v AS (FROM t);
CREATE TABLE V AS (FROM t)",
    );
    assert_eq!(at, "V");
    assert!(message.starts_with("Table V is already defined by an earlier statement"));
    let (_, message) = error("CREATE SCHEMA s; CREATE SCHEMA s");
    assert!(message.starts_with("Schema s is already defined"));

    // OR REPLACE replaces, and IF NOT EXISTS keeps the earlier definition.
    let text = "CREATE VIEW v AS (FROM t);
CREATE OR REPLACE VIEW v AS (FROM t |> SELECT amount);
CREATE VIEW IF NOT EXISTS v AS (FROM t |> SELECT region)";
    let catalog = MemoryCatalog::new().with_schema(&["t"], sales());
    let analysis = analyze_with(text, &catalog);
    assert_ok(&analysis);
    assert_eq!(analysis.definitions.len(), 2);
    let exported: Vec<_> = analysis.exported().collect();
    assert_eq!(exported.len(), 1);
    assert_eq!(exported[0].schema.as_ref().unwrap().fields().len(), 1);
    assert_eq!(
        exported[0].schema.as_ref().unwrap().field(0).name(),
        "amount"
    );
}

#[test]
fn the_columns_of_a_view_take_names() {
    let text = "CREATE VIEW v AS (FROM t |> SELECT amount, amount + 1)";
    let catalog = MemoryCatalog::new().with_schema(&["t"], sales());
    let found = errors(text, &analyze_with(text, &catalog));
    assert_eq!(found.len(), 1);
    assert!(found[0].1.starts_with("Column 2 of the query has no name"));

    let text = "CREATE TABLE v AS (FROM t |> SELECT amount, region AS Amount)";
    let found = errors(text, &analyze_with(text, &catalog));
    assert!(found[0].1.starts_with("Column Amount is named twice"));
}

#[test]
fn options_are_checked_against_the_table_of_options() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "CREATE EXTERNAL TABLE t OPTIONS (fromat = 'CSV', format = 'CSV', uris = ['a.csv'])",
            "fromat",
            "Unknown option fromat. Did you mean format?",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'], color = 'red')",
            "color",
            "Unknown option color",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'], max_staleness = INTERVAL 4 HOUR)",
            "max_staleness",
            "avenger-sql doesn't support BigQuery's option max_staleness: a data script caches no metadata of its files",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'], encoding = 'ISO_8859_1')",
            "encoding",
            "avenger-sql doesn't support BigQuery's option encoding: DataFusion reads UTF-8 files only",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'], date_format = 'YYYY')",
            "date_format",
            "avenger-sql doesn't support BigQuery's option date_format: its format elements differ from the formats that DataFusion's CSV and JSON readers take",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'], skip_leading_rows = 'one')",
            "'one'",
            "Option skip_leading_rows takes INT64; found STRING",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = 'a.csv')",
            "'a.csv'",
            "Option uris takes ARRAY<STRING>; found STRING",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = [1, 2])",
            "[1, 2]",
            "Option uris takes a literal of ARRAY<STRING>; found ARRAY<INT64>",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = concat('C', 'SV'), uris = ['a.csv'])",
            "concat('C', 'SV')",
            "Option format takes a literal of STRING",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'XLS', uris = ['a.csv'])",
            "'XLS'",
            "Unknown format 'XLS': it is one of CSV, NEWLINE_DELIMITED_JSON, JSON, PARQUET, AVRO, ARROW",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'], compression = 'LZ4')",
            "'LZ4'",
            "Unknown compression 'LZ4': it is one of GZIP, BZIP2, XZ, ZSTD",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'JSON', uris = ['a.json'], field_delimiter = ';')",
            "field_delimiter",
            "Option field_delimiter applies to CSV files, not NEWLINE_DELIMITED_JSON",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'ARROW', uris = ['a.arrow'], compression = 'GZIP')",
            "compression",
            "Option compression applies to CSV and NEWLINE_DELIMITED_JSON files, not ARROW",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (uris = ['a.csv'])",
            "OPTIONS (uris = ['a.csv'])",
            "The files take a format, as in format = 'CSV'",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV')",
            "OPTIONS (format = 'CSV')",
            "The files take uris, as in uris = ['data/files/sales.csv']",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = [])",
            "[]",
            "uris takes a URI or more",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['data/*/x*.csv'])",
            "'data/*/x*.csv'",
            "A URI holds one * at most",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['s3://bucket/a.csv'])",
            "'s3://bucket/a.csv'",
            "A data script reads local files, so a URI is a path or a file: URL, and not a URL of another scheme",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'], null_marker = 'NA')",
            "'NA'",
            "avenger-sql doesn't support null_marker other than '': DataFusion's CSV reader reads an empty field as null, and no other text",
        ),
        (
            "CREATE EXTERNAL TABLE t (a INT64) OPTIONS (format = 'CSV', uris = ['a.csv'], skip_leading_rows = 2)",
            "2",
            "avenger-sql doesn't support skip_leading_rows above 1: DataFusion's CSV reader skips a header and no other rows",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'], skip_leading_rows = -1)",
            "-1",
            "skip_leading_rows is 0 or more",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'], field_delimiter = '||')",
            "'||'",
            "field_delimiter is one ASCII character",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'], quote = 'é')",
            "'é'",
            "quote is one ASCII character",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', format = 'CSV', uris = ['a.csv'])",
            "format",
            "Option format is set twice",
        ),
        (
            "CREATE EXTERNAL TABLE t (a INT64, A STRING) OPTIONS (format = 'CSV', uris = ['a.csv'])",
            "A",
            "Column A is named twice",
        ),
        (
            "LOAD DATA OVERWRITE t FROM FILES (format = 'CSV', uris = ['a.csv'], description = 'x')",
            "description",
            "description is an option of the table, which goes in the OPTIONS (...) before FROM FILES",
        ),
        (
            "LOAD DATA OVERWRITE t OPTIONS (format = 'CSV') FROM FILES (format = 'CSV', uris = ['a.csv'])",
            "format",
            "format is an option of files, which goes in the FROM FILES (...) after OPTIONS",
        ),
        (
            "CREATE VIEW v OPTIONS (uris = ['a.csv']) AS (FROM t)",
            "uris",
            "uris is an option of files, which a view doesn't read",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'], allow_jagged_rows = TRUE)",
            "allow_jagged_rows",
            "avenger-sql doesn't support BigQuery's option allow_jagged_rows: DataFusion's CSV reader reads rows of equal length",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'], max_bad_records = 10)",
            "max_bad_records",
            "avenger-sql doesn't support BigQuery's option max_bad_records: DataFusion's readers stop at a bad record",
        ),
        (
            "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'], ignore_unknown_values = 'false')",
            "ignore_unknown_values",
            "avenger-sql doesn't support BigQuery's option ignore_unknown_values: DataFusion's readers read every column of a file",
        ),
        (
            "CREATE SCHEMA s OPTIONS (labels = [])",
            "labels",
            "avenger-sql doesn't support BigQuery's option labels: a data script keeps no labels of tables",
        ),
    ];
    for (text, at, message) in cases {
        let catalog = MemoryCatalog::new().with_schema(&["t"], sales());
        let found = errors(text, &analyze_with(text, &with_files(text, catalog)));
        assert_eq!(found, [(*at, message.to_string())], "{text}");
    }
    // An option whose value is NULL is not set, and UTF-8 is the encoding that DataFusion reads.
    let text = "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'], encoding = 'UTF-8', null_marker = NULL)";
    assert_ok(&analyze(text));
    // An option of BigQuery takes the value that is what avenger-sql does without it.
    let text = "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'], allow_jagged_rows = false, ignore_unknown_values = FALSE, max_bad_records = 0, require_hive_partition_filter = false, file_set_spec_type = 'file_system_match');
CREATE VIEW v OPTIONS (require_partition_filter = false) AS (FROM t)";
    assert_ok(&analyze(text));
}

#[test]
fn uris_are_relative_paths_absolute_paths_or_urls_by_their_text() {
    let cases = [
        ("data/files/sales.csv", UriKind::Relative),
        ("./data/../sales.csv", UriKind::Relative),
        // `\` separates names too, as on Windows.
        ("data\\x.csv", UriKind::Relative),
        ("data\\..\\..\\x.csv", UriKind::AboveRoot),
        ("..\\..\\secret.csv", UriKind::AboveRoot),
        ("data/..\\../x.csv", UriKind::AboveRoot),
        ("data/../../x.csv", UriKind::AboveRoot),
        ("../x.csv", UriKind::AboveRoot),
        ("/home/me/x.csv", UriKind::Absolute),
        // A Windows path is absolute on any machine.
        ("C:/x.csv", UriKind::Absolute),
        ("c:\\x.csv", UriKind::Absolute),
        ("\\x.csv", UriKind::Absolute),
        ("file:///home/me/x.csv", UriKind::FileUrl),
        ("FILE:x.csv", UriKind::FileUrl),
        ("s3://bucket/x.csv", UriKind::Url),
        ("https://example.com/x.csv", UriKind::Url),
    ];
    for (uri, kind) in cases {
        assert_eq!(uri_kind(uri), kind, "{uri}");
    }
    // A Windows path and a path above the root are not errors of the options: a loader decides
    // whether it reads them.
    for uri in ["C:/x.csv", "data\\..\\..\\x.csv"] {
        let text = format!("CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = [r'{uri}'])");
        assert_eq!(reads(&text).len(), 1, "{uri}");
    }
}

#[test]
fn skip_leading_rows_skips_the_header_of_a_column_list() {
    let (at, message) = error(
        "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'], skip_leading_rows = 1)",
    );
    assert_eq!(at, "1");
    assert!(message.starts_with("Without a column list, the first row names the columns"));
    let header = |text: &str| reads(text)[0].files.csv.as_ref().unwrap().header;
    assert!(!header(
        "CREATE EXTERNAL TABLE t (a INT64) OPTIONS (format = 'CSV', uris = ['a.csv'])"
    ));
    assert!(!header(
        "CREATE EXTERNAL TABLE t (a INT64) OPTIONS (format = 'CSV', uris = ['a.csv'], skip_leading_rows = 0)"
    ));
    assert!(header(
        "CREATE EXTERNAL TABLE t (a INT64) OPTIONS (format = 'CSV', uris = ['a.csv'], skip_leading_rows = 1)"
    ));
}

#[test]
fn partition_columns_read_the_keys_after_the_prefix() {
    let ok = "CREATE EXTERNAL TABLE t WITH PARTITION COLUMNS
OPTIONS (format = 'CSV', uris = ['events/*'], hive_partition_uri_prefix = 'events/')";
    assert_ok(&analyze(ok));
    assert_eq!(reads(ok)[0].files.partitions, Some(Partitions::Inferred));

    let (at, message) = error(
        "CREATE EXTERNAL TABLE t (year INT64, amount FLOAT64) WITH PARTITION COLUMNS (year INT64)
OPTIONS (format = 'CSV', uris = ['events/*'], hive_partition_uri_prefix = 'events/')",
    );
    assert_eq!(at, "year");
    assert_eq!(
        message,
        "Column year is both a column of the files and a partition column"
    );
    let (at, message) =
        error("LOAD DATA INTO events FROM FILES (format = 'CSV', uris = ['events/*']) WITH PARTITION COLUMNS");
    assert_eq!(at, "WITH PARTITION COLUMNS");
    assert!(message.starts_with("WITH PARTITION COLUMNS reads the keys of the paths after"));
    let (at, message) = error(
        "CREATE EXTERNAL TABLE t
OPTIONS (format = 'CSV', uris = ['events/*'], hive_partition_uri_prefix = 'events/')",
    );
    assert_eq!(at, "'events/'");
    assert!(message.starts_with("hive_partition_uri_prefix goes with WITH PARTITION COLUMNS"));
    let (at, message) = error(
        "CREATE EXTERNAL TABLE t WITH PARTITION COLUMNS
OPTIONS (format = 'CSV', uris = ['other/*'], hive_partition_uri_prefix = 'events/')",
    );
    assert_eq!(at, "'other/*'");
    assert!(message.starts_with("The URI doesn't start with the hive_partition_uri_prefix"));
    // An empty prefix, or a separator alone, names no directory of a Hive layout.
    for prefix in ["''", "'/'", r"'\\'"] {
        let text = format!(
            "CREATE EXTERNAL TABLE t WITH PARTITION COLUMNS
OPTIONS (format = 'CSV', uris = ['events/*'], hive_partition_uri_prefix = {prefix})"
        );
        let (at, message) = error(&text);
        assert_eq!(at, prefix);
        assert!(
            message.starts_with("hive_partition_uri_prefix is the path of the directory"),
            "{message}"
        );
    }
}

#[test]
fn a_star_matches_the_names_of_one_directory() {
    for uri in ["data/*/x.csv", r"data\*\x.csv", "*/x.csv"] {
        let text = format!("CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = [r'{uri}'])");
        let (at, message) = error(&text);
        assert_eq!(at, format!("r'{uri}'"));
        assert_eq!(
            message,
            r"A * matches the names of the files of one directory, so no / or \ follows it"
        );
    }
    for uri in ["data/x*.csv", r"data\x*", "*.csv"] {
        let text = format!("CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = [r'{uri}'])");
        assert_eq!(reads(&text).len(), 1, "{uri}");
    }
}

#[test]
fn the_catalog_gives_the_tables_of_files() {
    // A catalog that reads no files.
    let text = "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['a.csv'])";
    let analysis = analyze_with(text, &MemoryCatalog::new());
    assert_eq!(
        errors(text, &analysis),
        [(
            "['a.csv']",
            "The catalog of this analysis reads no files, so the files have no schema".to_string()
        )]
    );
    assert_eq!(analysis.definitions[0].schema, None);

    // The error of reading the files.
    let files = reads(text).remove(0).files;
    let catalog = MemoryCatalog::new().with_files_error(files, "No file matches a.csv");
    let analysis = analyze_with(text, &catalog);
    assert_eq!(
        errors(text, &analysis),
        [("['a.csv']", "No file matches a.csv".to_string())]
    );
}

#[test]
fn a_memory_catalog_holds_schemas_and_definitions() {
    let definition = TableDefinition {
        file: "data/sources.sql".to_string(),
        range: TextRange::new(0.into(), 10.into()),
        description: Some("Sales".to_string()),
    };
    let catalog = MemoryCatalog::new()
        .with_schema(&["finance", "orders"], sales())
        .with_namespace(&["Empty"])
        .with_definition(&["Finance", "Orders"], definition.clone());
    assert!(catalog.has_schema(&names("FINANCE")));
    assert!(catalog.has_schema(&names("empty")));
    assert!(!catalog.has_schema(&names("orders")));
    assert_eq!(
        catalog.definition(&names("finance.orders")),
        Some(definition)
    );
    assert_eq!(catalog.definition(&names("orders")), None);
}

#[test]
fn functions_and_modules_serve_the_statements_of_a_data_script() {
    let text = "IMPORT MODULE units;
CREATE TEMP FUNCTION k(x FLOAT64) AS (x / 1000);
LOAD DATA OVERWRITE sales FROM FILES (format = 'CSV', uris = ['a.csv']);
CREATE VIEW v AS (FROM sales |> SELECT k(amount) AS k, units.twice(month) AS m)";
    let catalog = with_files(
        text,
        MemoryCatalog::new().with_module(
            &["units"],
            "MODULE units; CREATE PUBLIC FUNCTION twice(x INT64) AS (x * 2);",
        ),
    );
    let analysis = analyze_with(text, &catalog);
    assert_ok(&analysis);
    let schema = analysis.definitions[1].schema.as_ref().unwrap();
    assert_eq!(schema.field(0).data_type(), &DataType::Float64);
    assert_eq!(schema.field(1).data_type(), &DataType::Int64);
}

#[cfg(feature = "parquet")]
#[test]
fn parquet_reads_with_the_parquet_feature() {
    assert_ok(&analyze(
        "CREATE EXTERNAL TABLE t OPTIONS (format = 'PARQUET', uris = ['a.parquet'])",
    ));
}

#[cfg(not(feature = "parquet"))]
#[test]
fn parquet_is_an_error_without_the_parquet_feature() {
    let (at, message) =
        error("CREATE EXTERNAL TABLE t OPTIONS (format = 'parquet', uris = ['a.parquet'])");
    assert_eq!(at, "'parquet'");
    assert_eq!(
        message,
        "This build of avenger-sql reads no PARQUET files: it lacks the cargo feature parquet"
    );
}

#[cfg(feature = "avro")]
#[test]
fn avro_reads_with_the_avro_feature() {
    assert_ok(&analyze(
        "CREATE EXTERNAL TABLE t OPTIONS (format = 'AVRO', uris = ['a.avro'])",
    ));
}

#[cfg(not(feature = "avro"))]
#[test]
fn avro_is_an_error_without_the_avro_feature() {
    let (_, message) =
        error("CREATE EXTERNAL TABLE t OPTIONS (format = 'AVRO', uris = ['a.avro'])");
    assert_eq!(
        message,
        "This build of avenger-sql reads no AVRO files: it lacks the cargo feature avro"
    );
}

#[cfg(feature = "compression")]
#[test]
fn compressed_files_read_with_the_compression_feature() {
    let text = "LOAD DATA OVERWRITE t FROM FILES (format = 'CSV', uris = ['a.csv.zst'], compression = 'zstd')";
    assert_ok(&analyze(text));
    assert_eq!(
        reads(text)[0].files.compression,
        Some(avenger_sql::data::Compression::Zstd)
    );
}

#[cfg(not(feature = "compression"))]
#[test]
fn compressed_files_are_an_error_without_the_compression_feature() {
    let (at, message) = error(
        "LOAD DATA OVERWRITE t FROM FILES (format = 'CSV', uris = ['a.csv.gz'], compression = 'GZIP')",
    );
    assert_eq!(at, "'GZIP'");
    assert_eq!(
        message,
        "This build of avenger-sql reads no GZIP files: it lacks the cargo feature compression"
    );
}

#[test]
fn a_statement_with_syntax_errors_defines_its_table() {
    // `OPTIONS` is missing, and then `FROM FILES`, which the parse reports.
    let text = "CREATE EXTERNAL TABLE e (a INT64);
LOAD DATA OVERWRITE l (b INT64);
CREATE VIEW v AS (FROM e JOIN l ON a = b)";
    let parse = parse_data(text);
    assert!(!parse.ok());
    assert!(file_reads(&parse.syntax(), TypeOptions::default()).is_empty());
    let analysis = analyze_data(
        &parse.syntax(),
        &MemoryCatalog::new(),
        library(),
        &Options::default(),
    );
    assert_ok(&analysis);
    let paths: Vec<String> = analysis
        .definitions
        .iter()
        .map(|d| d.names.join("."))
        .collect();
    assert_eq!(paths, ["e", "l", "v"]);
    assert!(analysis.reads.is_empty());
    assert!(analysis
        .definitions
        .iter()
        .all(|definition| definition.schema.is_none()));
}

#[test]
fn a_temporary_load_hides_a_table_of_the_catalog() {
    let text = "LOAD DATA INTO TEMP TABLE t FROM FILES (format = 'CSV', uris = ['a.csv']);
CREATE VIEW v AS (FROM t)";
    let analysis = analyze(text);
    assert_ok(&analysis);
    assert!(analysis.reads.is_empty());
}

#[test]
fn a_module_reads_the_tables_of_the_catalog_and_not_those_of_the_script() {
    let text = "IMPORT MODULE m;
CREATE TEMP EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['sales.csv']);
CREATE VIEW v AS (FROM m.recent(5))";
    let catalog = with_files(
        text,
        MemoryCatalog::new()
            .with_schema(&["sales"], sales())
            .with_module(
            &["m"],
            "MODULE m; CREATE PUBLIC TABLE FUNCTION recent(n INT64) AS (FROM sales |> LIMIT n);",
        ),
    );
    let analysis = analyze_with(text, &catalog);
    assert_ok(&analysis);
    // The import checks the body against the catalog, though a later statement defines sales,
    // and the call reads the catalog's sales, not the script's.
    let reads: Vec<(&str, Vec<String>)> = (analysis.reads.iter())
        .map(|read| (&text[read.range], read.names.clone()))
        .collect();
    assert_eq!(
        reads,
        [("m", names("sales")), ("m.recent(5)", names("sales"))]
    );
    let DefinitionKind::View { plan: Some(plan) } = &analysis.definitions[1].kind else {
        panic!("{:?}", analysis.definitions[1].kind);
    };
    assert_eq!(placeholders(plan), []);
}

#[test]
fn reads_in_the_bodies_of_functions_are_at_their_calls() {
    let text = "IMPORT MODULE m;
CREATE VIEW v AS (FROM m.everything())";
    let catalog = MemoryCatalog::new()
        .with_schema(&["t"], sales())
        .with_module(
            &["m"],
            "MODULE m; CREATE PUBLIC TABLE FUNCTION everything() AS (FROM t);",
        );
    let analysis = analyze_with(text, &catalog);
    assert_ok(&analysis);
    let reads: Vec<(&str, Vec<String>)> = (analysis.reads.iter())
        .map(|read| (&text[read.range], read.names.clone()))
        .collect();
    // The import checks the body of the function, and the call plans it.
    assert_eq!(reads, [("m", names("t")), ("m.everything()", names("t"))]);
}
