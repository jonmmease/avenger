//! Adding data files to a project: the format of a file from its extensions, its table name, the
//! statement in the style of the formatter, the name of a copy, the script that takes the
//! statements, and an add that copies files and writes the statements into a script.

use std::collections::BTreeSet;

use avenger_sql::data::{Compression, Format};
use std::path::{Path, PathBuf};

use avenger_sql_project::add::{self, adding, table_names, Added, DataFile, NEW_SCRIPT};
use avenger_sql_project::ProjectFiles;
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
        of("points.arrow"),
        Ok(("points", "arrow", Format::Arrow, None))
    );
    // A format or a compression whose cargo feature the build lacks reads no files.
    let available = |name, file, format: Format, compression: Option<Compression>| match format
        .available()
        && compression.is_none_or(Compression::available)
    {
        true => assert_eq!(of(name), Ok(file), "{name}"),
        false => assert!(of(name).unwrap_err().contains("This build"), "{name}"),
    };
    let parquet = ("trips.2024", "parquet", Format::Parquet, None);
    available("trips.2024.parquet", parquet, Format::Parquet, None);
    available(
        "rows.avro",
        ("rows", "avro", Format::Avro, None),
        Format::Avro,
        None,
    );
    let gzip = Some(Compression::Gzip);
    available(
        "sales.csv.gz",
        ("sales", "csv.gz", Format::Csv, gzip),
        Format::Csv,
        gzip,
    );
    let zstd = Some(Compression::Zstd);
    available(
        "log.json.zst",
        ("log", "json.zst", Format::Json, zstd),
        Format::Json,
        zstd,
    );

    // Another extension, a compression of a format that takes none, and a `*`, which a URI
    // reads as a pattern.
    let error = DataFile::of("notes.txt").unwrap_err();
    assert!(
        error.starts_with("notes.txt has no format that a data script reads"),
        "{error}"
    );
    assert!(error.contains(".csv"), "{error}");
    assert_eq!(
        error.contains(".gz"),
        Compression::Gzip.available(),
        "{error}"
    );
    assert!(DataFile::of("trips.parquet.gz").is_err());
    assert!(DataFile::of("sales.gz").is_err());
    assert!(DataFile::of("csv").is_err());
    assert!(DataFile::of("a*.csv").unwrap_err().contains("pattern"));
}

#[test]
fn a_table_name_is_an_identifier_that_no_table_has() {
    let taken: BTreeSet<String> = ["sales", "sales_2", "cars"].map(String::from).into();
    let name = |stem| add::table_name(stem, &taken);
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
    let statement = add::statement("sales", &file, "data/files/sales.csv", Role::Data);
    assert_eq!(
        statement,
        "CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/files/sales.csv']);"
    );
    let uri = "data/files/vega_datasets/seattle-weather-observations.csv";
    let statement = add::statement("seattle_weather_observations", &file, uri, Role::Data);
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
    let statement = add::statement("log", &file, "data/files/it's\\log.json.gz", Role::Data);
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
    let statement = add::statement("sales", &file, "data/files/sales.csv", Role::Script);
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
    let scripts = ["data/b.av.sql", "data/a.av.sql"].map(String::from);
    fn script<'a>(active: Option<&'a str>, scripts: &'a [String]) -> &'a str {
        add::script(active, scripts, &["charts/x.av.sql".to_string()])
    }
    assert_eq!(script(Some("data/b.av.sql"), &scripts), "data/b.av.sql");
    // The active chart takes its own tables, and a file of no script the first data script.
    assert_eq!(script(Some("charts/x.av.sql"), &scripts), "charts/x.av.sql");
    assert_eq!(script(Some("styles/main.css"), &scripts), "data/a.av.sql");
    assert_eq!(script(None, &scripts), "data/a.av.sql");
    assert_eq!(script(Some("charts/x.av.sql"), &[]), "charts/x.av.sql");
    assert_eq!(script(None, &[]), NEW_SCRIPT);

    let statements = ["CREATE A;".to_string(), "CREATE B;".to_string()];
    assert_eq!(add::append("", &statements), "CREATE A;\n\nCREATE B;\n");
    assert_eq!(
        add::append("-- Tables.\nCREATE X;\n\n\n", &statements[..1]),
        "-- Tables.\nCREATE X;\n\nCREATE A;\n"
    );
}

#[test]
fn a_chart_takes_its_tables_before_its_query_after_its_other_tables_of_files() {
    let statements = ["CREATE TEMP EXTERNAL TABLE a OPTIONS ();".to_string()];
    // Before the query, after the comments before it and after the DECLAREs.
    assert_eq!(
        add::insert("-- A chart.\nDECLARE n INT64 DEFAULT 1;\nFROM t\n", &statements),
        "-- A chart.\nDECLARE n INT64 DEFAULT 1;\nCREATE TEMP EXTERNAL TABLE a OPTIONS ();\n\nFROM t\n"
    );
    // After the chart's last statement of files.
    let text = "LOAD DATA OVERWRITE TEMP TABLE x FROM FILES ();\nSET n = 2;\nCREATE TEMP EXTERNAL TABLE y OPTIONS (); -- y\nFROM x\n";
    assert_eq!(
        add::insert(text, &statements),
        "LOAD DATA OVERWRITE TEMP TABLE x FROM FILES ();\nSET n = 2;\nCREATE TEMP EXTERNAL TABLE y OPTIONS (); -- y\n\nCREATE TEMP EXTERNAL TABLE a OPTIONS ();\nFROM x\n"
    );
    // An empty chart, and a chart of DECLAREs alone, take them at the end.
    assert_eq!(
        add::insert("", &statements),
        "CREATE TEMP EXTERNAL TABLE a OPTIONS ();\n"
    );
    // The names that a chart defines.
    let text = "CREATE TEMP TABLE Recent AS (FROM t);\nCREATE TEMP EXTERNAL TABLE y OPTIONS ();\nFROM recent";
    assert_eq!(
        add::chart_tables(text),
        BTreeSet::from(["recent".to_string(), "y".to_string()])
    );
}

const SALES: &str = "region,amount\nNorth,3\nSouth,5\n";
const REGIONS: &str = "data/regions.av.sql";

/// A copy of the fixture project in a temporary directory, by its canonical root.
fn copy() -> (tempfile::TempDir, PathBuf) {
    fn walk(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let path = entry.unwrap().path();
            let target = to.join(path.file_name().unwrap());
            match path.is_dir() {
                true => walk(&path, &target),
                false => drop(std::fs::copy(&path, &target).unwrap()),
            }
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects/vega");
    walk(&fixture, directory.path());
    let root = directory.path().canonicalize().unwrap();
    (directory, root)
}

/// The add of the files at `paths` to the project at `root`, with `given` first, into the text
/// of the script on disk.
async fn add_files(root: &Path, given: Option<&str>, paths: &[PathBuf]) -> Result<Added, String> {
    let files = ProjectFiles::find(root).await.unwrap();
    let copied = adding(root, &files, given, paths)?.copy();
    let text = std::fs::read_to_string(root.join(copied.script())).ok();
    let taken = ["heavy_by_origin", "wet_days"].map(String::from).into();
    copied.add(&files, text.as_deref(), &taken)
}

#[tokio::test]
async fn an_add_copies_files_and_writes_their_statements() {
    let (_directory, root) = copy();
    let outside = tempfile::tempdir().unwrap();
    let sales = outside.path().join("Sales.csv");
    std::fs::write(&sales, SALES).unwrap();
    let under = root.join("data/files/extra/heavy-by-origin.csv");
    std::fs::create_dir_all(under.parent().unwrap()).unwrap();
    std::fs::write(&under, SALES).unwrap();
    let notes = outside.path().join("notes.txt");
    std::fs::write(&notes, "").unwrap();

    // A file outside the root is copied into data/files, and a file under it keeps its place.
    // The statements go into the first data script, and a name that the project has takes a
    // number.
    let added = add_files(&root, None, &[sales.clone(), under, notes])
        .await
        .unwrap();
    assert_eq!(added.script, REGIONS);
    assert!(!added.new);
    assert_eq!(added.tables, ["sales", "heavy_by_origin_2"]);
    assert_eq!(added.copied, ["data/files/Sales.csv"]);
    assert_eq!(added.errors.len(), 1);
    assert!(added.errors[0].starts_with("notes.txt has no format"));
    assert_eq!(
        std::fs::read_to_string(root.join("data/files/Sales.csv")).unwrap(),
        SALES
    );
    let csv = DataFile::of("x.csv").unwrap();
    let statements = [
        add::statement("sales", &csv, "data/files/Sales.csv", Role::Data),
        add::statement(
            "heavy_by_origin_2",
            &csv,
            "data/files/extra/heavy-by-origin.csv",
            Role::Data,
        ),
    ];
    let original = std::fs::read_to_string(root.join(REGIONS)).unwrap();
    assert_eq!(added.text, add::append(&original, &statements));

    // The same file again is a copy of its own, and a given chart takes a TEMP table before its
    // query, with a name that no table of the chart has.
    let added = add_files(&root, Some("charts/origins.av.sql"), &[sales])
        .await
        .unwrap();
    assert_eq!(added.script, "charts/origins.av.sql");
    assert_eq!(added.copied, ["data/files/Sales-2.csv"]);
    assert_eq!(added.tables, ["sales"]);
    assert!(
        added.text.contains("CREATE TEMP EXTERNAL TABLE sales"),
        "{}",
        added.text
    );
    assert!(
        avenger_sql_syntax::parse(&added.text).ok(),
        "{}",
        added.text
    );
}

#[tokio::test]
async fn a_project_without_data_scripts_gets_one_for_added_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let manifest = avenger_sql_project::Manifest::smallest("sales");
    std::fs::write(root.join(avenger_sql_project::MANIFEST), manifest).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let sales = outside.path().join("sales.csv");
    std::fs::write(&sales, SALES).unwrap();

    let added = add_files(&root, None, &[sales]).await.unwrap();
    assert_eq!(added.script, NEW_SCRIPT);
    assert!(added.new);
    assert_eq!(
        added.text,
        "CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/files/sales.csv']);\n"
    );
    // No file to add is an error.
    let error = add_files(&root, None, &[outside.path().join("notes.txt")])
        .await
        .unwrap_err();
    assert!(error.contains("notes.txt has no format"), "{error}");
}

#[tokio::test]
async fn the_names_of_the_tables_come_from_the_graph() {
    let (_directory, root) = copy();
    let files = ProjectFiles::find(&root).await.unwrap();
    let analysis = avenger_sql_project::Analysis::new(&files).await.unwrap();
    let names = table_names(analysis.project().graph());
    assert!(
        names.contains("heavy_by_origin") && names.contains("wet_days"),
        "{names:?}"
    );
    // A table in a schema has a name of two, which the table of an added file never takes.
    assert!(!names.contains("cars"), "{names:?}");
}
