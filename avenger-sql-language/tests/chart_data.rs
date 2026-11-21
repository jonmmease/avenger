//! The requests on a chart script's own tables of files, its `CREATE TEMP EXTERNAL TABLE` and
//! `LOAD DATA ... TEMP TABLE` statements: diagnostics, highlighting, hover, completion, and the
//! outline, as in a data script.

mod common;

use std::sync::Arc;

use avenger_sql::{
    analysis::Options, catalog::MemoryCatalog, data::file_reads, library::Library,
    types::TypeOptions,
};
use avenger_sql_language::{CompletionKind, Completions, Document, Highlight, Role, SymbolKind};
use avenger_sql_syntax::parse;
use common::{at, find};
use datafusion::arrow::datatypes::{DataType, Field, Schema};

/// The paths of the project of the tests.
const PATHS: &[&str] = &[
    "data/files/sales.csv",
    "data/files/cars.json",
    "charts/a.sql",
];

/// The document of the chart script `text`, in the project of [`PATHS`], whose catalog gives the
/// columns `region` and `amount` for the files of each of its statements of files.
fn chart(text: &str) -> Document {
    let files = Schema::new(vec![
        Field::new("region", DataType::Utf8, true),
        Field::new("amount", DataType::Float64, true),
    ]);
    let reads = file_reads(&parse(text).syntax(), TypeOptions::default());
    let catalog = (reads.into_iter()).fold(MemoryCatalog::new(), |catalog, read| {
        catalog.with_files_schema(read.files, Arc::new(files.clone()))
    });
    let paths: Vec<String> = PATHS.iter().map(|path| path.to_string()).collect();
    Document::new(
        text,
        Role::Script,
        Arc::new(catalog),
        Arc::new(Library::new()),
        Options::default(),
    )
    .with_paths(paths.into())
}

/// The completions of the chart script `text` at its cursor marker `$0`.
fn complete(text: &str) -> Completions {
    let offset = text.find("$0").expect("a cursor marker `$0`");
    chart(&text.replacen("$0", "", 1)).completions((offset as u32).into())
}

fn labels(completions: &Completions, kind: CompletionKind) -> Vec<&str> {
    (completions.items.iter())
        .filter(|item| item.kind == kind)
        .map(|item| item.label.as_str())
        .collect()
}

const CHART: &str = "CREATE TEMP EXTERNAL TABLE sales
OPTIONS (format = 'CSV', uris = ['data/files/sales.csv'], description = 'The sales');
LOAD DATA OVERWRITE TEMP TABLE cars FROM FILES (format = 'JSON', uris = ['data/files/cars.json']);
FROM sales JOIN cars USING (region)
|> AGGREGATE mark_bar(x => region, y => SUM(sales.amount)) AS chart";

#[test]
fn a_chart_with_its_own_tables_of_files_has_no_errors() {
    let document = chart(CHART);
    assert!(
        document.diagnostics().is_empty(),
        "{:?}",
        document.diagnostics()
    );
    // A statement without TEMP has the error of the syntax alone.
    let text =
        "CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/files/sales.csv'])";
    let diagnostics = chart(text).diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].message.contains("without TEMP"));
}

#[test]
fn formats_and_uris_of_a_chart_have_their_classes() {
    let document = chart(CHART);
    let class = |needle: &str| {
        let start = find(&document, needle, 0);
        (document.highlights().into_iter())
            .find(|range| range.range.start() == start)
            .map(|range| range.highlight)
            .unwrap_or_else(|| panic!("no class at {needle}"))
    };
    assert_eq!(class("'CSV'"), Highlight::Format);
    assert_eq!(class("'data/files/cars.json'"), Highlight::Uri);
}

#[test]
fn hover_shows_the_options_and_the_tables_of_a_chart() {
    let document = chart(CHART);
    let hover = |needle: &str| {
        let hover = (document.hover(find(&document, needle, 0)))
            .unwrap_or_else(|| panic!("no hover at {needle}"));
        (at(&document, hover.range).to_string(), hover.markdown)
    };
    let (range, markdown) = hover("format");
    assert_eq!(range, "format");
    assert!(markdown.contains("format: STRING"), "{markdown}");
    let (range, markdown) = hover("sales\n");
    assert_eq!(range, "sales");
    assert!(markdown.contains("TEMP EXTERNAL TABLE sales"), "{markdown}");
    assert!(markdown.contains("The sales"), "{markdown}");
    assert!(markdown.contains("`amount`: FLOAT64"), "{markdown}");
}

#[test]
fn a_chart_completes_its_statements_of_files_and_their_options() {
    let statements = complete("$0");
    let keywords = labels(&statements, CompletionKind::Keyword);
    assert!(
        keywords.contains(&"CREATE TEMP EXTERNAL TABLE"),
        "{keywords:?}"
    );
    assert!(keywords.contains(&"LOAD DATA"), "{keywords:?}");
    assert!(!keywords.contains(&"CREATE VIEW"), "{keywords:?}");
    // The options of the files, by the format.
    let options = complete("CREATE TEMP EXTERNAL TABLE t OPTIONS (format = 'CSV', $0)");
    let names = labels(&options, CompletionKind::Option);
    assert!(names.contains(&"skip_leading_rows"), "{names:?}");
    assert!(!names.contains(&"format"), "{names:?}");
    // The formats, and the paths of the project in a URI.
    let formats = complete("LOAD DATA INTO TEMP TABLE t FROM FILES (format = $0");
    let values = labels(&formats, CompletionKind::EnumValue);
    assert!(values.contains(&"'CSV'"), "{values:?}");
    let uris =
        complete("CREATE TEMP EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['data/files/$0'])");
    assert_eq!(
        labels(&uris, CompletionKind::File),
        ["data/files/sales.csv"]
    );
    // A query after them completes the chart's own tables.
    let tables = complete(
        "CREATE TEMP EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/files/sales.csv']);
FROM $0",
    );
    assert!(labels(&tables, CompletionKind::Table).contains(&"sales"));
}

#[test]
fn the_outline_of_a_chart_holds_its_tables_of_files_before_its_query() {
    let symbols = chart(CHART).symbols();
    let outline: Vec<(&str, SymbolKind, Option<&str>)> = (symbols.iter())
        .map(|symbol| (symbol.name.as_str(), symbol.kind, symbol.detail.as_deref()))
        .collect();
    assert_eq!(outline.len(), 3, "{outline:?}");
    assert_eq!(
        outline[..2],
        [
            ("sales", SymbolKind::Table, Some("TEMP EXTERNAL TABLE")),
            (
                "cars",
                SymbolKind::Table,
                Some("LOAD DATA OVERWRITE TEMP TABLE")
            ),
        ]
    );
}
