//! The `avenger` binary on copies of avenger-sql-project's fixture project: `check` with its
//! text and JSON output and its exit statuses, a stylesheet's errors, `--run`, a loose script,
//! and `fmt`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A copy of the fixture project in a temporary directory, by its canonical root.
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
    (directory, root)
}

/// The exit status, standard output, and standard error of `avenger` with `args` in `directory`.
fn avenger(directory: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_avenger"))
        .args(args)
        .current_dir(directory)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn a_project_without_errors_checks_clean() {
    let (_directory, root) = project();
    let (status, stdout, stderr) = avenger(&root, &["check"]);
    assert_eq!((status, stdout.as_str(), stderr.as_str()), (0, "", ""));
    // A subdirectory of the project, and one of its files, check against the whole project.
    let charts = root.join("charts");
    assert_eq!(avenger(&charts, &["check", "."]).0, 0);
    assert_eq!(avenger(&root, &["check", "charts/origins.av.sql"]).0, 0);
}

#[test]
fn an_error_has_its_place_its_line_and_exit_status_1() {
    let (_directory, root) = project();
    std::fs::write(
        root.join("charts/bad.av.sql"),
        "FROM vega_datasets.cars\n|> SELECT nope\n",
    )
    .unwrap();
    let (status, stdout, stderr) = avenger(&root, &["check"]);
    assert_eq!(status, 1);
    assert_eq!(
        stdout,
        "charts/bad.av.sql:2:11: error: Unrecognized name: nope\n 2 | |> SELECT nope\n   |           ^^^^\n"
    );
    assert_eq!(stderr, "1 error, 0 warnings\n");

    // The same as JSON.
    let (status, stdout, _) = avenger(&root, &["check", "--json"]);
    assert_eq!(status, 1);
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["version"], 1);
    let diagnostic = &value["diagnostics"][0];
    assert_eq!(diagnostic["path"], "charts/bad.av.sql");
    assert_eq!(diagnostic["severity"], "error");
    assert_eq!(diagnostic["source"], "analysis");
    assert_eq!(diagnostic["range"]["start"]["line"], 2);
    assert_eq!(diagnostic["range"]["start"]["column"], 11);
    assert_eq!(diagnostic["range"]["bytes"][0], 34);
}

#[test]
fn a_stylesheet_has_the_errors_of_its_load() {
    let (_directory, root) = project();
    std::fs::write(root.join("styles/main.css"), "mark { fill: ; }\n").unwrap();
    let (status, stdout, _) = avenger(&root, &["check", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let diagnostics = value["diagnostics"].as_array().unwrap();
    assert!(
        (diagnostics.iter()).any(|d| d["path"] == "styles/main.css" && d["source"] == "style"),
        "{stdout}"
    );
    assert_eq!(status, 1, "{stdout}");
}

#[test]
fn run_finds_the_errors_of_a_query_as_it_runs() {
    let (_directory, root) = project();
    let chart = "FROM vega_datasets.cars\n|> SELECT CAST(Name AS INT64) AS n\n|> AGGREGATE mark_text(text => n) AS chart\n";
    std::fs::write(root.join("charts/cast.av.sql"), chart).unwrap();
    assert_eq!(avenger(&root, &["check"]).0, 0);
    let (status, stdout, _) = avenger(&root, &["check", "--run"]);
    assert_eq!(status, 1);
    assert!(
        stdout.starts_with("charts/cast.av.sql: error: the chart's query fails:"),
        "{stdout}"
    );
}

#[test]
fn a_loose_script_checks_alone() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("alone.av.sql"),
        "FROM sales |> SELECT x\n",
    )
    .unwrap();
    let (status, stdout, _) = avenger(directory.path(), &["check"]);
    assert_eq!(status, 1);
    assert!(
        stdout.starts_with("alone.av.sql:1:6: error: Table not found: sales"),
        "{stdout}"
    );
    assert!(
        stdout.contains("= help: The file is in no avenger project"),
        "{stdout}"
    );
    // A path that does not exist, and a file that is no script, can't be checked.
    assert_eq!(avenger(directory.path(), &["check", "nope.av.sql"]).0, 2);
    std::fs::write(directory.path().join("notes.txt"), "").unwrap();
    let (status, _, stderr) = avenger(directory.path(), &["check", "notes.txt"]);
    assert_eq!(status, 2);
    assert!(stderr.contains("not an avenger-sql script"), "{stderr}");
}

#[test]
fn fmt_formats_scripts_in_place_and_check_lists_them() {
    let (_directory, root) = project();
    let chart = root.join("charts/origins.av.sql");
    std::fs::write(
        &chart,
        "FROM heavy_by_origin|>AGGREGATE mark_bar(x=>Origin,y=>cars) AS chart\n",
    )
    .unwrap();
    let (status, stdout, stderr) = avenger(&root, &["fmt", "--check", "charts"]);
    assert_eq!(status, 1);
    assert_eq!(stdout, "charts/origins.av.sql\n");
    assert!(stderr.contains("needs formatting"), "{stderr}");
    assert!(std::fs::read_to_string(&chart)
        .unwrap()
        .contains("|>AGGREGATE"));

    let (status, stdout, _) = avenger(&root, &["fmt", "--json", "charts"]);
    assert_eq!(status, 0);
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["formatted"][0], "charts/origins.av.sql");
    let formatted = std::fs::read_to_string(&chart).unwrap();
    assert_eq!(
        formatted,
        avenger_sql_syntax::format::format(&formatted, avenger_sql_syntax::Role::Script)
    );
    assert_eq!(avenger(&root, &["fmt", "--check", "charts"]).0, 0);
}

/// A document with an error in a cell after a list, in an indented fence.
const REPORT: &str = "# Report\n\n```{avenger-sql}\nFROM heavy_by_origin\n|> AGGREGATE mark_bar(x => Origin, y => cars) AS chart\n```\n\n1. Weather\n\n    ```{avenger-sql}\n    FROM wet_days |> SELECT nope\n    ```\n";

#[test]
fn the_cells_of_a_document_check_at_their_places_in_it() {
    let (_directory, root) = project();
    std::fs::create_dir_all(root.join("reports")).unwrap();
    std::fs::write(root.join("reports/overview.qmd"), REPORT).unwrap();
    let (status, stdout, _) = avenger(&root, &["check"]);
    assert_eq!(status, 1);
    assert_eq!(
        stdout,
        "reports/overview.qmd:11:29: error: Unrecognized name: nope\n 11 |     FROM wet_days |> SELECT nope\n    |                             ^^^^\n"
    );
    let (_, stdout, _) = avenger(&root, &["check", "--json", "reports/overview.qmd"]);
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let range = &value["diagnostics"][0]["range"];
    let start = REPORT.find("nope").unwrap();
    assert_eq!(range["bytes"], serde_json::json!([start, start + 4]));

    // With `--run`, a cell whose query fails has the error at its first line.
    let cast = "```{avenger-sql}\nFROM vega_datasets.cars\n|> SELECT CAST(Name AS INT64) AS n\n|> AGGREGATE mark_text(text => n) AS chart\n```\n";
    std::fs::write(root.join("reports/cast.qmd"), cast).unwrap();
    let (status, stdout, _) = avenger(&root, &["check", "--run", "reports/cast.qmd"]);
    assert_eq!(status, 1);
    assert!(
        stdout.starts_with("reports/cast.qmd:2:1: error: the chart's query fails:"),
        "{stdout}"
    );

    // A document outside every project checks its cells alone.
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("alone.qmd"), REPORT).unwrap();
    let (status, stdout, _) = avenger(directory.path(), &["check"]);
    assert_eq!(status, 1);
    assert!(
        stdout.starts_with("alone.qmd:4:6: error: Table not found: heavy_by_origin"),
        "{stdout}"
    );
}

#[test]
fn an_option_of_a_cell_with_the_wrong_type_checks_at_its_value() {
    let (_directory, root) = project();
    std::fs::create_dir_all(root.join("reports")).unwrap();
    let report = "# Report\n\n```{avenger-sql}\n#| fig-width: wide\nFROM heavy_by_origin\n|> AGGREGATE mark_bar(x => Origin, y => cars) AS chart\n```\n";
    std::fs::write(root.join("reports/sized.qmd"), report).unwrap();
    let (status, stdout, _) = avenger(&root, &["check", "reports/sized.qmd"]);
    assert_eq!(status, 1);
    assert!(
        stdout.starts_with(
            "reports/sized.qmd:4:15: error: `fig-width` is a number of inches, as `fig-width: 6`, more than 0"
        ),
        "{stdout}"
    );
    // A document outside every project checks its options too.
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("alone.qmd"), report).unwrap();
    let (_, stdout, _) = avenger(directory.path(), &["check"]);
    assert!(
        stdout.contains("alone.qmd:4:15: error: `fig-width`"),
        "{stdout}"
    );
}

#[test]
fn fmt_formats_the_cells_of_a_document() {
    let (_directory, root) = project();
    std::fs::create_dir_all(root.join("reports")).unwrap();
    let document = root.join("reports/overview.qmd");
    let text = "Some  prose.\n\n  ```{avenger-sql}\n  FROM wet_days|>SELECT date\n  ```\n";
    std::fs::write(&document, text).unwrap();
    let (status, stdout, _) = avenger(&root, &["fmt", "reports"]);
    assert_eq!((status, stdout.as_str()), (0, "reports/overview.qmd\n"));
    assert_eq!(
        std::fs::read_to_string(&document).unwrap(),
        "Some  prose.\n\n  ```{avenger-sql}\n  FROM wet_days\n  |> SELECT date\n  ```\n"
    );
    assert_eq!(avenger(&root, &["fmt", "--check", "reports"]).0, 0);
}

/// A document whose cells read what earlier cells define: a temporary table of cell 1, read by
/// cells 2, 4, and 5, a variable that cell 3 declares and cell 4 reads, and a table of cell 3
/// with an error, which only cell 5 reads.
const CELLS: &str = "# Cells\n\n```{avenger-sql}\nCREATE TEMP TABLE t AS (FROM UNNEST([1, 2, 3]) AS a)\n```\n\n```{avenger-sql}\n#| label: fig-all\nFROM t |> AGGREGATE mark_bar(x => a, y => a) AS chart\n```\n\n```{avenger-sql}\nDECLARE n INT64 DEFAULT 2;\nCREATE TEMP TABLE broken AS (FROM t |> WHERE nope)\n```\n\n```{avenger-sql}\nFROM t |> WHERE a >= n |> AGGREGATE mark_bar(x => a, y => a) AS chart\n```\n\n```{avenger-sql}\nFROM broken |> AGGREGATE mark_bar(x => a, y => a) AS chart\n```\n";

#[test]
fn the_cells_of_a_document_read_what_earlier_cells_define() {
    let (_directory, root) = project();
    std::fs::create_dir_all(root.join("reports")).unwrap();
    std::fs::write(root.join("reports/cells.qmd"), CELLS).unwrap();
    // The error of cell 3 is the document's, and its `DECLARE` at the start of a cell is none.
    let (status, stdout, _) = avenger(&root, &["check", "reports/cells.qmd"]);
    assert_eq!(status, 1);
    assert!(
        stdout.starts_with("reports/cells.qmd:14:46: error: Unrecognized name: nope"),
        "{stdout}"
    );
    assert!(!stdout.contains("DECLARE"), "{stdout}");
    // The cells with values are listed, and a cell without a query is not.
    let (_, stdout, _) = avenger(&root, &["charts"]);
    // Cell 5's query, which reads cell 3's table, plans with its error.
    for listed in [
        "cells.qmd#fig-all: chart",
        "cells.qmd#4: chart",
        "cells.qmd#5: error",
    ] {
        assert!(stdout.contains(listed), "{stdout}");
    }
    assert!(
        !stdout.contains("cells.qmd#1") && !stdout.contains("cells.qmd#3"),
        "{stdout}"
    );
    // A cell runs with what it reads: cell 4 with cell 1's table and cell 3's variable, and not
    // cell 3's table, whose error stops only cell 5.
    let out = tempfile::tempdir().unwrap();
    let image = |name: &str| out.path().join(name).to_string_lossy().into_owned();
    for (cell, file) in [("fig-all", "two.svg"), ("4", "four.svg")] {
        let target = format!("reports/cells.qmd#{cell}");
        let (status, _, stderr) = avenger(&root, &["render", &target, "-o", &image(file)]);
        assert_eq!(status, 0, "{cell}: {stderr}");
    }
    let (status, stdout, _) = avenger(
        &root,
        &["render", "reports/cells.qmd#5", "-o", &image("five.svg")],
    );
    assert_eq!(status, 1);
    assert!(
        stdout.starts_with(
            "reports/cells.qmd:22:1: error: the cell reads cell 3, whose statement at line 14 has an error: Unrecognized name: nope"
        ),
        "{stdout}"
    );
    let (status, _, stderr) = avenger(
        &root,
        &["render", "reports/cells.qmd#1", "-o", &image("one.svg")],
    );
    assert_eq!(status, 2);
    assert!(
        stderr.contains(
            "cell `1` has no value, since it makes no query; the cells with values are 2, 4, 5"
        ),
        "{stderr}"
    );
}

#[test]
fn a_declare_starts_a_cell() {
    let (_directory, root) = project();
    std::fs::create_dir_all(root.join("reports")).unwrap();
    let late = "```{avenger-sql}\nFROM heavy_by_origin\n|> AGGREGATE mark_bar(x => Origin, y => cars) AS chart;\nDECLARE n INT64 DEFAULT 1;\nFROM heavy_by_origin |> LIMIT n\n```\n";
    std::fs::write(root.join("reports/late.qmd"), late).unwrap();
    let (status, stdout, _) = avenger(&root, &["check", "reports/late.qmd"]);
    assert_eq!(status, 1);
    assert!(
        stdout.starts_with(
            "reports/late.qmd:4:1: error: a DECLARE comes before the other statements of its cell"
        ),
        "{stdout}"
    );
}
