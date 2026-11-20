//! Loading the fixture project `tests/projects/vega` into a session, over its directory and
//! over embedded copies of its files: the order of its data scripts and the tables that they
//! register, its chart scripts, the failures and the cycles of data scripts, reloads, and the
//! scripts that read a file that changed.

use std::path::{Path, PathBuf};

use avenger_chart::{charts_from_arrow, Decode};
use avenger_sql_project::{
    Change, ChartError, FileSource, LoadedProject, ProjectFiles, ScriptStatus, TableChange,
};
use avenger_sql_syntax::TextRange;
use datafusion::{arrow::util::pretty::pretty_format_batches, prelude::SessionContext};

const VEGA: &str = "data/vega_datasets.sql";
const REGIONS: &str = "data/regions.sql";
const SUMMARIES: &str = "data/summaries.sql";
const ORIGINS: &str = "charts/origins.sql";
const WARM_DAYS: &str = "charts/weather/warm-days.sql";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects/vega")
}

/// The files of the fixture, each a path relative to its root and its text.
fn fixture_files() -> Vec<(String, String)> {
    fn walk(root: &Path, directory: &Path, files: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, files);
            } else {
                let relative = path.strip_prefix(root).unwrap().to_str().unwrap();
                let text = std::fs::read_to_string(&path).unwrap();
                files.push((relative.replace('\\', "/"), text));
            }
        }
    }
    let mut files = Vec::new();
    walk(&fixture(), &fixture(), &mut files);
    files.sort();
    files
}

/// The fixture loaded from its directory.
async fn directory() -> LoadedProject {
    let source = FileSource::directory(fixture()).unwrap();
    load(source).await
}

/// The fixture loaded from embedded copies of its files, with the text of each path of
/// `changes` in place of the fixture's, or without the file for `None`.
async fn embedded(changes: &[(&str, Option<&str>)]) -> LoadedProject {
    let mut files = fixture_files();
    for (path, text) in changes {
        files.retain(|(file, _)| file != path);
        if let Some(text) = text {
            files.push((path.to_string(), text.to_string()));
        }
    }
    load(FileSource::embedded("vega", files).await.unwrap()).await
}

async fn load(source: FileSource) -> LoadedProject {
    let files = ProjectFiles::open(source).await.unwrap();
    LoadedProject::load(files, SessionContext::new())
        .await
        .unwrap()
}

/// The paths of the scripts of `project` in the order of the load, with their statuses.
fn statuses(project: &LoadedProject) -> Vec<(String, &'static str)> {
    (project.scripts())
        .map(|script| {
            let status = match &script.status {
                ScriptStatus::Loaded(_) => "loaded",
                ScriptStatus::Failed => "failed",
                ScriptStatus::NotLoaded { .. } => "not loaded",
                ScriptStatus::Blocked => "blocked",
            };
            (script.path.clone(), status)
        })
        .collect()
}

/// The paths of the tables, views, and schemas that the script at `path` registered.
fn registered(project: &LoadedProject, path: &str) -> Vec<String> {
    let script = project.script(path).unwrap().loaded().unwrap();
    (script.registered.iter())
        .map(|registered| {
            script.analysis.definitions[registered.definition]
                .names
                .join(".")
        })
        .collect()
}

/// The messages of the diagnostics of the script at `path`, each with the text of its range.
fn diagnostics(project: &LoadedProject, path: &str) -> Vec<(String, String)> {
    let text = project.text(path).unwrap();
    (project.script(path).unwrap().diagnostics.iter())
        .map(|diagnostic| {
            assert_eq!(diagnostic.path, path);
            (at(text, diagnostic.range), diagnostic.message.clone())
        })
        .collect()
}

fn at(text: &str, range: TextRange) -> String {
    text[range].to_string()
}

/// The text of the chart script at `path` of the fixture.
fn chart(path: &str) -> String {
    std::fs::read_to_string(fixture().join(path)).unwrap()
}

/// The tables of the chart of the chart script `text`, as rows in the order of their text.
async fn chart_tables(project: &LoadedProject, text: &str) -> String {
    let frame = project.run_chart("charts/run.sql", text).await.unwrap();
    let batches = frame.collect().await.unwrap();
    let mut tables = Vec::new();
    for batch in batches {
        let column = batch.column_by_name("chart").expect("a chart column");
        for chart in charts_from_arrow(column, Decode::Auto).unwrap() {
            let chart = chart.unwrap();
            let text = pretty_format_batches(chart.tables()).unwrap().to_string();
            // The rows of a chart's table are in no order.
            let mut lines: Vec<&str> = text.lines().collect();
            let end = lines.len().saturating_sub(1);
            lines[3.min(end)..end].sort();
            tables.push(lines.join("\n"));
        }
    }
    assert_eq!(tables.len(), 1, "one chart");
    tables.remove(0)
}

/// The rows of the query `query` in the session of `project`.
async fn rows(project: &LoadedProject, query: &str) -> String {
    let frame = avenger_sql::sql(project.session(), query).await.unwrap();
    let batches = frame.collect().await.unwrap();
    pretty_format_batches(&batches).unwrap().to_string()
}

/// `text` without the white space at the start of its lines, as a table of rows prints.
fn table(text: &str) -> String {
    (text.trim().lines())
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
}

fn change(names: &str, change: Change) -> TableChange {
    TableChange {
        names: names.split('.').map(str::to_string).collect(),
        change,
    }
}

#[tokio::test]
async fn data_scripts_load_in_the_order_of_their_dependencies() {
    let project = directory().await;
    assert_eq!(
        statuses(&project),
        [(VEGA, "loaded"), (REGIONS, "loaded"), (SUMMARIES, "loaded")]
            .map(|(path, status)| (path.to_string(), status))
    );
    assert!(project.graph().errors().is_empty());
    assert_eq!(
        registered(&project, VEGA),
        [
            "vega_datasets",
            "vega_datasets.cars",
            "vega_datasets.seattle_weather"
        ]
    );
    // A table in the schema of another script, which it reads no table of.
    assert_eq!(registered(&project, REGIONS), ["vega_datasets.regions"]);
    // The TEMP step registers nothing.
    assert_eq!(
        registered(&project, SUMMARIES),
        ["heavy_by_origin", "wet_days"]
    );
    assert!(project
        .session()
        .table_exist("heavy")
        .is_ok_and(|exists| !exists));
    assert_eq!(
        rows(&project, "FROM heavy_by_origin |> ORDER BY Origin").await,
        table(
            "+--------+------+
             | Origin | cars |
             +--------+------+
             | Europe | 1    |
             | USA    | 2    |
             +--------+------+"
        )
    );
    assert_eq!(
        rows(&project, "FROM wet_days |> AGGREGATE COUNT(*) AS days").await,
        table(
            "+------+
             | days |
             +------+
             | 5    |
             +------+"
        )
    );
}

#[tokio::test]
async fn chart_scripts_run_against_the_tables_of_the_data_scripts() {
    let project = directory().await;
    assert_eq!(project.files().chart_scripts(), [ORIGINS, WARM_DAYS]);
    assert_eq!(
        chart_tables(&project, &chart(ORIGINS)).await,
        table(
            "+----------+------+
             | region   | cars |
             +----------+------+
             | Americas | 2    |
             | Europe   | 1    |
             +----------+------+"
        )
    );
    // The chart imports the project's module `weather`.
    let warm = chart_tables(&project, &chart(WARM_DAYS)).await;
    assert_eq!(warm.lines().count(), 4 + 4, "{warm}");
    // And it lifts with the module.
    let lifted = project.lift(WARM_DAYS, &chart(WARM_DAYS)).await.unwrap();
    assert!(
        lifted.text.contains("weather.warm(temp_max)"),
        "{}",
        lifted.text
    );
}

#[tokio::test]
async fn an_embedded_project_loads_as_its_directory_does() {
    let (on_disk, in_memory) = (directory().await, embedded(&[]).await);
    assert_eq!(statuses(&on_disk), statuses(&in_memory));
    assert!(in_memory
        .files()
        .source()
        .base()
        .as_str()
        .starts_with("memory://vega/"));
    for path in [ORIGINS, WARM_DAYS] {
        assert_eq!(
            chart_tables(&on_disk, &chart(path)).await,
            chart_tables(&in_memory, &chart(path)).await
        );
    }
}

#[tokio::test]
async fn a_failed_data_script_names_itself_downstream_and_in_charts() {
    let vega = std::fs::read_to_string(fixture().join(VEGA))
        .unwrap()
        .replace("seattle-weather.csv", "seattle-weather-2013.csv");
    let project = embedded(&[(VEGA, Some(&vega))]).await;
    assert_eq!(
        statuses(&project),
        [
            (VEGA, "failed"),
            (REGIONS, "not loaded"),
            (SUMMARIES, "not loaded")
        ]
        .map(|(path, status)| (path.to_string(), status))
    );
    let errors = diagnostics(&project, VEGA);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(
        errors[0].0,
        "['data/files/vega_datasets/seattle-weather-2013.csv']"
    );
    assert!(
        errors[0].1.contains("seattle-weather-2013.csv"),
        "{}",
        errors[0].1
    );
    // The failed script registers nothing, not even the tables before its error.
    assert!(project.session().table_exist("vega_datasets.cars").is_err());

    // A script that reads a table of the failed script has an error at its read.
    assert_eq!(
        diagnostics(&project, SUMMARIES),
        [(
            "vega_datasets.cars".to_string(),
            "Table vega_datasets.cars is defined by the data script data/vega_datasets.sql, which failed to load".to_string()
        )]
    );
    assert_eq!(
        diagnostics(&project, REGIONS),
        [(
            "vega_datasets".to_string(),
            "Schema vega_datasets is created by the data script data/vega_datasets.sql, which failed to load".to_string()
        )]
    );
    let ScriptStatus::NotLoaded { dependencies } = &project.script(SUMMARIES).unwrap().status
    else {
        panic!("summaries is not loaded");
    };
    assert_eq!(dependencies, &[VEGA]);

    // A chart that reads a table of a script that did not load names the script.
    let Err(ChartError::Unloaded { reads, .. }) =
        project.run_chart(WARM_DAYS, &chart(WARM_DAYS)).await
    else {
        panic!("the chart reads a table that did not load");
    };
    let text = chart(WARM_DAYS);
    let found: Vec<(String, &str, &str)> = (reads.iter())
        .map(|read| {
            (
                at(&text, read.range),
                read.script.as_str(),
                read.message.as_str(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [(
            "vega_datasets.seattle_weather".to_string(),
            VEGA,
            "Table vega_datasets.seattle_weather is defined by the data script data/vega_datasets.sql, which failed to load"
        )]
    );
    let error = project
        .run_chart(ORIGINS, &chart(ORIGINS))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Table heavy_by_origin is defined by the data script data/summaries.sql, which did not load, since data/vega_datasets.sql failed to load\n\
         Table vega_datasets.regions is defined by the data script data/regions.sql, which did not load, since data/vega_datasets.sql failed to load"
    );
}

#[tokio::test]
async fn the_file_tables_of_the_scripts_are_those_of_their_last_loads() {
    // The table of each set of files that a script read, by its URIs, and whether it read.
    let tables = |project: &LoadedProject| -> Vec<(Vec<String>, bool)> {
        (project.file_tables().into_iter())
            .map(|(files, table)| (files.uris, table.is_ok()))
            .collect()
    };
    let uris = |uri: &str| vec![uri.to_string()];
    let project = directory().await;
    assert_eq!(
        tables(&project),
        [
            (uris("data/files/vega_datasets/cars.json"), true),
            (uris("data/files/vega_datasets/seattle-weather.csv"), true),
            (uris("data/files/regions.csv"), true),
        ]
    );

    // A failed script keeps the error of the files that it can't read, and a script that did
    // not run read none.
    let vega = std::fs::read_to_string(fixture().join(VEGA))
        .unwrap()
        .replace("seattle-weather.csv", "seattle-weather-2013.csv");
    let project = embedded(&[(VEGA, Some(&vega))]).await;
    assert_eq!(
        tables(&project),
        [
            (uris("data/files/vega_datasets/cars.json"), true),
            (
                uris("data/files/vega_datasets/seattle-weather-2013.csv"),
                false
            ),
        ]
    );
}

#[tokio::test]
async fn a_chart_that_fails_for_another_reason_has_the_error_of_its_run() {
    let project = directory().await;
    let run = project.run_chart("charts/run.sql", "FROM no_such_table");
    let error = run.await.unwrap_err();
    assert!(matches!(error, ChartError::Run(_)), "{error}");
}

#[tokio::test]
async fn data_scripts_in_a_cycle_are_blocked() {
    let first = "CREATE TABLE first AS (FROM second);";
    let second = "CREATE TABLE second AS (FROM first);";
    let third = "CREATE VIEW third AS (FROM second);";
    // A reader that comes before the cycle by path, and loads before it.
    let reader = "CREATE VIEW reader AS (FROM first);";
    let project = embedded(&[
        ("data/cycle/first.sql", Some(first)),
        ("data/cycle/second.sql", Some(second)),
        ("data/cycle/third.sql", Some(third)),
        ("data/a_reader.sql", Some(reader)),
    ])
    .await;
    let blocked: Vec<_> = (statuses(&project).into_iter())
        .filter(|(_, status)| *status == "blocked")
        .collect();
    assert_eq!(
        blocked,
        [
            "data/a_reader.sql",
            "data/cycle/first.sql",
            "data/cycle/second.sql",
            "data/cycle/third.sql"
        ]
        .map(|path| (path.to_string(), "blocked"))
    );
    // The scripts that the cycle does not reach load.
    assert!(project.script(SUMMARIES).unwrap().loaded().is_some());
    assert_eq!(project.graph().errors().len(), 1);
    let cycle = project.graph().errors()[0].message();
    assert_eq!(
        diagnostics(&project, "data/cycle/first.sql"),
        [("second".to_string(), cycle.clone())]
    );
    assert_eq!(
        diagnostics(&project, "data/cycle/second.sql"),
        [("first".to_string(), cycle)]
    );
    assert_eq!(
        diagnostics(&project, "data/cycle/third.sql"),
        [(
            "second".to_string(),
            "Table second is defined by the data script data/cycle/second.sql, which can't run, since it is in a cycle of data scripts that read each other's tables, or reads one".to_string()
        )]
    );
    assert_eq!(
        diagnostics(&project, "data/a_reader.sql"),
        [(
            "first".to_string(),
            "Table first is defined by the data script data/cycle/first.sql, which can't run, since it is in a cycle of data scripts that read each other's tables, or reads one".to_string()
        )]
    );
}

#[tokio::test]
async fn a_lowering_error_is_at_its_statement() {
    let load = "LOAD DATA OVERWRITE codes (Origin INT64, region STRING)\n\
                FROM FILES (format = 'CSV', uris = ['data/files/regions.csv'], skip_leading_rows = 1);";
    let reader = "CREATE VIEW code_view AS (FROM codes);";
    let project = embedded(&[
        ("data/codes.sql", Some(load)),
        ("data/code_view.sql", Some(reader)),
    ])
    .await;
    assert!(matches!(
        project.script("data/codes.sql").unwrap().status,
        ScriptStatus::Failed
    ));
    let errors = diagnostics(&project, "data/codes.sql");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].0, load.trim_end_matches(';'));
    assert!(
        errors[0].1.contains("'USA' as type 'Int64'"),
        "{}",
        errors[0].1
    );
    assert!(project
        .session()
        .table_exist("codes")
        .is_ok_and(|exists| !exists));
    assert!(matches!(
        project.script("data/code_view.sql").unwrap().status,
        ScriptStatus::NotLoaded { .. }
    ));
}

#[tokio::test]
async fn a_name_that_two_data_scripts_define_loads_neither() {
    let other = "CREATE VIEW heavy_by_origin AS (FROM vega_datasets.regions);";
    let project = embedded(&[("data/other.sql", Some(other))]).await;
    assert_eq!(project.graph().errors().len(), 1);
    let message = project.graph().errors()[0].message();
    for path in ["data/other.sql", SUMMARIES] {
        assert!(matches!(
            project.script(path).unwrap().status,
            ScriptStatus::Failed
        ));
        assert_eq!(
            diagnostics(&project, path),
            [("heavy_by_origin".to_string(), message.clone())]
        );
    }
    assert!(project
        .session()
        .table_exist("heavy_by_origin")
        .is_ok_and(|exists| !exists));
}

#[tokio::test]
async fn a_reload_changes_a_table_and_the_charts_that_read_it() {
    let mut project = embedded(&[]).await;
    let summaries = project
        .text(SUMMARIES)
        .unwrap()
        .replace("Weight_in_lbs > 3000", "Weight_in_lbs > 2000");
    let changes = project.reload(SUMMARIES, summaries).await.unwrap();
    assert_eq!(
        changes,
        [
            change("heavy_by_origin", Change::Replaced),
            change("wet_days", Change::Replaced)
        ]
    );
    assert_eq!(
        chart_tables(&project, &chart(ORIGINS)).await,
        table(
            "+----------+------+
             | region   | cars |
             +----------+------+
             | Americas | 2    |
             | Asia     | 2    |
             | Europe   | 2    |
             +----------+------+"
        )
    );

    // A reload of a script reloads the scripts downstream of it.
    let vega = project.text(VEGA).unwrap().to_string();
    let changes = project.reload(VEGA, vega).await.unwrap();
    assert_eq!(
        changes,
        [
            change("heavy_by_origin", Change::Replaced),
            change("vega_datasets.cars", Change::Replaced),
            change("vega_datasets.regions", Change::Replaced),
            change("vega_datasets.seattle_weather", Change::Replaced),
            change("wet_days", Change::Replaced),
        ]
    );
    assert_eq!(
        chart_tables(&project, &chart(ORIGINS))
            .await
            .lines()
            .count(),
        7
    );
}

#[tokio::test]
async fn a_reload_removes_the_tables_that_a_script_no_longer_defines() {
    let mut project = embedded(&[]).await;
    let text = project.text(SUMMARIES).unwrap();
    let summaries = text[..text.find("CREATE VIEW").unwrap()].to_string()
        + "CREATE VIEW dry_days AS (FROM vega_datasets.seattle_weather |> WHERE precipitation = 0);";
    let changes = project.reload(SUMMARIES, summaries).await.unwrap();
    assert_eq!(
        changes,
        [
            change("dry_days", Change::Registered),
            change("heavy_by_origin", Change::Replaced),
            change("wet_days", Change::Removed),
        ]
    );
    assert!(project
        .session()
        .table_exist("wet_days")
        .is_ok_and(|exists| !exists));
    assert_eq!(
        rows(&project, "FROM dry_days |> AGGREGATE COUNT(*) AS days").await,
        table(
            "+------+
             | days |
             +------+
             | 2    |
             +------+"
        )
    );

    // A schema that a script no longer creates leaves the session, with its tables and the
    // scripts that read it.
    let changes = project
        .reload(VEGA, "LOAD DATA OVERWRITE unrelated FROM FILES (format = 'CSV', uris = ['data/files/regions.csv']);".into())
        .await
        .unwrap();
    assert_eq!(
        changes,
        [
            change("dry_days", Change::Removed),
            change("heavy_by_origin", Change::Removed),
            change("unrelated", Change::Registered),
            change("vega_datasets.cars", Change::Removed),
            change("vega_datasets.regions", Change::Removed),
            change("vega_datasets.seattle_weather", Change::Removed),
        ]
    );
    let catalog = project.session().catalog("datafusion").unwrap();
    assert!(!catalog
        .schema_names()
        .contains(&"vega_datasets".to_string()));
    assert!(matches!(
        project.script(REGIONS).unwrap().status,
        ScriptStatus::Failed
    ));
    let error = project.reload("data/missing.sql", String::new()).await;
    assert!(error.is_err());
}

#[tokio::test]
async fn a_project_on_disk_reloads_a_script_and_the_whole_project() {
    let root = tempfile::tempdir().unwrap();
    for (path, text) in fixture_files() {
        let path = root.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let mut project = load(FileSource::directory(root.path()).unwrap()).await;

    // A data file changes on disk, and the data script that reads it loads again.
    let regions = root.path().join("data/files/regions.csv");
    std::fs::write(
        &regions,
        "Origin,region\nUSA,North America\nEurope,Europe\n",
    )
    .unwrap();
    let text = std::fs::read_to_string(root.path().join(REGIONS)).unwrap();
    let changes = project.reload(REGIONS, text).await.unwrap();
    assert_eq!(changes, [change("vega_datasets.regions", Change::Replaced)]);
    assert!(chart_tables(&project, &chart(ORIGINS))
        .await
        .contains("North America"));

    // A new data script, which the whole project's reload finds.
    let extra = root.path().join("data/extra.sql");
    std::fs::write(&extra, "CREATE VIEW extra AS (FROM vega_datasets.regions);").unwrap();
    let changes = project.reload_all().await.unwrap();
    assert!(
        changes.contains(&change("extra", Change::Registered)),
        "{changes:?}"
    );
    assert!(
        changes.contains(&change("wet_days", Change::Replaced)),
        "{changes:?}"
    );
    assert!(project.script("data/extra.sql").unwrap().loaded().is_some());
    assert_eq!(project.session().catalog_names().len(), 1);
}

/// A data script that reads the CSV files of a pattern and of a directory.
const PARTS: &str = "data/parts.sql";
const PARTS_TEXT: &str = "CREATE EXTERNAL TABLE parts
OPTIONS (format = 'CSV', uris = ['data/files/parts/*.csv']);

CREATE EXTERNAL TABLE folder
OPTIONS (format = 'CSV', uris = ['data/files/folder/']);
";

#[tokio::test]
async fn a_file_maps_to_the_data_scripts_whose_uris_read_it() {
    let part = "Origin,count\nUSA,1\n";
    let project = embedded(&[
        (PARTS, Some(PARTS_TEXT)),
        ("data/files/parts/a.csv", Some(part)),
        ("data/files/folder/a.csv", Some(part)),
    ])
    .await;
    let reading = |path: &str| project.scripts_reading(path);

    // A URI of one file.
    assert_eq!(reading("data/files/regions.csv"), [REGIONS]);
    assert_eq!(reading("data/files/vega_datasets/cars.json"), [VEGA]);
    // A file of a pattern or of a directory, which no load listed, and a file that a pattern
    // or a directory does not hold: another name, another extension, or a subdirectory.
    assert_eq!(reading("data/files/parts/b.csv"), [PARTS]);
    assert_eq!(reading("data/files/folder/b.csv"), [PARTS]);
    assert!(reading("data/files/parts/b.json").is_empty());
    assert!(reading("data/files/folder/b.txt").is_empty());
    assert!(reading("data/files/parts/old/b.csv").is_empty());
    // A name with `=`, and a subdirectory that looks like a partition of a Hive layout, which a
    // table without partition columns does not read.
    assert_eq!(reading("data/files/parts/ratio=1.csv"), [PARTS]);
    assert_eq!(reading("data/files/folder/ratio=1.csv"), [PARTS]);
    assert!(reading("data/files/parts/year=2025/b.csv").is_empty());
    assert!(reading("data/files/folder/year=2025/b.csv").is_empty());
    assert!(reading("data/files/vega_datasets/other.csv").is_empty());
    // A script, the manifest, and a path that is not relative to the root.
    assert!(reading(REGIONS).is_empty());
    assert!(reading("avenger.toml").is_empty());
    assert!(reading("../regions.csv").is_empty());
}

#[tokio::test]
async fn a_project_on_disk_reloads_the_scripts_that_read_changed_files() {
    let root = tempfile::tempdir().unwrap();
    let mut files = fixture_files();
    files.push((PARTS.to_string(), PARTS_TEXT.to_string()));
    files.push((
        "data/files/folder/a.csv".to_string(),
        "Origin,count\nUSA,1\n".to_string(),
    ));
    for (path, text) in files {
        let path = root.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let mut project = load(FileSource::directory(root.path()).unwrap()).await;
    // No file matches the pattern, so the script fails, and it reads the files of the pattern.
    assert!(project.script(PARTS).unwrap().loaded().is_none());
    assert_eq!(project.scripts_reading("data/files/parts/a.csv"), [PARTS]);

    // A new file of the pattern loads the script.
    let part = root.path().join("data/files/parts/a.csv");
    std::fs::create_dir_all(part.parent().unwrap()).unwrap();
    std::fs::write(&part, "Origin,count\nUSA,2\n").unwrap();
    let changes = project
        .reload_files(&["data/files/parts/a.csv"])
        .await
        .tables;
    assert_eq!(
        changes,
        [
            change("folder", Change::Registered),
            change("parts", Change::Registered)
        ]
    );
    assert_eq!(
        rows(&project, "FROM parts |> SELECT count").await,
        table(
            "+-------+
             | count |
             +-------+
             | 2     |
             +-------+"
        )
    );

    // Changes of the files of two scripts reload each once, and a file that no script reads
    // reloads none.
    std::fs::write(
        root.path().join("data/files/regions.csv"),
        "Origin,region\nUSA,North America\n",
    )
    .unwrap();
    let changes = project
        .reload_files(&[
            "data/files/regions.csv",
            "data/files/parts/a.csv",
            "data/files/notes.txt",
        ])
        .await
        .tables;
    assert_eq!(
        changes,
        [
            change("folder", Change::Replaced),
            change("parts", Change::Replaced),
            change("vega_datasets.regions", Change::Replaced),
        ]
    );
    let reload = project.reload_files(&["data/files/notes.txt"]).await;
    assert_eq!(reload, Default::default());

    // A removed file fails the script again, which removes its tables.
    std::fs::remove_file(&part).unwrap();
    let changes = project
        .reload_files(&["data/files/parts/a.csv"])
        .await
        .tables;
    assert_eq!(
        changes,
        [
            change("folder", Change::Removed),
            change("parts", Change::Removed)
        ]
    );
}

#[test]
fn a_loaded_project_moves_between_threads() {
    fn send_and_sync<T: Send + Sync>() {}
    send_and_sync::<LoadedProject>();
}
