//! Add file's statements: the format of a file from its extensions, its table name, the
//! statement in the style of the formatter, the name of a copy, and the data script that takes
//! the statements.

use std::collections::BTreeSet;

use avenger_sql::data::{Compression, Format};
use avenger_sql_editor::add_file::{self, DataFile, NEW_SCRIPT};
use avenger_sql_syntax::Role;

#[test]
fn the_format_of_a_file_comes_from_its_extensions() {
    let of = |name| {
        DataFile::of(name).map(|file| (file.stem, file.extension, file.format, file.compression))
    };
    assert_eq!(of("sales.csv"), Ok(("sales", "csv", Format::Csv, None)));
    assert_eq!(of("Sales.CSV"), Ok(("Sales", "CSV", Format::Csv, None)));
    assert_eq!(of("cars.json"), Ok(("cars", "json", Format::Json, None)));
    assert_eq!(
        of("trips.2024.parquet"),
        Ok(("trips.2024", "parquet", Format::Parquet, None))
    );
    assert_eq!(
        of("points.arrow"),
        Ok(("points", "arrow", Format::Arrow, None))
    );
    assert_eq!(of("rows.avro"), Ok(("rows", "avro", Format::Avro, None)));
    assert_eq!(
        of("sales.csv.gz"),
        Ok(("sales", "csv.gz", Format::Csv, Some(Compression::Gzip)))
    );
    assert_eq!(
        of("log.json.zst"),
        Ok(("log", "json.zst", Format::Json, Some(Compression::Zstd)))
    );

    // Another extension, a compression of a format that takes none, and a `*`, which a URI
    // reads as a pattern.
    let error = DataFile::of("notes.txt").unwrap_err();
    assert!(
        error.starts_with("notes.txt has no format that a data script reads"),
        "{error}"
    );
    assert!(error.contains(".csv"), "{error}");
    assert!(error.contains(".gz"), "{error}");
    assert!(DataFile::of("trips.parquet.gz").is_err());
    assert!(DataFile::of("sales.gz").is_err());
    assert!(DataFile::of("csv").is_err());
    assert!(DataFile::of("a*.csv").unwrap_err().contains("pattern"));
}

#[test]
fn a_table_name_is_an_identifier_that_no_table_has() {
    let taken: BTreeSet<String> = ["sales", "sales_2", "cars"].map(String::from).into();
    let name = |stem| add_file::table_name(stem, &taken);
    assert_eq!(name("seattle-weather"), "seattle_weather");
    assert_eq!(name("Penguins"), "penguins");
    assert_eq!(name("2024 trips"), "_2024_trips");
    assert_eq!(name("trips.2024"), "trips_2024");
    assert_eq!(name("Café"), "caf_");
    assert_eq!(name(""), "table");
    // A reserved keyword takes `_`, and a taken name the first number that is free.
    assert_eq!(name("select"), "select_");
    assert_eq!(name("Sales"), "sales_3");
    assert_eq!(name("cars"), "cars_2");
}

#[test]
fn the_statement_is_in_the_style_of_the_formatter() {
    // On one line when it fits, and otherwise with `OPTIONS` on a line of its own.
    let file = DataFile::of("sales.csv").unwrap();
    let statement = add_file::statement("sales", &file, "data/files/sales.csv", Role::Data);
    assert_eq!(
        statement,
        "CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/files/sales.csv']);"
    );
    let uri = "data/files/vega_datasets/seattle-weather-observations.csv";
    let statement = add_file::statement("seattle_weather_observations", &file, uri, Role::Data);
    assert_eq!(
        statement,
        format!("CREATE EXTERNAL TABLE seattle_weather_observations\nOPTIONS (format = 'CSV', uris = ['{uri}']);")
    );
    assert_eq!(
        avenger_sql_syntax::format::format(&statement, Role::Data),
        statement
    );
    assert!(avenger_sql_syntax::parse_data(&statement).ok());

    // A compressed file has its compression, a JSON file the name of its format, and a quote or
    // a backslash of a URI is escaped.
    let file = DataFile::of("log.json.gz").unwrap();
    let statement = add_file::statement("log", &file, "data/files/it's\\log.json.gz", Role::Data);
    assert!(
        avenger_sql_syntax::parse_data(&statement).ok(),
        "{statement}"
    );
    assert!(
        statement.contains("format = 'NEWLINE_DELIMITED_JSON'"),
        "{statement}"
    );
    assert!(statement.contains("compression = 'GZIP'"), "{statement}");
    assert!(
        statement.contains(r"'data/files/it\'s\\log.json.gz'"),
        "{statement}"
    );

    // A chart's table of files is its own, so TEMP.
    let file = DataFile::of("sales.csv").unwrap();
    let statement = add_file::statement("sales", &file, "data/files/sales.csv", Role::Script);
    assert_eq!(
        statement,
        "CREATE TEMP EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/files/sales.csv']);"
    );
    assert!(avenger_sql_syntax::parse(&statement).ok());
}

#[test]
fn a_copy_takes_a_name_that_no_file_has() {
    let file = DataFile::of("sales.csv.gz").unwrap();
    let taken = ["sales.csv.gz", "sales-2.csv.gz"];
    assert_eq!(
        file.free_name(|name| taken.contains(&name)),
        "sales-3.csv.gz"
    );
    assert_eq!(file.free_name(|_| false), "sales.csv.gz");
}

#[test]
fn the_statements_go_into_the_active_script_or_the_first_data_script() {
    let scripts = ["data/b.sql", "data/a.sql"].map(String::from);
    fn script<'a>(active: Option<&'a str>, scripts: &'a [String]) -> &'a str {
        add_file::script(active, scripts, &["charts/x.sql".to_string()])
    }
    assert_eq!(script(Some("data/b.sql"), &scripts), "data/b.sql");
    // The active chart takes its own tables, and a file of no script the first data script.
    assert_eq!(script(Some("charts/x.sql"), &scripts), "charts/x.sql");
    assert_eq!(script(Some("styles/main.css"), &scripts), "data/a.sql");
    assert_eq!(script(None, &scripts), "data/a.sql");
    assert_eq!(script(Some("charts/x.sql"), &[]), "charts/x.sql");
    assert_eq!(script(None, &[]), NEW_SCRIPT);

    let statements = ["CREATE A;".to_string(), "CREATE B;".to_string()];
    assert_eq!(
        add_file::append("", &statements),
        "CREATE A;\n\nCREATE B;\n"
    );
    assert_eq!(
        add_file::append("-- Tables.\nCREATE X;\n\n\n", &statements[..1]),
        "-- Tables.\nCREATE X;\n\nCREATE A;\n"
    );
}

#[test]
fn a_chart_takes_its_tables_before_its_query_after_its_other_tables_of_files() {
    let statements = ["CREATE TEMP EXTERNAL TABLE a OPTIONS ();".to_string()];
    // Before the query, after the comments before it and after the DECLAREs.
    assert_eq!(
        add_file::insert("-- A chart.\nDECLARE n INT64 DEFAULT 1;\nFROM t\n", &statements),
        "-- A chart.\nDECLARE n INT64 DEFAULT 1;\nCREATE TEMP EXTERNAL TABLE a OPTIONS ();\n\nFROM t\n"
    );
    // After the chart's last statement of files.
    let text = "LOAD DATA OVERWRITE TEMP TABLE x FROM FILES ();\nSET n = 2;\nCREATE TEMP EXTERNAL TABLE y OPTIONS (); -- y\nFROM x\n";
    assert_eq!(
        add_file::insert(text, &statements),
        "LOAD DATA OVERWRITE TEMP TABLE x FROM FILES ();\nSET n = 2;\nCREATE TEMP EXTERNAL TABLE y OPTIONS (); -- y\n\nCREATE TEMP EXTERNAL TABLE a OPTIONS ();\nFROM x\n"
    );
    // An empty chart, and a chart of DECLAREs alone, take them at the end.
    assert_eq!(
        add_file::insert("", &statements),
        "CREATE TEMP EXTERNAL TABLE a OPTIONS ();\n"
    );
    // The names that a chart defines.
    let text = "CREATE TEMP TABLE Recent AS (FROM t);\nCREATE TEMP EXTERNAL TABLE y OPTIONS ();\nFROM recent";
    assert_eq!(
        add_file::chart_tables(text),
        BTreeSet::from(["recent".to_string(), "y".to_string()])
    );
}
