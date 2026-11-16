//! The table names of files.

use std::collections::BTreeSet;
use std::path::Path;

use avenger_sql_editor::sources::{table_name, FileKind};

/// A name is the lowercase file stem, with `_` for each character that an identifier without
/// quotes cannot hold, and before a leading digit.
#[test]
fn a_name_is_the_file_stem_as_an_identifier() {
    let none = BTreeSet::new();
    let name = |path: &str| table_name(Path::new(path), &none);
    assert_eq!(name("/data/penguins.csv"), "penguins");
    assert_eq!(name("/data/NYC Taxi-2024.parquet"), "nyc_taxi_2024");
    assert_eq!(name("/data/2024 sales.csv"), "_2024_sales");
    assert_eq!(name("/data/café.csv"), "caf_");
}

/// A name that another table has takes the first free suffix.
#[test]
fn a_taken_name_takes_a_suffix() {
    let taken: BTreeSet<String> = ["penguins", "penguins_2"].map(String::from).into();
    assert_eq!(
        table_name(Path::new("/other/penguins.parquet"), &taken),
        "penguins_3"
    );
}

#[test]
fn the_kind_comes_from_the_extension_in_any_case() {
    assert_eq!(FileKind::of(Path::new("a.CSV")), Some(FileKind::Csv));
    assert_eq!(
        FileKind::of(Path::new("a.parquet")),
        Some(FileKind::Parquet)
    );
    assert_eq!(FileKind::of(Path::new("a.json")), None);
}

/// A file whose name holds the characters of a glob, such as `[`, is the file itself, not the
/// files that the glob matches.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_whose_name_holds_a_glob_is_read_as_it_is() {
    let dir = std::env::temp_dir().join(format!("avenger-sql-editor-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("sales [2024] *?.csv");
    std::fs::write(&file, "region,units\nnorth,3\nsouth,4\n").unwrap();
    // A file that the glob `sales [2024] *?.csv` matches, which the table must not read.
    std::fs::write(dir.join("sales 2 xy.csv"), "other\n1\n").unwrap();
    let ctx = avenger_sql_editor::query::session().unwrap();
    let source = avenger_sql_editor::query::register(&ctx, "sales", &file, FileKind::Csv).await;
    std::fs::remove_dir_all(&dir).unwrap();
    let names: Vec<String> = (source.unwrap().columns.iter())
        .map(|(name, _)| name.clone())
        .collect();
    assert_eq!(names, ["region", "units"]);
}
