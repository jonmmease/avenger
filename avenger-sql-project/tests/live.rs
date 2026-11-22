//! A live project: the loader and the analysis of the fixture project kept in step,
//! the loader's errors on the text that loaded, the charts that run again after a change, the
//! scripts that load again after a data file changes, and the cells of documents.

use std::path::{Path, PathBuf};

use avenger_sql_language::Source;
use avenger_sql_project::{
    charts_to_run, Analysis, Change, LoadState, Loader, ProjectFiles, Status, TableChange,
};
use datafusion::prelude::SessionContext;

const VEGA: &str = "data/vega_datasets.av.sql";
const REGIONS: &str = "data/regions.av.sql";
const SUMMARIES: &str = "data/summaries.av.sql";
const ORIGINS: &str = "charts/origins.av.sql";
const WARM_DAYS: &str = "charts/weather/warm-days.av.sql";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects/vega")
}

/// A copy of the fixture in a temporary directory, whose files a test can change.
fn copy() -> tempfile::TempDir {
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
    walk(&fixture(), directory.path());
    directory
}

/// The loader and the analysis of the project at `root`, after its first load.
async fn live(root: &Path) -> (Loader, Analysis, LoadState) {
    let files = ProjectFiles::find(root).await.unwrap();
    let mut analysis = Analysis::new(&files).await.unwrap();
    let (loader, state) = Loader::load(files, SessionContext::new()).await.unwrap();
    analysis.apply(&state);
    (loader, analysis, state)
}

fn read(root: &Path, path: &str) -> String {
    std::fs::read_to_string(root.join(path)).unwrap()
}

fn replaced(names: &str) -> TableChange {
    TableChange {
        names: names.split('.').map(str::to_string).collect(),
        change: Change::Replaced,
    }
}

#[tokio::test]
async fn a_load_gives_the_analysis_its_tables() {
    let directory = copy();
    let (_, analysis, state) = live(directory.path()).await;
    let statuses: Vec<(&str, Status)> = (state.scripts.iter())
        .map(|script| (script.path.as_str(), script.status))
        .collect();
    assert_eq!(
        statuses,
        [
            (VEGA, Status::Loaded),
            (REGIONS, Status::Loaded),
            (SUMMARIES, Status::Loaded)
        ]
    );
    let summaries = (state.scripts.iter())
        .find(|script| script.path == SUMMARIES)
        .unwrap();
    let tables: Vec<String> = (summaries.tables.iter())
        .map(|table| table.names.join("."))
        .collect();
    assert_eq!(tables, ["heavy_by_origin", "wet_days"]);
    // Every file analyzes without errors with the tables of the load.
    for path in analysis.project().files() {
        assert_eq!(analysis.project().diagnostics(path), Vec::new(), "{path}");
    }
}

#[tokio::test]
async fn a_reload_changes_tables_and_runs_again_the_charts_that_read_them() {
    let directory = copy();
    let root = directory.path();
    let (loader, mut analysis, _) = live(root).await;
    let edited = read(root, SUMMARIES).replace("Weight_in_lbs > 3000", "Weight_in_lbs > 4000");
    (analysis.project_mut()).set_texts([(SUMMARIES, edited.clone())]);
    let (changes, state) = loader.reload(SUMMARIES, edited).await.unwrap();
    assert_eq!(changes, [replaced("heavy_by_origin"), replaced("wet_days")]);
    analysis.apply(&state);

    let charts = [
        (ORIGINS, read(root, ORIGINS)),
        (WARM_DAYS, read(root, WARM_DAYS)),
    ];
    let all = || charts.iter().map(|(path, text)| (*path, text.as_str()));
    let schema = loader.default_schema().await;
    // The chart that reads a changed table runs again, and so does the chart that imports a
    // module, since the module's functions can read tables that its text doesn't name.
    assert_eq!(
        charts_to_run(all(), &changes, &[], &schema),
        [ORIGINS, WARM_DAYS]
    );
    let warm = [(ORIGINS, charts[0].1.as_str())];
    assert!(charts_to_run(warm, &[replaced("wet_days")], &[], &schema).is_empty());
    // A table of one name is in the default schema, and a chart whose files changed runs.
    let public = [replaced("public.heavy_by_origin")];
    assert_eq!(charts_to_run(warm, &public, &[], &schema), [ORIGINS]);
    let reading = [ORIGINS.to_string()];
    assert_eq!(charts_to_run(warm, &[], &reading, &schema), [ORIGINS]);
}

#[tokio::test]
async fn the_loader_s_errors_apply_to_the_text_that_loaded() {
    let directory = copy();
    let root = directory.path();
    // A script whose query fails as it runs, which only the loader finds.
    const BOOM: &str = "data/boom.av.sql";
    let boom =
        "CREATE TABLE boom AS (FROM vega_datasets.regions |> SELECT CAST(region AS INT64) AS n);";
    std::fs::write(root.join(BOOM), boom).unwrap();
    let (loader, mut analysis, state) = live(root).await;
    let script = (state.scripts.iter())
        .find(|script| script.path == BOOM)
        .unwrap();
    assert_eq!(script.status, Status::Failed);
    let loader_errors = |analysis: &Analysis| {
        (analysis.project().diagnostics(BOOM).into_iter())
            .filter(|error| error.source == Source::Loader)
            .count()
    };
    assert_eq!(
        loader_errors(&analysis),
        1,
        "{:?}",
        analysis.project().diagnostics(BOOM)
    );

    // An edit since the load moves the ranges, so a state of the old text leaves them out.
    let edited = format!("-- An edit.\n{boom}");
    (analysis.project_mut()).set_texts([(BOOM, edited.clone())]);
    analysis.apply(&state);
    assert_eq!(loader_errors(&analysis), 0);
    // The load of the new text gives them again.
    let (_, state) = loader.reload(BOOM, edited).await.unwrap();
    analysis.apply(&state);
    assert_eq!(loader_errors(&analysis), 1);

    // A script whose files don't exist fails, and the scripts that read it don't load.
    let broken = read(root, VEGA).replace("seattle-weather.csv", "seattle-weather-2013.csv");
    (analysis.project_mut()).set_texts([(VEGA, broken.clone())]);
    let (_, state) = loader.reload(VEGA, broken.clone()).await.unwrap();
    analysis.apply(&state);
    let status = |path: &str| {
        (state.scripts.iter())
            .find(|script| script.path == path)
            .map(|script| script.status)
    };
    assert_eq!(status(VEGA), Some(Status::Failed));
    assert_eq!(status(REGIONS), Some(Status::NotLoaded));
    let error = (analysis.project().diagnostics(VEGA).into_iter())
        .find(|error| error.message.contains("seattle-weather-2013.csv"))
        .expect("the error of the file");
    assert_eq!(
        &broken[error.range],
        "['data/files/vega_datasets/seattle-weather-2013.csv']"
    );
}

#[tokio::test]
async fn a_read_of_a_table_that_did_not_load_has_one_error() {
    let directory = copy();
    let root = directory.path();
    let (loader, mut analysis, _) = live(root).await;
    let broken = read(root, VEGA).replace("cars.json", "autos.json");
    (analysis.project_mut()).set_texts([(VEGA, broken.clone())]);
    let (_, state) = loader.reload(VEGA, broken).await.unwrap();
    analysis.apply(&state);

    // The loader's error and the analysis's error at the read are one.
    let summaries = read(root, SUMMARIES);
    let errors: Vec<String> = (analysis.project().diagnostics(SUMMARIES).into_iter())
        .filter(|error| &summaries[error.range] == "vega_datasets.cars")
        .map(|error| error.message)
        .collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].starts_with("Table vega_datasets.cars is defined by the data script"),
        "{errors:?}"
    );
}

#[tokio::test]
async fn a_changed_data_file_loads_the_scripts_that_read_it() {
    let directory = copy();
    let root = directory.path();
    let (loader, _, _) = live(root).await;
    std::fs::write(
        root.join("data/files/regions.csv"),
        "Origin,region\nUSA,North America\nEurope,Europe\nJapan,Asia\n",
    )
    .unwrap();
    // A file that no data script reads loads nothing.
    let notes = ["data/files/notes.txt".to_string()];
    assert!(loader.reload_files(&notes).await.is_none());

    let regions = ["data/files/regions.csv".to_string()];
    let (reload, state) = loader.reload_files(&regions).await.unwrap();
    assert_eq!(reload.tables, [replaced("vega_datasets.regions")]);
    assert_eq!(reload.charts, Vec::<String>::new());
    let regions = (state.scripts.iter())
        .find(|script| script.path == REGIONS)
        .unwrap();
    assert_eq!(regions.status, Status::Loaded);
    // A chart runs against the new rows.
    let chart = "FROM vega_datasets.regions |> AGGREGATE mark_text(text => region) AS chart";
    assert!(loader.run_chart(ORIGINS, chart).await.is_ok());
}

#[tokio::test]
async fn the_cells_of_a_document_are_chart_scripts_of_the_project() {
    let directory = copy();
    let root = directory.path();
    std::fs::create_dir_all(root.join("reports")).unwrap();
    let text = "# Report\n\n```{avenger-sql}\nFROM heavy_by_origin |> SELECT cars\n```\n";
    std::fs::write(root.join("reports/overview.qmd"), text).unwrap();
    let (_, mut analysis, _) = live(root).await;
    let document = analysis.qmd("reports/overview.qmd").unwrap();
    assert_eq!(document.cells.len(), 1);
    let cell = "reports/overview.qmd#1";
    assert_eq!(analysis.project().diagnostics(cell), Vec::new());

    // An edit of a data script reaches the cell that reads its table.
    let summaries = read(root, SUMMARIES).replace("AS cars", "AS n");
    let reached = analysis.project_mut().set_texts([(SUMMARIES, summaries)]);
    assert!(reached.contains(cell), "{reached:?}");
    let messages: Vec<String> = (analysis.project().diagnostics(cell).into_iter())
        .map(|diagnostic| diagnostic.message)
        .collect();
    assert_eq!(messages, ["Unrecognized name: cars"]);

    // A new text adds a cell, and the result holds the document.
    let two = format!("{text}\n```{{avenger-sql}}\nFROM wet_days\n```\n");
    let reached = analysis.set_document("reports/overview.qmd", Some(two));
    let reached: Vec<&str> = reached.iter().map(String::as_str).collect();
    assert_eq!(reached, ["reports/overview.qmd", "reports/overview.qmd#2"]);
    // Without a text, the document and its cells go.
    let reached = analysis.set_document("reports/overview.qmd", None);
    assert!(reached.contains("reports/overview.qmd#1"), "{reached:?}");
    assert!(analysis.qmd("reports/overview.qmd").is_none());
    assert!(analysis.project().document(cell).is_none());
}
