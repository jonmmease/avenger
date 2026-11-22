//! A chart script's own tables of files in a project: definitions, references, renames, hover,
//! and warnings that stay in the chart, and the tables of the other files, which never see them.

use std::sync::Arc;

use avenger_sql::{
    analysis::Options,
    catalog::{FilesTable, MemoryCatalog},
    data::{file_reads, Files},
    library::Library,
    types::TypeOptions,
};
use avenger_sql_language::{
    CompletionKind, FileRole, Offset, Project, Severity, Source, TextRange,
};
use avenger_sql_syntax::parse;
use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    datasource::empty::EmptyTable,
};

const REGIONS: &str = "data/regions.sql";
const SALES: &str = "charts/sales.sql";
const OTHER: &str = "charts/other.sql";

const REGIONS_TEXT: &str =
    "CREATE EXTERNAL TABLE regions OPTIONS (format = 'CSV', uris = ['data/files/regions.csv']);";
const SALES_TEXT: &str = "CREATE TEMP EXTERNAL TABLE sales
OPTIONS (format = 'CSV', uris = ['data/files/sales.csv'], description = 'The sales');
FROM sales JOIN regions USING (region)
|> AGGREGATE mark_bar(x => name, y => SUM(amount)) AS chart";
const OTHER_TEXT: &str = "FROM regions |> SELECT name";

/// The table of each set of files that the scripts `texts` read: the columns `region` and
/// `name` of the regions, and `region` and `amount` of the other files.
fn tables(texts: &[&str]) -> Vec<(Files, FilesTable)> {
    let column = |name: &str, ty: DataType| Field::new(name, ty, true);
    (texts.iter())
        .flat_map(|text| file_reads(&parse(text).syntax(), TypeOptions::default()))
        .map(|read| {
            let second = match read.files.uris[0].as_str() {
                "data/files/regions.csv" => column("name", DataType::Utf8),
                _ => column("amount", DataType::Float64),
            };
            let schema = Schema::new(vec![column("region", DataType::Utf8), second]);
            let table: FilesTable = Ok(Arc::new(EmptyTable::new(Arc::new(schema))));
            (read.files, table)
        })
        .collect()
}

/// The project of a data script and two charts, whose loader read the files of `read`.
fn project_with(read: &[&str], sales: &str) -> Project {
    Project::new(
        Arc::new(MemoryCatalog::new()),
        Arc::new(Library::new()),
        Options::default(),
    )
    .with_file_tables(tables(read))
    .with_files([
        (REGIONS, FileRole::Data, REGIONS_TEXT),
        (SALES, FileRole::Chart, sales),
        (OTHER, FileRole::Chart, OTHER_TEXT),
    ])
}

fn project() -> Project {
    project_with(&[REGIONS_TEXT, SALES_TEXT], SALES_TEXT)
}

/// The offset of the `n`th `needle` in the file at `path`.
fn find(project: &Project, path: &str, needle: &str, n: usize) -> Offset {
    let text = project.document(path).unwrap().text();
    Offset::from(text.match_indices(needle).nth(n).unwrap().0 as u32)
}

#[test]
fn a_chart_s_own_tables_have_no_errors_and_no_other_file_sees_them() {
    let mut project = project();
    for path in [REGIONS, SALES, OTHER] {
        assert_eq!(project.diagnostics(path), Vec::new(), "{path}");
    }
    // Another chart doesn't read or complete the chart's table.
    project.set_texts([(OTHER, "FROM sales")]);
    let errors = project.diagnostics(OTHER);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].message.contains("sales"), "{errors:?}");
    let document = project.document(OTHER).unwrap();
    let completions = document.completions(Offset::from(5));
    assert!(!(completions.items.iter())
        .any(|item| item.kind == CompletionKind::Table && item.label == "sales"));
}

#[test]
fn a_chart_s_own_table_has_its_definition_references_and_hover_in_the_chart() {
    // With the table of its files, and without one, before the loader has run the chart.
    for project in [project(), project_with(&[REGIONS_TEXT], SALES_TEXT)] {
        let read = find(&project, SALES, "FROM sales", 0) + Offset::from(6);
        let link = project.definition(SALES, read).unwrap();
        assert_eq!(link.path, SALES);
        let name = TextRange::at(find(&project, SALES, "sales", 0), Offset::from(5));
        assert_eq!(link.target, name);
        let references = project.references(SALES, read);
        let paths: Vec<&str> = (references.iter())
            .map(|reference| reference.path.as_str())
            .collect();
        assert_eq!(paths, [SALES, SALES], "{references:?}");
        let hover = project.document(SALES).unwrap().hover(read).unwrap();
        let sales = find(&project, SALES, "FROM sales", 0) + Offset::from(5);
        assert_eq!(hover.range, TextRange::at(sales, Offset::from(5)));
        assert!(
            hover.markdown.contains("TEMP EXTERNAL TABLE sales"),
            "{}",
            hover.markdown
        );
        let edits = project.rename(SALES, read, "sold").unwrap();
        assert_eq!(edits.keys().collect::<Vec<_>>(), [SALES]);
        assert_eq!(edits[SALES].len(), 2);
    }
}

#[test]
fn a_chart_s_own_table_renames_in_the_chart_alone() {
    let project = project();
    let read = find(&project, SALES, "FROM sales", 0) + Offset::from(6);
    let edits = project.rename(SALES, read, "orders").unwrap();
    assert_eq!(edits.keys().collect::<Vec<_>>(), [SALES]);
    assert_eq!(edits[SALES].len(), 2);
    // The name of a table of the project is taken.
    assert!(project.rename(SALES, read, "regions").is_err());
}

#[test]
fn a_chart_s_uri_outside_the_root_is_a_warning() {
    let text = SALES_TEXT.replace("'data/files/sales.csv'", "'../sales.csv'");
    let project = project_with(&[REGIONS_TEXT, &text], &text);
    let warnings: Vec<(Source, Severity)> = (project.diagnostics(SALES).into_iter())
        .map(|diagnostic| (diagnostic.source, diagnostic.severity))
        .collect();
    assert_eq!(warnings, [(Source::Project, Severity::Warning)]);
}

#[test]
fn a_chart_whose_files_the_loader_has_not_read_has_no_errors_at_them() {
    let mut project = project_with(&[REGIONS_TEXT], SALES_TEXT);
    assert_eq!(project.diagnostics(SALES), Vec::new());
    // When the loader reads them, the chart analyzes again with their columns.
    project.set_file_tables(tables(&[REGIONS_TEXT, SALES_TEXT]));
    let read = find(&project, SALES, "FROM sales", 0) + Offset::from(6);
    let hover = project.document(SALES).unwrap().hover(read).unwrap();
    assert!(
        hover.markdown.contains("`amount`: FLOAT64"),
        "{}",
        hover.markdown
    );
}
