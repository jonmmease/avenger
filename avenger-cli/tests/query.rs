//! `tables`, `schema`, `query`, and `charts` on copies of avenger-sql-project's fixture project:
//! their text and JSON output, the formats of `query`, the errors of a query and of a data script
//! that does not load, and their exit statuses.

use std::path::{Path, PathBuf};
use std::process::Command;

use datafusion::arrow::array::{Array, Int64Array, StringArray};

/// A copy of the fixture project in a temporary directory, by its canonical root, with a chart
/// script of a table of charts and a document of a labeled cell and an unlabeled one.
fn project() -> (tempfile::TempDir, PathBuf) {
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
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../avenger-sql-project/tests/projects/vega");
    let directory = tempfile::tempdir().unwrap();
    walk(&fixture, directory.path());
    let root = directory.path().canonicalize().unwrap();
    std::fs::write(
        root.join("charts/by-origin.av.sql"),
        "FROM vega_datasets.cars\n|> AGGREGATE COUNT(*) AS cars, mark_symbol(x => Horsepower, y => Miles_per_Gallon) AS chart GROUP BY Origin\n|> ORDER BY Origin\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("reports")).unwrap();
    std::fs::write(
        root.join("reports/overview.qmd"),
        "# Report\n\n```{avenger-sql}\n--| label: fig-weather\nFROM wet_days |> AGGREGATE mark_bar(x => weather, y => COUNT(*)) AS chart\n```\n\n```{avenger-sql}\nFROM wet_days |> AGGREGATE COUNT(*) AS days\n```\n",
    )
    .unwrap();
    (directory, root)
}

/// The exit status, standard output, and standard error of `avenger` with `args` in `directory`.
fn avenger(directory: &Path, args: &[&str]) -> (i32, Vec<u8>, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_avenger"))
        .args(args)
        .current_dir(directory)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        output.stdout,
        String::from_utf8(output.stderr).unwrap(),
    )
}

/// `avenger` with `args` in `directory`, with its standard output as text.
fn text(directory: &Path, args: &[&str]) -> (i32, String, String) {
    let (status, stdout, stderr) = avenger(directory, args);
    (status, String::from_utf8(stdout).unwrap(), stderr)
}

/// `avenger --json` with `args` in `directory`, with its standard output as JSON.
fn json(directory: &Path, args: &[&str]) -> (i32, serde_json::Value) {
    let args: Vec<&str> = std::iter::once("--json")
        .chain(args.iter().copied())
        .collect();
    let (status, stdout, stderr) = text(directory, &args);
    let value = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout} {stderr}"));
    (status, value)
}

#[test]
fn tables_lists_the_name_and_the_script_of_each_table() {
    let (_directory, root) = project();
    let (status, stdout, stderr) = text(&root, &["tables"]);
    assert_eq!((status, stderr.as_str()), (0, ""));
    assert_eq!(
        stdout,
        "vega_datasets.cars             data/vega_datasets.av.sql\n\
         vega_datasets.seattle_weather  data/vega_datasets.av.sql\n\
         vega_datasets.regions          data/regions.av.sql\n\
         heavy_by_origin                data/summaries.av.sql\n\
         wet_days                       data/summaries.av.sql\n"
    );
    let (status, value) = json(&root, &["tables"]);
    assert_eq!((status, value["version"].as_u64()), (0, Some(1)));
    assert_eq!(
        value["tables"][3],
        serde_json::json!({"name": "heavy_by_origin", "path": ["heavy_by_origin"], "script": "data/summaries.av.sql"})
    );
    assert_eq!(value["diagnostics"], serde_json::json!([]));
    // From a subdirectory, and from another directory with `--project`, with the paths of the
    // scripts from the current directory.
    let (status, below, _) = text(&root.join("charts"), &["tables"]);
    let script = root.join("data/summaries.av.sql");
    assert!(
        status == 0 && below.contains(&script.display().to_string()),
        "{below}"
    );
    let project = root.to_str().unwrap();
    let (status, elsewhere, _) = text(root.parent().unwrap(), &["tables", "--project", project]);
    let name = root.file_name().unwrap().to_str().unwrap();
    let script = format!("{name}/data/summaries.av.sql");
    assert!(status == 0 && elsewhere.contains(&script), "{elsewhere}");
}

#[test]
fn a_data_script_that_does_not_load_fails_the_listing_with_its_errors() {
    let (_directory, root) = project();
    let summaries = std::fs::read_to_string(root.join("data/summaries.av.sql")).unwrap();
    let broken = summaries.replace("COUNT(*) AS cars", "COUNT(nope) AS cars");
    std::fs::write(root.join("data/summaries.av.sql"), broken).unwrap();
    for command in ["tables", "schema"] {
        let (status, stdout, stderr) = text(&root, &[command]);
        assert_eq!(status, 1, "{command}");
        assert!(!stdout.contains("heavy_by_origin"), "{stdout}");
        assert!(stdout.contains("vega_datasets.cars"), "{stdout}");
        assert!(
            stderr.contains("data/summaries.av.sql:") && stderr.contains("nope"),
            "{stderr}"
        );
        let (status, value) = json(&root, &[command]);
        assert_eq!(status, 1);
        let diagnostic = &value["diagnostics"][0];
        assert_eq!(diagnostic["path"], "data/summaries.av.sql");
        assert_eq!(diagnostic["severity"], "error");
    }
    // The table that the script would register is missing, and the error says why.
    let (status, _, stderr) = text(&root, &["schema", "heavy_by_origin"]);
    assert_eq!(status, 2);
    assert!(stderr.contains("data/summaries.av.sql"), "{stderr}");
}

#[test]
fn schema_prints_the_columns_and_the_rows_of_the_tables_that_it_names() {
    let (_directory, root) = project();
    let (status, stdout, _) = text(&root, &["schema", "heavy_by_origin"]);
    assert_eq!(
        (status, stdout.as_str()),
        (
            0,
            "heavy_by_origin (2 rows, data/summaries.av.sql)\n  Origin  STRING\n  cars    INT64\n"
        )
    );
    // By its path in the default schema, and several tables, in their order.
    assert_eq!(text(&root, &["schema", "public.heavy_by_origin"]).1, stdout);
    let (status, stdout, _) = text(&root, &["schema", "vega_datasets.regions", "wet_days"]);
    assert_eq!(status, 0);
    assert!(
        stdout.starts_with("vega_datasets.regions (3 rows, data/regions.av.sql)\n  Origin  STRING\n  region  STRING\n\nwet_days (623 rows, data/summaries.av.sql)\n"),
        "{stdout}"
    );
    // Without a name, every table.
    let (status, stdout, _) = text(&root, &["schema"]);
    assert_eq!(status, 0);
    assert_eq!(stdout.matches(" rows, ").count(), 5, "{stdout}");
    let (status, value) = json(&root, &["schema", "vega_datasets.regions"]);
    assert_eq!(status, 0);
    let table = &value["tables"][0];
    assert_eq!(
        table["path"],
        serde_json::json!(["vega_datasets", "regions"])
    );
    assert_eq!(table["rows"], 3);
    assert_eq!(
        table["columns"][1],
        serde_json::json!({"name": "region", "type": "STRING"})
    );
    // A table of another schema by its name alone suggests its path, and a temporary table is
    // no table.
    let (status, stdout, stderr) = text(&root, &["schema", "wet_days", "regions"]);
    assert_eq!((status, stdout.as_str()), (2, ""));
    assert_eq!(
        stderr,
        "error: no table `regions`. Did you mean vega_datasets.regions?\n"
    );
    let (status, _, stderr) = text(&root, &["schema", "heavy"]);
    assert_eq!(status, 2);
    assert_eq!(
        stderr,
        "error: no table `heavy`: `avenger tables` lists the tables\n"
    );
}

#[test]
fn query_prints_rows_as_a_table_csv_json_or_the_object_of_json() {
    let (_directory, root) = project();
    let sql = "FROM heavy_by_origin |> ORDER BY Origin";
    let (status, stdout, stderr) = text(&root, &["query", sql]);
    assert_eq!((status, stderr.as_str()), (0, ""));
    assert!(
        stdout.starts_with("+--------+------+\n| Origin | cars |\n"),
        "{stdout}"
    );
    assert!(stdout.contains("| Europe | 11   |"), "{stdout}");
    let (_, csv, _) = text(&root, &["query", "--output", "csv", sql]);
    assert_eq!(csv, "Origin,cars\nEurope,11\nUSA,163\n");
    let (_, rows, _) = text(&root, &["query", "--output", "json", "--limit", "1", sql]);
    assert_eq!(rows, r#"[{"Origin":"Europe","cars":11}]"#);
    let (status, value) = json(&root, &["query", sql]);
    assert_eq!(status, 0);
    assert_eq!(
        value["columns"][1],
        serde_json::json!({"name": "cars", "type": "INT64", "chart": false})
    );
    assert_eq!(
        value["rows"][1],
        serde_json::json!({"Origin": "USA", "cars": 163})
    );
    // A result without rows keeps its header.
    let none = "FROM heavy_by_origin |> WHERE cars < 0";
    assert_eq!(
        text(&root, &["query", "--output", "csv", none]).1,
        "Origin,cars\n"
    );
    assert_eq!(text(&root, &["query", "--output", "json", none]).1, "[]");
}

#[test]
fn query_keeps_the_first_10_rows_unless_its_limit_says_otherwise() {
    let (_directory, root) = project();
    let sql = "FROM wet_days |> SELECT date";
    let (status, csv, stderr) = text(&root, &["query", "--output", "csv", sql]);
    assert_eq!(status, 0);
    assert_eq!(csv.lines().count(), 11, "{csv}");
    assert_eq!(
        stderr,
        "the first 10 rows of more: `--limit N` for more, or `--limit all`\n"
    );
    let (_, csv, stderr) = text(&root, &["query", "--output", "csv", "--limit", "all", sql]);
    assert_eq!((csv.lines().count(), stderr.as_str()), (624, ""));
    // A limit that cuts nothing off says nothing.
    let (_, csv, stderr) = text(&root, &["query", "--output", "csv", "--limit", "623", sql]);
    assert_eq!((csv.lines().count(), stderr.as_str()), (624, ""));
    let (_, value) = json(&root, &["query", "--limit", "2", sql]);
    assert_eq!(
        (value["rows"].as_array().unwrap().len(), &value["more"]),
        (2, &serde_json::json!(true))
    );
    let (_, value) = json(&root, &["query", "--limit", "all", sql]);
    assert_eq!(value["more"], false);
    let (status, _, _) = text(&root, &["query", "--limit", "some", sql]);
    assert_eq!(status, 2);
    // A file that `-o` writes keeps every row, unless `--limit` says otherwise.
    let parquet_rows = |file: &str| -> usize {
        let file = std::fs::File::open(root.join(file)).unwrap();
        let reader =
            datafusion::parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(
                file,
            )
            .unwrap()
            .build()
            .unwrap();
        reader.map(|batch| batch.unwrap().num_rows()).sum()
    };
    let (status, stdout, stderr) = text(&root, &["query", sql, "-o", "rows.parquet"]);
    assert_eq!(
        (status, stdout.as_str(), stderr.as_str()),
        (0, "rows.parquet\n", "")
    );
    assert_eq!(parquet_rows("rows.parquet"), 623);
    let (status, _, stderr) = text(&root, &["query", sql, "--limit", "5", "-o", "five.parquet"]);
    assert_eq!(status, 0);
    assert_eq!(parquet_rows("five.parquet"), 5);
    assert!(stderr.starts_with("the first 5 rows of more"), "{stderr}");
}

#[test]
fn query_runs_a_chart_script_or_a_cell_with_its_charts_as_text() {
    let (_directory, root) = project();
    let (status, csv, stderr) = text(
        &root,
        &["query", "--output", "csv", "-f", "charts/by-origin.av.sql"],
    );
    assert_eq!((status, stderr.as_str()), (0, ""));
    assert_eq!(
        csv,
        "Origin,cars,chart\nEurope,73,<chart>\nJapan,79,<chart>\nUSA,254,<chart>\n"
    );
    let (_, value) = json(&root, &["query", "-f", "charts/by-origin.av.sql"]);
    assert_eq!(value["columns"][2]["chart"], true);
    let (_, csv, _) = text(
        &root,
        &["query", "--output", "csv", "-f", "reports/overview.qmd#2"],
    );
    assert_eq!(csv, "days\n623\n");
    let (_, csv, _) = text(
        &root,
        &[
            "query",
            "--output",
            "csv",
            "-f",
            "reports/overview.qmd#fig-weather",
        ],
    );
    assert_eq!(csv, "chart\n<chart>\n");
    // Another file of the project runs in it, with its tables.
    std::fs::write(
        root.join("scratch.sql"),
        "FROM wet_days |> AGGREGATE COUNT(*) AS days\n",
    )
    .unwrap();
    let (_, csv, _) = text(&root, &["query", "--output", "csv", "-f", "scratch.sql"]);
    assert_eq!(csv, "days\n623\n");
}

#[test]
fn query_writes_parquet_and_arrow_that_read_back() {
    let (_directory, root) = project();
    let sql = "FROM heavy_by_origin |> ORDER BY Origin";
    // By the extension of `-o`.
    let (status, stdout, _) = text(&root, &["query", sql, "-o", "rows.parquet"]);
    assert_eq!((status, stdout.as_str()), (0, "rows.parquet\n"));
    let file = std::fs::File::open(root.join("rows.parquet")).unwrap();
    let reader =
        datafusion::parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();
    let batches: Vec<_> = reader.map(Result::unwrap).collect();
    let cars = batches[0]
        .column(1)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(cars.values(), &[11, 163]);
    // To standard output, which is a pipe here.
    let (status, bytes, _) = avenger(&root, &["query", "--output", "arrow", sql]);
    assert_eq!(status, 0);
    let reader =
        datafusion::arrow::ipc::reader::FileReader::try_new(std::io::Cursor::new(bytes), None)
            .unwrap();
    let batch = reader.into_iter().next().unwrap().unwrap();
    let origins = batch
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!((origins.len(), origins.value(0)), (2, "Europe"));
    // A chart keeps its bytes.
    let (status, _, _) = text(
        &root,
        &[
            "query",
            "-f",
            "charts/by-origin.av.sql",
            "-o",
            "charts.arrow",
        ],
    );
    assert_eq!(status, 0);
    let file = std::fs::File::open(root.join("charts.arrow")).unwrap();
    let reader = datafusion::arrow::ipc::reader::FileReader::try_new(file, None).unwrap();
    let schema = reader.schema();
    assert!(avenger_chart::results::is_chart_field(schema.field(2)));
}

#[test]
fn query_explains_its_plan() {
    let (_directory, root) = project();
    let sql = "FROM wet_days |> AGGREGATE COUNT(*) AS days";
    let (status, stdout, _) = text(&root, &["query", "--explain", sql]);
    assert_eq!(status, 0);
    assert!(stdout.starts_with("logical_plan:\n"), "{stdout}");
    assert!(stdout.contains("physical_plan:\n"), "{stdout}");
    let (status, value) = json(&root, &["query", "--explain", sql]);
    assert_eq!(status, 0);
    assert_eq!(value["plans"][0]["type"], "logical_plan");
}

#[test]
fn a_query_that_fails_writes_no_rows_and_its_errors_at_their_places() {
    let (_directory, root) = project();
    let (status, stdout, stderr) = text(&root, &["query", "FROM heavy_by_origin |> SELECT nope"]);
    assert_eq!((status, stdout.as_str()), (1, ""));
    assert!(
        stderr.starts_with("<query>:1:32: error: Unrecognized name: nope\n"),
        "{stderr}"
    );
    let (status, value) = json(&root, &["query", "FROM heavy_by_origin |> SELECT nope"]);
    assert_eq!(status, 1);
    assert_eq!(value["diagnostics"][0]["range"]["start"]["column"], 32);
    // A file's errors are at their places in the file.
    std::fs::write(root.join("scratch.sql"), "FROM wet_days\n|> SELECT nope\n").unwrap();
    let (status, _, stderr) = text(&root, &["query", "-f", "scratch.sql"]);
    assert_eq!(status, 1);
    assert!(stderr.starts_with("scratch.sql:2:11: error: "), "{stderr}");
}

#[test]
fn query_usage_errors_exit_2() {
    let (_directory, root) = project();
    let sql = "FROM wet_days";
    for args in [
        vec!["--json", "query", "--output", "json", sql],
        vec!["query", "--output", "xlsx", sql],
        vec!["query", "--explain", "--output", "csv", sql],
        vec!["query", sql, "-o", "rows.xlsx"],
        vec!["query"],
    ] {
        let (status, stdout, _) = text(&root, &args);
        assert_eq!((status, stdout.as_str()), (2, ""), "{args:?}");
    }
    // Outside a project.
    let outside = tempfile::tempdir().unwrap();
    let (status, _, stderr) = text(outside.path(), &["query", sql]);
    assert_eq!(status, 2);
    assert!(stderr.contains("not in an avenger project"), "{stderr}");
}

#[test]
fn charts_lists_the_names_that_render_takes() {
    let (_directory, root) = project();
    let (status, stdout, stderr) = text(&root, &["charts"]);
    assert_eq!((status, stderr.as_str()), (0, ""));
    assert_eq!(
        stdout,
        "charts/by-origin.av.sql: chart\ncharts/origins.av.sql: chart\ncharts/weather/warm-days.av.sql: chart\nreports/overview.qmd#fig-weather: chart\nreports/overview.qmd#2: no charts\n"
    );
    let (status, value) = json(&root, &["charts"]);
    assert_eq!(status, 0);
    let cell = &value["charts"][3];
    assert_eq!(
        (
            cell["target"].as_str(),
            cell["cell"].as_u64(),
            cell["label"].as_str()
        ),
        (
            Some("reports/overview.qmd#fig-weather"),
            Some(1),
            Some("fig-weather")
        )
    );
    // Each name renders.
    for chart in value["charts"].as_array().unwrap() {
        if chart["charts"].as_array().unwrap().is_empty() {
            continue;
        }
        let target = chart["target"].as_str().unwrap();
        let (status, _, stderr) = text(&root, &["render", target, "-o", "chart.svg"]);
        assert_eq!(status, 0, "{target}: {stderr}");
    }
    // A chart whose query does not plan lists its error and fails.
    std::fs::write(
        root.join("charts/bad.av.sql"),
        "FROM wet_days |> SELECT nope\n",
    )
    .unwrap();
    let (status, stdout, _) = text(&root, &["charts"]);
    assert_eq!(status, 1);
    assert!(stdout.starts_with("charts/bad.av.sql: error: "), "{stdout}");
}
