//! The requests on data scripts: diagnostics, highlighting, hover, completion, the outline, and
//! formatting.

mod common;

use std::sync::Arc;

use avenger_sql::{
    analysis::Options,
    catalog::{MemoryCatalog, TableDefinition},
    data::{file_reads, Compression, Format},
    library::Library,
    types::TypeOptions,
};
use avenger_sql_language::{
    CompletionKind, Completions, Document, Highlight, Role, Source, SymbolKind, TextRange,
};
use avenger_sql_syntax::{parse_data, NameRole};
use common::{at, find};
use datafusion::arrow::datatypes::{DataType, Field, Schema};

/// The paths of the project of the tests.
const PATHS: &[&str] = &[
    "README.md",
    "data/files/2025/january.csv",
    "data/files/cars.json",
    "data/files/sales.csv",
    "data/files/sales.csv.gz",
    "data/sales.sql",
];

/// The catalog of a data script `text`: the table `penguins` of the host, with the place that
/// defines it, the schema `finance` of another script, and the columns `id` and `amount` for the
/// files of each statement of `text` that reads files.
fn catalog(text: &str) -> MemoryCatalog {
    let penguins = Schema::new(vec![Field::new("species", DataType::Utf8, true)]);
    let mut catalog = MemoryCatalog::new()
        .with_schema(&["penguins"], Arc::new(penguins))
        .with_definition(
            &["penguins"],
            TableDefinition {
                file: "data/penguins.sql".to_string(),
                range: TextRange::default(),
                description: Some("The penguins of the Palmer archipelago".to_string()),
            },
        )
        .with_namespace(&["finance"])
        .with_definition(
            &["finance"],
            TableDefinition {
                file: "data/finance.sql".to_string(),
                range: TextRange::default(),
                description: Some("The books".to_string()),
            },
        );
    let files = Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new("amount", DataType::Float64, true),
    ]);
    for read in file_reads(&parse_data(text).syntax(), TypeOptions::default()) {
        catalog = catalog.with_files_schema(read.files, Arc::new(files.clone()));
    }
    catalog
}

/// The document of the data script `text`, in the project of [`PATHS`].
fn data(text: &str) -> Document {
    let paths: Vec<String> = PATHS.iter().map(|path| path.to_string()).collect();
    Document::new(
        text,
        Role::Data,
        Arc::new(catalog(text)),
        Arc::new(Library::new()),
        Options::default(),
    )
    .with_paths(paths.into())
}

/// The completions of the data script `text` at its cursor marker `$0`, and the text that they
/// replace.
fn complete(text: &str) -> (Completions, String, String) {
    let offset = text.find("$0").expect("a cursor marker `$0`");
    let document = data(&text.replacen("$0", "", 1));
    let completions = document.completions((offset as u32).into());
    let range = at(&document, completions.range).to_string();
    let replace = at(&document, completions.replace).to_string();
    (completions, range, replace)
}

fn labels(completions: &Completions, kind: CompletionKind) -> Vec<&str> {
    (completions.items.iter())
        .filter(|item| item.kind == kind)
        .map(|item| item.label.as_str())
        .collect()
}

/// The hover at the `n`th `needle`, with the text of its range.
fn hover(document: &Document, needle: &str, n: usize) -> (String, String) {
    let hover = (document.hover(find(document, needle, n)))
        .unwrap_or_else(|| panic!("no hover at {needle}"));
    (at(document, hover.range).to_string(), hover.markdown)
}

const SCRIPT: &str = "CREATE SCHEMA sales OPTIONS (description = 'The sales');
CREATE EXTERNAL TABLE sales.orders
OPTIONS (format = 'CSV', uris = ['data/files/sales.csv'], description = 'The orders');
LOAD DATA OVERWRITE TEMP TABLE raw FROM FILES (format = 'JSON', uris = ['data/files/cars.json']);
LOAD DATA INTO TEMP TABLE raw FROM FILES (format = 'CSV', uris = ['data/files/2025/*.csv']);
CREATE VIEW big AS (FROM sales.orders |> WHERE amount > 100);
CREATE TABLE totals AS (FROM raw |> AGGREGATE SUM(amount) AS total);
CREATE VIEW finance.recent AS (FROM penguins);";

#[test]
fn a_data_script_analyzes_as_a_data_script() {
    let document = data(SCRIPT);
    assert_eq!(document.role(), Role::Data);
    assert!(
        document.diagnostics().is_empty(),
        "{:?}",
        document.diagnostics()
    );
    assert!(document.analysis().is_none());
    let analysis = document.data_analysis().unwrap();
    let names: Vec<String> = (analysis.exported())
        .map(|definition| definition.names.join("."))
        .collect();
    assert_eq!(
        names,
        ["sales", "sales.orders", "big", "totals", "finance.recent"]
    );
}

#[test]
fn the_diagnostics_of_a_data_script_come_from_its_parse_and_its_analysis() {
    let text = "FROM penguins;
CREATE EXTERNAL TABLE t OPTIONS (forma = 'CSV', uris = ['data/files/sales.csv']);
CREATE VIEW v AS (FROM missing);";
    let diagnostics = data(text).diagnostics();
    let syntax = (diagnostics.iter())
        .find(|d| d.source == Source::Syntax)
        .unwrap();
    assert!(
        syntax
            .help
            .as_deref()
            .is_some_and(|help| help.contains("chart script")),
        "{syntax:?}"
    );
    let messages: Vec<&str> = (diagnostics.iter())
        .filter(|d| d.source == Source::Analysis)
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        messages.contains(&"Unknown option forma. Did you mean format?"),
        "{messages:?}"
    );
    assert!(
        messages.contains(&"Table not found: missing"),
        "{messages:?}"
    );
    // The same text as a chart script has the errors of a chart script.
    let script = common::document(text);
    assert!(script.data_analysis().is_none());
    assert!(
        (script.diagnostics().iter()).any(|d| d.message.contains("data script")
            || d.help
                .as_deref()
                .is_some_and(|help| help.contains("data script")))
    );
}

#[test]
fn a_module_has_the_requests_of_its_syntax() {
    let text = "MODULE m;\nCREATE PUBLIC FUNCTION twice(x INT64) AS (x * 2);";
    let document = Document::new(
        text,
        Role::Module,
        Arc::new(MemoryCatalog::new()),
        Arc::new(Library::new()),
        Options::default(),
    );
    assert!(
        document.diagnostics().is_empty(),
        "{:?}",
        document.diagnostics()
    );
    assert!(document.analysis().is_none() && document.data_analysis().is_none());
    assert!(!document.highlights().is_empty());
}

#[test]
fn the_names_that_statements_define_have_their_classes() {
    let document = data(SCRIPT);
    let class = |needle: &str, n: usize| {
        let start = find(&document, needle, n);
        (document.highlights().into_iter())
            .find(|range| range.range.start() == start)
            .map(|range| (range.highlight, range.definition))
            .unwrap_or_else(|| panic!("no class at {needle}"))
    };
    // The names that the statements define keep the classes of tables and schemas, beside the
    // classes of formats and URIs, which avenger-sql-syntax tests.
    assert_eq!(class("sales", 0), (Highlight::Name(NameRole::Schema), true));
    assert_eq!(class("orders", 0), (Highlight::Name(NameRole::Table), true));
    assert_eq!(class("raw", 0), (Highlight::Name(NameRole::Table), true));
    assert_eq!(class("big", 0), (Highlight::Name(NameRole::Table), true));
}

#[test]
fn hover_shows_an_option_its_type_formats_and_meaning() {
    let text = "LOAD DATA OVERWRITE t FROM FILES (format = 'CSV', uris = ['data/files/sales.csv'], skip_leading_rows = 0, max_staleness = 1)";
    let document = data(text);
    let (range, markdown) = hover(&document, "skip_leading_rows", 0);
    assert_eq!(range, "skip_leading_rows");
    assert_eq!(
        markdown,
        "```googlesql\nskip_leading_rows: INT64\n```\n\nThe rows before the data: 1 for a header that a column list names in its place.\n\nAn option of CSV files."
    );
    let (_, markdown) = hover(&document, "format", 0);
    assert!(
        markdown.starts_with("```googlesql\nformat: STRING\n```"),
        "{markdown}"
    );
    assert!(
        markdown.contains("every statement that reads files sets"),
        "{markdown}"
    );
    assert!(markdown.contains("Values: 'CSV', "), "{markdown}");
    let (_, markdown) = hover(&document, "max_staleness", 0);
    assert_eq!(
        markdown,
        "```googlesql\nmax_staleness\n```\n\nAn option of BigQuery that avenger-sql doesn't support: a data script caches no metadata of its files."
    );
    // An option that takes the value that is what avenger-sql does, as GoogleSQL writes it.
    let text =
        "LOAD DATA OVERWRITE t FROM FILES (format = 'CSV', uris = ['a.csv'], max_bad_records = 0)";
    let (_, markdown) = hover(&data(text), "max_bad_records", 0);
    assert!(
        markdown.ends_with("\n\nIt takes 0, which is what avenger-sql does without it."),
        "{markdown}"
    );
}

#[test]
fn hover_shows_a_table_its_files_description_and_columns() {
    let document = data(SCRIPT);
    // A table of the script, at its definition and at a read of it.
    let (range, markdown) = hover(&document, "orders", 0);
    assert_eq!(range, "sales.orders");
    let expected = "```googlesql\nEXTERNAL TABLE sales.orders\n```\n\nThe orders\n\nReads CSV files: `data/files/sales.csv`\n\n- `id`: INT64\n- `amount`: FLOAT64";
    assert_eq!(markdown, expected);
    let (range, markdown) = hover(&document, "sales.orders |>", 0);
    assert_eq!(range, "sales.orders");
    assert_eq!(markdown, expected);
    // A table that two loads fill shows the files of each.
    let (_, markdown) = hover(&document, "raw", 2);
    assert_eq!(
        markdown,
        "```googlesql\nTEMP TABLE raw\n```\n\nLoads NEWLINE_DELIMITED_JSON files: `data/files/cars.json`\n\nLoads CSV files: `data/files/2025/*.csv`\n\n- `id`: INT64\n- `amount`: FLOAT64"
    );
    let (_, markdown) = hover(&document, "big", 0);
    assert!(
        markdown.starts_with("```googlesql\nVIEW big\n```"),
        "{markdown}"
    );
    // A table of the catalog, with the place that defines it.
    let (_, markdown) = hover(&document, "penguins", 0);
    assert_eq!(
        markdown,
        "```googlesql\nTABLE penguins\n```\n\nThe penguins of the Palmer archipelago\n\nDefined in `data/penguins.sql`\n\n- `species`: STRING"
    );
}

#[test]
fn hover_shows_a_schema_as_a_dataset() {
    let document = data(SCRIPT);
    let (range, markdown) = hover(&document, "sales", 0);
    assert_eq!(range, "sales");
    assert_eq!(
        markdown,
        "```googlesql\nSCHEMA sales\n```\n\nA schema of tables, which BigQuery calls a dataset.\n\nThe sales"
    );
    // The schema of a table's path, which this script creates or the catalog holds.
    let (range, markdown) = hover(&document, "sales.orders", 0);
    assert_eq!(range, "sales");
    assert!(markdown.ends_with("\n\nThe sales"), "{markdown}");
    let (range, markdown) = hover(&document, "finance", 0);
    assert_eq!(range, "finance");
    assert_eq!(
        markdown,
        "```googlesql\nSCHEMA finance\n```\n\nA schema of tables, which BigQuery calls a dataset.\n\nThe books\n\nDefined in `data/finance.sql`"
    );
}

#[test]
fn a_data_script_completes_its_statements() {
    let (completions, _, _) = complete("$0");
    let keywords = labels(&completions, CompletionKind::Keyword);
    for keyword in [
        "CREATE EXTERNAL TABLE",
        "LOAD DATA",
        "CREATE VIEW",
        "CREATE SCHEMA",
    ] {
        assert!(keywords.contains(&keyword), "{keyword}: {keywords:?}");
    }
    assert!(!keywords.contains(&"DECLARE") && !keywords.contains(&"FROM"));
    let (completions, _, _) = complete("CREATE SCHEMA s;\nCREATE $0");
    let keywords = labels(&completions, CompletionKind::Keyword);
    for keyword in ["EXTERNAL TABLE", "VIEW", "SCHEMA", "TABLE", "TEMP"] {
        assert!(keywords.contains(&keyword), "{keyword}: {keywords:?}");
    }
    // A chart script creates no views.
    let (document, offset) = common::at_cursor("CREATE $0");
    let keywords: Vec<String> = (document.completions(offset).items.into_iter())
        .filter(|item| item.kind == CompletionKind::Keyword)
        .map(|item| item.label)
        .collect();
    assert!(
        !keywords.iter().any(|keyword| keyword == "VIEW"),
        "{keywords:?}"
    );
}

#[test]
fn options_complete_by_the_format_of_their_statement() {
    let (completions, _, _) = complete("CREATE EXTERNAL TABLE t OPTIONS ($0)");
    let options = labels(&completions, CompletionKind::Option);
    for option in [
        "format",
        "uris",
        "skip_leading_rows",
        "compression",
        "description",
    ] {
        assert!(options.contains(&option), "{option}: {options:?}");
    }
    assert_eq!(
        completions.items.len(),
        options.len(),
        "{:?}",
        completions.items
    );
    let format = (completions.items.iter())
        .find(|item| item.label == "format")
        .unwrap();
    assert_eq!(format.insert, "format = ");
    assert_eq!(format.detail.as_deref(), Some("STRING"));
    assert!(format
        .documentation
        .as_deref()
        .unwrap()
        .starts_with("The format of the files"));
    // The format of the statement keeps the options that apply to it, without those set.
    let (completions, range, _) =
        complete("CREATE EXTERNAL TABLE t OPTIONS (format = 'JSON', com$0)");
    assert_eq!(range, "com");
    let options = labels(&completions, CompletionKind::Option);
    assert!(
        options.contains(&"compression") && options.contains(&"uris"),
        "{options:?}"
    );
    assert!(!options.contains(&"skip_leading_rows") && !options.contains(&"format"));
    // `FROM FILES` takes the options of files, and the `OPTIONS` of a `LOAD DATA` those of a
    // table.
    let (completions, _, _) =
        complete("LOAD DATA INTO t OPTIONS ($0) FROM FILES (format = 'CSV', uris = ['a.csv'])");
    assert_eq!(
        labels(&completions, CompletionKind::Option),
        ["description"]
    );
    let (completions, _, _) = complete("LOAD DATA INTO t FROM FILES (format = 'CSV', $0");
    let options = labels(&completions, CompletionKind::Option);
    assert!(options.contains(&"skip_leading_rows") && !options.contains(&"description"));
    let (completions, _, _) = complete("CREATE VIEW v OPTIONS ($0) AS (FROM penguins)");
    assert_eq!(
        labels(&completions, CompletionKind::Option),
        ["description"]
    );
}

#[test]
fn the_values_of_format_and_compression_complete() {
    let (completions, _, _) = complete("LOAD DATA INTO t FROM FILES (format = $0");
    let values = labels(&completions, CompletionKind::EnumValue);
    for value in ["'CSV'", "'NEWLINE_DELIMITED_JSON'", "'JSON'", "'ARROW'"] {
        assert!(values.contains(&value), "{value}: {values:?}");
    }
    // The formats that this build reads.
    assert_eq!(values.contains(&"'PARQUET'"), Format::Parquet.available());
    assert_eq!(
        values.len(),
        completions.items.len(),
        "{:?}",
        completions.items
    );
    // In the string, the values replace it.
    let (completions, range, _) = complete("LOAD DATA INTO t FROM FILES (format = 'C$0'");
    assert_eq!(range, "'C'");
    assert!(labels(&completions, CompletionKind::EnumValue).contains(&"'CSV'"));
    let (completions, _, _) =
        complete("CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', compression = $0)");
    let values = labels(&completions, CompletionKind::EnumValue);
    match Compression::Gzip.available() {
        true => assert_eq!(values, ["'BZIP2'", "'GZIP'", "'XZ'", "'ZSTD'"]),
        false => assert!(values.is_empty(), "{values:?}"),
    }
    let (completions, _, _) =
        complete("CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', allow_quoted_newlines = $0)");
    assert_eq!(
        labels(&completions, CompletionKind::Keyword),
        ["FALSE", "TRUE"]
    );
}

#[test]
fn a_uri_completes_the_paths_of_the_project() {
    let (completions, range, replace) =
        complete("LOAD DATA INTO t FROM FILES (format = 'CSV', uris = ['$0'])");
    assert_eq!((range.as_str(), replace.as_str()), ("", ""));
    assert_eq!(labels(&completions, CompletionKind::Folder), ["data/"]);
    // A URI of one file reads it whatever its name, so the files of other names complete too.
    assert_eq!(labels(&completions, CompletionKind::File), ["README.md"]);
    // One segment past what is typed: the files that a directory of the format reads first, and
    // a compressed file only with its compression.
    let (completions, range, replace) =
        complete("LOAD DATA INTO t FROM FILES (format = 'CSV', uris = ['data/files/$0x'])");
    assert_eq!(
        (range.as_str(), replace.as_str()),
        ("data/files/", "data/files/x")
    );
    assert_eq!(
        labels(&completions, CompletionKind::Folder),
        ["data/files/2025/"]
    );
    assert_eq!(
        labels(&completions, CompletionKind::File),
        [
            "data/files/sales.csv",
            "data/files/cars.json",
            "data/files/sales.csv.gz"
        ]
    );
    let (completions, _, _) = complete(
        "LOAD DATA INTO t FROM FILES (format = 'CSV', compression = 'GZIP', uris = ['data/files/s$0'])",
    );
    assert_eq!(
        labels(&completions, CompletionKind::File),
        ["data/files/sales.csv.gz", "data/files/sales.csv"]
    );
    // Without a format, every file.
    let (completions, _, _) = complete("CREATE EXTERNAL TABLE t OPTIONS (uris = ['data/$0'])");
    assert_eq!(
        labels(&completions, CompletionKind::Folder),
        ["data/files/"]
    );
    assert_eq!(
        labels(&completions, CompletionKind::File),
        ["data/sales.sql"]
    );
    let (completions, _, _) = complete(
        "CREATE EXTERNAL TABLE t OPTIONS (format = 'JSON', uris = ['a.csv', 'data/files/c$0'])",
    );
    assert_eq!(
        labels(&completions, CompletionKind::File),
        ["data/files/cars.json"]
    );
    // An unclosed string runs to the end of its line, and an item replaces the text before the
    // cursor alone.
    let (completions, range, replace) =
        complete("LOAD DATA INTO t FROM FILES (format = 'CSV', uris = ['data/$0]);\nCREATE VIEW v AS (FROM t);");
    assert_eq!((range.as_str(), replace.as_str()), ("data/", "data/"));
    assert_eq!(
        labels(&completions, CompletionKind::Folder),
        ["data/files/"]
    );
    // An unclosed string that ends with a character of more than one byte.
    let (completions, range, replace) =
        complete("LOAD DATA INTO t FROM FILES (format = 'CSV', uris = ['data/files/café$0");
    assert_eq!(
        (range.as_str(), replace.as_str()),
        ("data/files/café", "data/files/café")
    );
    assert!(labels(&completions, CompletionKind::File).is_empty());
}

#[test]
fn a_uri_completes_through_the_escapes_of_its_string() {
    let complete = |text: &str| {
        let offset = text.find("$0").expect("a cursor marker `$0`");
        let document = data(&text.replacen("$0", "", 1)).with_paths(
            ["data/o'neil.csv", r"data/a\b.csv"]
                .map(String::from)
                .to_vec()
                .into(),
        );
        let completions = document.completions((offset as u32).into());
        let range = at(&document, completions.range).to_string();
        let inserts: Vec<String> = (completions.items.iter())
            .map(|item| item.insert.clone())
            .collect();
        (range, inserts)
    };
    // An escaped quote doesn't close the string, and the typed text matches by its value.
    let (range, inserts) = complete(r"CREATE EXTERNAL TABLE t OPTIONS (uris = ['data/o\'$0");
    assert_eq!(range, r"data/o\'");
    assert_eq!(inserts, [r"data/o\'neil.csv"]);
    // A path that holds the string's quote or a backslash goes in escaped, and in a raw string
    // as it is.
    let (_, inserts) = complete("CREATE EXTERNAL TABLE t OPTIONS (uris = ['data/$0'])");
    assert_eq!(inserts, [r"data/a\\b.csv", r"data/o\'neil.csv"]);
    let (_, inserts) = complete(r#"CREATE EXTERNAL TABLE t OPTIONS (uris = ["data/o$0"])"#);
    assert_eq!(inserts, ["data/o'neil.csv"]);
    let (_, inserts) = complete("CREATE EXTERNAL TABLE t OPTIONS (uris = [r'data/a$0'])");
    assert_eq!(inserts, [r"data/a\b.csv"]);
}

#[test]
fn an_option_name_of_an_entry_with_its_value_replaces_the_name_alone() {
    let (completions, range, _) =
        complete("CREATE EXTERNAL TABLE t OPTIONS (fo$0rm = 'CSV', uris = ['a.csv'])");
    assert_eq!(range, "fo");
    let format = (completions.items.iter())
        .find(|item| item.label == "format")
        .unwrap();
    assert_eq!(format.insert, "format");
    // A new entry takes its `=` too.
    let (completions, _, _) = complete("CREATE EXTERNAL TABLE t OPTIONS (uris = ['a.csv'], fo$0)");
    let format = (completions.items.iter())
        .find(|item| item.label == "format")
        .unwrap();
    assert_eq!(format.insert, "format = ");
}

#[test]
fn the_hive_prefix_completes_with_partition_columns() {
    let names = |text: &str| -> Vec<String> {
        let (completions, _, _) = complete(text);
        labels(&completions, CompletionKind::Option)
            .into_iter()
            .map(String::from)
            .collect()
    };
    let plain = names("CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', $0)");
    assert!(!plain.iter().any(|name| name == "hive_partition_uri_prefix"));
    let hive = names("CREATE EXTERNAL TABLE t WITH PARTITION COLUMNS OPTIONS (format = 'CSV', $0)");
    assert!(hive.iter().any(|name| name == "hive_partition_uri_prefix"));
}

#[test]
fn the_hover_of_an_option_tells_its_errors_and_the_values_of_the_build() {
    let document = data(
        "LOAD DATA INTO t OPTIONS (format = 'CSV') FROM FILES (format = 'NEWLINE_DELIMITED_JSON', uris = ['a.json'], field_delimiter = ';');",
    );
    let (_, markdown) = hover(&document, "field_delimiter", 0);
    assert!(
        markdown.contains("**Error:**") && markdown.contains("CSV"),
        "{markdown}"
    );
    let (_, markdown) = hover(&document, "format", 0);
    assert!(markdown.contains("**Error:**"), "{markdown}");
    // The second `format` is in its place, and lists the formats that this build reads.
    let (_, markdown) = hover(&document, "format", 1);
    assert!(!markdown.contains("**Error:**"), "{markdown}");
    assert_eq!(
        markdown.contains("'PARQUET'"),
        Format::Parquet.available(),
        "{markdown}"
    );
}

#[test]
fn the_outline_names_a_path_by_its_names() {
    let symbols = data("CREATE SCHEMA fin;\nCREATE VIEW fin . /* c */ v AS (FROM t);").symbols();
    let names: Vec<&str> = (symbols.iter())
        .map(|symbol| symbol.name.as_str())
        .collect();
    assert_eq!(names, ["fin", "fin.v"]);
}

#[test]
fn a_query_of_a_data_script_completes_its_tables() {
    let (completions, _, _) = complete(
        "CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/files/sales.csv']);
LOAD DATA OVERWRITE TEMP TABLE raw FROM FILES (format = 'CSV', uris = ['data/files/sales.csv']);
CREATE VIEW v AS (FROM $0);
CREATE VIEW later AS (FROM penguins);",
    );
    let tables = labels(&completions, CompletionKind::Table);
    assert!(
        tables.contains(&"sales") && tables.contains(&"raw") && tables.contains(&"penguins"),
        "{tables:?}"
    );
    assert!(!tables.contains(&"later"), "{tables:?}");
    let sales = (completions.items.iter())
        .find(|item| item.label == "sales")
        .unwrap();
    assert_eq!(sales.detail.as_deref(), Some("2 columns"));
}

#[test]
fn the_outline_holds_the_tables_views_and_schemas_of_a_data_script() {
    let symbols: Vec<(String, SymbolKind, Option<String>, usize)> = (data(SCRIPT).symbols())
        .into_iter()
        .map(|symbol| {
            (
                symbol.name,
                symbol.kind,
                symbol.detail,
                symbol.children.len(),
            )
        })
        .collect();
    let symbol = |name: &str, kind, detail: &str, children| {
        (name.to_string(), kind, Some(detail.to_string()), children)
    };
    assert_eq!(
        symbols,
        [
            symbol("sales", SymbolKind::Schema, "SCHEMA", 0),
            symbol("sales.orders", SymbolKind::Table, "EXTERNAL TABLE", 0),
            symbol(
                "raw",
                SymbolKind::Table,
                "LOAD DATA OVERWRITE TEMP TABLE",
                0
            ),
            symbol("raw", SymbolKind::Table, "LOAD DATA INTO TEMP TABLE", 0),
            symbol("big", SymbolKind::View, "VIEW", 1),
            symbol("totals", SymbolKind::Table, "TABLE", 1),
            symbol("finance.recent", SymbolKind::View, "VIEW", 0),
        ]
    );
    let temp = data("CREATE TEMP VIEW v AS (FROM penguins);").symbols();
    assert_eq!(temp[0].detail.as_deref(), Some("TEMP VIEW"));
    // A chart script that holds a statement of a data script has an error there, and no symbol.
    assert!(common::document("CREATE VIEW v AS (FROM penguins);")
        .symbols()
        .is_empty());
}

#[test]
fn formatting_follows_the_role_of_the_text() {
    let text = "create external table t options(format='CSV',uris=['a.csv']);\nload data into u from files(format='CSV',uris=['b.csv']);\n";
    let document = data(text);
    let formatted = avenger_sql_syntax::format::format(text, Role::Data);
    assert_ne!(formatted, text);
    let edits = document.formatting();
    assert_eq!(edits.len(), 1);
    let mut result = text.to_string();
    let range = std::ops::Range::<usize>::from(edits[0].range);
    result.replace_range(range, &edits[0].replacement);
    assert_eq!(result, formatted);
    assert!(data(&formatted).formatting().is_empty());
}

#[test]
fn a_value_of_an_option_takes_its_values_alone() {
    for text in [
        "LOAD DATA INTO t FROM FILES (format = 'CSV', uris = [$0",
        "LOAD DATA INTO t FROM FILES (format = 'CSV', uris = ['a.csv', $0",
        "LOAD DATA INTO t FROM FILES (forma = $0",
        "LOAD DATA INTO t FROM FILES (max_staleness = $0",
        "CREATE EXTERNAL TABLE t OPTIONS (skip_leading_rows = $0)",
    ] {
        let (completions, _, _) = complete(text);
        assert!(
            completions.items.is_empty(),
            "{text}: {:?}",
            completions.items
        );
    }
}

#[test]
fn a_module_completes_its_statements() {
    let text = "MODULE m;\n";
    let document = Document::new(
        text,
        Role::Module,
        Arc::new(MemoryCatalog::new()),
        Arc::new(Library::new()),
        Options::default(),
    );
    let keywords: Vec<String> = (document.completions((text.len() as u32).into()).items)
        .into_iter()
        .filter(|item| item.kind == CompletionKind::Keyword)
        .map(|item| item.label)
        .collect();
    assert!(
        !keywords.iter().any(|keyword| keyword == "FROM"),
        "{keywords:?}"
    );
    // Each statement that completion offers starts a statement of a module.
    for keyword in &keywords {
        let rest = match keyword.as_str() {
            "IMPORT MODULE" => "other",
            keyword if keyword.ends_with("FUNCTION") => "f() AS (1)",
            keyword if keyword.ends_with("CONSTANT") => "c = 1",
            _ => continue,
        };
        let module = format!("{text}{keyword} {rest};");
        let parse = avenger_sql_syntax::parse_module(&module);
        assert!(
            parse.diagnostics().is_empty(),
            "{module}: {:?}",
            parse.diagnostics()
        );
    }
    assert!(
        keywords
            .iter()
            .any(|keyword| keyword == "CREATE PUBLIC FUNCTION"),
        "{keywords:?}"
    );
}
