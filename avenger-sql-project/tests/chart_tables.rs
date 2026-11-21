//! A chart script's own tables of files in a loaded project: the tables that its runs build and
//! keep by its path, which no other script reads, and the errors of its statements of files.

use std::path::Path;

use avenger_sql_project::{ChartError, FileSource, LoadedProject, ProjectFiles};
use datafusion::{arrow::util::pretty::pretty_format_batches, prelude::SessionContext};

const SALES: &str = "region,amount\neast,1\nwest,2\n";

/// A project in a temporary directory, with a data script of the table `regions` and the files
/// `data/files/sales.csv` and `data/files/regions.csv`.
async fn project() -> (tempfile::TempDir, LoadedProject) {
    let root = tempfile::tempdir().unwrap();
    let files = [
        ("avenger.toml", "[project]\nname = \"charts\"\nformat = 1\n"),
        ("data/files/sales.csv", SALES),
        ("data/files/regions.csv", "region,name\neast,East\nwest,West\n"),
        (
            "data/regions.sql",
            "CREATE EXTERNAL TABLE regions OPTIONS (format = 'CSV', uris = ['data/files/regions.csv']);\n",
        ),
        ("charts/sales.sql", "FROM regions"),
    ];
    for (path, text) in files {
        write(root.path(), path, text);
    }
    let files = ProjectFiles::open(FileSource::directory(root.path()).unwrap())
        .await
        .unwrap();
    let project = LoadedProject::load(files, SessionContext::new())
        .await
        .unwrap();
    (root, project)
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The rows of the chart script `text` at `path` of `project`, as a table.
async fn rows(project: &LoadedProject, path: &str, text: &str) -> Result<String, ChartError> {
    let frame = project.run_chart(path, text).await?;
    let batches = frame.collect().await.unwrap();
    Ok(pretty_format_batches(&batches).unwrap().to_string())
}

/// The total of the amounts of the table `sales`, after the statements `statements`.
fn total(statements: &str) -> String {
    format!("{statements};\nFROM sales |> AGGREGATE SUM(amount) AS total")
}

const EXTERNAL: &str =
    "CREATE TEMP EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/files/sales.csv'])";
const LOAD: &str =
    "LOAD DATA OVERWRITE TEMP TABLE sales FROM FILES (format = 'CSV', uris = ['data/files/sales.csv'])";

#[tokio::test]
async fn a_chart_reads_its_own_tables_and_no_other_script_does() {
    let (_root, project) = project().await;
    let text = format!(
        "{EXTERNAL};\nFROM sales JOIN regions USING (region) |> SELECT name, amount |> ORDER BY name"
    );
    let expected = "\
+------+--------+
| name | amount |
+------+--------+
| East | 1      |
| West | 2      |
+------+--------+";
    assert_eq!(
        rows(&project, "charts/sales.sql", &text).await.unwrap(),
        expected
    );
    // The session holds no table of the chart's, and another chart doesn't read it.
    assert!(!project.session().table_exist("sales").unwrap());
    let error = rows(&project, "charts/other.sql", "FROM sales")
        .await
        .unwrap_err();
    assert!(matches!(error, ChartError::Run(_)), "{error}");
}

#[tokio::test]
async fn a_chart_keeps_its_tables_until_its_statements_of_files_change() {
    let (root, project) = project().await;
    let path = "charts/sales.sql";
    let three = "\
+-------+
| total |
+-------+
| 3     |
+-------+";
    assert_eq!(rows(&project, path, &total(LOAD)).await.unwrap(), three);
    // The rows of the load stay in memory: a run of another query reads them, and not the file.
    write(
        root.path(),
        "data/files/sales.csv",
        "region,amount\neast,10\n",
    );
    let query = format!("{LOAD};\nFROM sales |> AGGREGATE SUM(amount) AS total, COUNT(*) AS n");
    let kept = rows(&project, path, &query).await.unwrap();
    assert!(kept.contains("| 3     | 2 |"), "{kept}");
    // Another chart builds its own tables, from the file as it is now.
    let other = rows(&project, "charts/other.sql", &total(LOAD))
        .await
        .unwrap();
    assert!(other.contains("| 10    |"), "{other}");
    // A change of the statements of files builds the tables again.
    let changed = LOAD.replace(" FROM FILES", " OPTIONS (description = 'sales') FROM FILES");
    let built = rows(&project, path, &total(&changed)).await.unwrap();
    assert!(built.contains("| 10    |"), "{built}");
}

#[tokio::test]
async fn a_failing_statement_of_files_is_an_error_at_the_statement() {
    let (_root, project) = project().await;
    // The files have two columns, and the column list one, so their rows fail to load.
    let text = "LOAD DATA OVERWRITE TEMP TABLE sales (region STRING)
FROM FILES (format = 'CSV', uris = ['data/files/sales.csv'], skip_leading_rows = 1);
FROM sales |> AGGREGATE COUNT(*) AS n";
    let Err(ChartError::Run(error)) = project.run_chart("charts/sales.sql", text).await else {
        panic!("an error of the run");
    };
    let datafusion::common::DataFusionError::External(error) = error else {
        panic!("a script error");
    };
    let error = error.downcast_ref::<avenger_sql::ScriptError>().unwrap();
    assert_eq!(error.diagnostics.len(), 1);
    assert!(text[error.diagnostics[0].range].starts_with("LOAD DATA"));
    // A URI that matches no file is an error at the URIs.
    let text = total(
        "CREATE TEMP EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/files/none.csv'])",
    );
    let Err(ChartError::Run(error)) = project.run_chart("charts/sales.sql", &text).await else {
        panic!("an error of the run");
    };
    assert!(error.to_string().contains("none.csv"), "{error}");
}

#[tokio::test]
async fn a_chart_lifts_with_its_statements_of_files() {
    let (_root, project) = project().await;
    let text = format!(
        "{EXTERNAL};\nFROM sales |> AGGREGATE mark_bar(x => region, y => SUM(amount)) AS chart"
    );
    let lifted = project.lift("charts/sales.sql", &text).await.unwrap();
    assert!(lifted.text.starts_with(EXTERNAL), "{}", lifted.text);
}

#[tokio::test]
async fn a_changed_file_drops_the_tables_of_the_charts_that_read_it() {
    let (root, mut project) = project().await;
    let path = "charts/sales.sql";
    rows(&project, path, &total(LOAD)).await.unwrap();
    // A pattern that matches no file yet, whose chart fails.
    let pattern = "CREATE TEMP EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/files/new/*.csv'])";
    assert!(rows(&project, "charts/new.sql", &total(pattern))
        .await
        .is_err());
    assert_eq!(project.scripts_reading("data/files/sales.csv"), [path]);
    assert_eq!(
        project.scripts_reading("data/files/new/a.csv"),
        ["charts/new.sql"]
    );
    assert_eq!(
        project.scripts_reading("data/files/regions.csv"),
        ["data/regions.sql"]
    );

    // The chart's load reads the changed file at its next run, and a data script's tables stay.
    write(
        root.path(),
        "data/files/sales.csv",
        "region,amount\neast,10\n",
    );
    let reload = project.reload_files(&["data/files/sales.csv"]).await;
    assert!(reload.tables.is_empty(), "{reload:?}");
    assert_eq!(reload.charts, [path]);
    let rerun = rows(&project, path, &total(LOAD)).await.unwrap();
    assert!(rerun.contains("| 10    |"), "{rerun}");

    // A new file of the pattern runs the chart that failed.
    write(root.path(), "data/files/new/a.csv", SALES);
    let reload = project.reload_files(&["data/files/new/a.csv"]).await;
    assert_eq!(reload.charts, ["charts/new.sql"]);
    assert!(rows(&project, "charts/new.sql", &total(pattern))
        .await
        .is_ok());
}
