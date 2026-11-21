//! A project open in the editor, without a window: the fixture project of avenger-sql-project,
//! its files by role in the sidebar with the tables of its data scripts, its tabs, a chart that
//! runs against its tables, a definition in another file, a saved data script that loads again
//! and changes a chart, and a new project.

use std::path::{Path, PathBuf};

use avenger_sql_editor::assist::{definition, Analysis, Target};
use avenger_sql_editor::result::Shape;
use avenger_sql_editor::workspace::{self, Saved, Status, Workspace};
use avenger_sql_project::{Change, TableChange};

const VEGA: &str = "data/vega_datasets.sql";
const REGIONS: &str = "data/regions.sql";
const SUMMARIES: &str = "data/summaries.sql";
const ORIGINS: &str = "charts/origins.sql";
const WARM_DAYS: &str = "charts/weather/warm-days.sql";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../avenger-sql-project/tests/projects/vega")
}

/// A copy of the fixture in a temporary directory, which a test can save files in.
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

async fn open(path: &Path) -> Workspace {
    let (workspace, file) = workspace::open(path).await.unwrap();
    assert_eq!(file, None);
    workspace
}

/// The rows of the chart script `text`, as a run of the tab of a chart shows them, and its chart.
async fn run(workspace: &Workspace, text: &str) -> (usize, String) {
    let outcome = workspace.loader().run_chart("charts/run.sql", text).await;
    let outcome = outcome.unwrap();
    assert_eq!(outcome.result.shape(), Shape::Chart);
    let chart = outcome.result.chart(0, 0).unwrap();
    (outcome.result.num_rows(), format!("{chart:?}"))
}

#[tokio::test]
async fn a_project_opens_from_its_directory_with_its_files_by_role() {
    let workspace = open(&fixture()).await;
    assert_eq!(workspace.name(), "vega");
    let sidebar = workspace.sidebar();

    // The data scripts by path, each with the tables that it registered.
    let data: Vec<(&str, Option<Status>, Vec<String>)> = (sidebar.data.iter())
        .map(|(path, state)| {
            let state = state.as_ref();
            let tables = (state.into_iter())
                .flat_map(|state| &state.tables)
                .map(|table| table.names.join("."))
                .collect();
            (path.as_str(), state.map(|state| state.status), tables)
        })
        .collect();
    assert_eq!(
        data,
        [
            (
                REGIONS,
                Some(Status::Loaded),
                vec!["vega_datasets.regions".to_string()]
            ),
            (
                SUMMARIES,
                Some(Status::Loaded),
                vec!["heavy_by_origin".to_string(), "wet_days".to_string()]
            ),
            (
                VEGA,
                Some(Status::Loaded),
                vec![
                    "vega_datasets.cars".to_string(),
                    "vega_datasets.seattle_weather".to_string()
                ]
            ),
        ]
    );
    // A table's columns, as the sidebar lists them.
    let (_, regions) = &sidebar.data[0];
    let columns: Vec<&str> = (regions.as_ref().unwrap().tables[0].columns.iter())
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(columns, ["Origin", "region"]);

    // The chart scripts by folder, the stylesheets, and the modules.
    assert_eq!(
        sidebar.charts,
        [
            ("charts".to_string(), vec![ORIGINS.to_string()]),
            ("charts/weather".to_string(), vec![WARM_DAYS.to_string()]),
        ]
    );
    assert_eq!(sidebar.stylesheets, ["styles/main.css"]);
    let modules: Vec<(&str, Vec<String>)> = (sidebar.modules.iter())
        .map(|module| (module.path.as_str(), module.names.clone()))
        .collect();
    assert_eq!(
        modules,
        [("modules/weather.sqlm", vec!["weather".to_string()])]
    );

    // The files of the project analyze without errors, with the tables of the load.
    for path in [VEGA, REGIONS, SUMMARIES, ORIGINS, WARM_DAYS] {
        let diagnostics = workspace.language().diagnostics(path);
        assert!(diagnostics.is_empty(), "{path}: {diagnostics:?}");
    }
}

#[tokio::test]
async fn a_file_under_the_root_opens_the_project_and_names_the_file() {
    let (workspace, file) = workspace::open(&fixture().join(WARM_DAYS)).await.unwrap();
    assert_eq!(workspace.name(), "vega");
    assert_eq!(file.as_deref(), Some(WARM_DAYS));

    let empty = tempfile::tempdir().unwrap();
    let error = workspace::open(empty.path()).await.err().unwrap();
    assert!(error.contains("is not in a project"), "{error}");
}

#[tokio::test]
async fn a_chart_tab_runs_against_the_tables_of_the_project() {
    let mut workspace = open(&fixture()).await;
    assert_eq!(workspace.open_tab(ORIGINS), Some(true));
    assert_eq!(workspace.open_tab(WARM_DAYS), Some(true));
    // A tab that is open shows again, and a file that is not a script, a module, or a stylesheet
    // has none.
    assert_eq!(workspace.open_tab(ORIGINS), Some(false));
    assert_eq!(workspace.active().unwrap().path, ORIGINS);
    assert_eq!(workspace.open_tab("data/files/regions.csv"), None);

    for path in [ORIGINS, WARM_DAYS] {
        let text = workspace.tab(path).unwrap().buffer.text.clone();
        let (rows, _) = run(&workspace, &text).await;
        assert_eq!(rows, 1, "{path}");
    }

    // A table of a chart goes to the statement of the data script that creates it.
    let text = &workspace.tab(ORIGINS).unwrap().buffer.text;
    let cursor = text[..text.find("heavy_by_origin").unwrap()]
        .chars()
        .count();
    let analysis = Analysis::file(workspace.language(), ORIGINS).unwrap();
    let Some(Target {
        path: Some(path),
        range,
    }) = definition(analysis, cursor)
    else {
        panic!("a definition in another file");
    };
    assert_eq!(path, SUMMARIES);
    let summaries = workspace.language().document(SUMMARIES).unwrap().text();
    let name: String = summaries
        .chars()
        .skip(range.start)
        .take(range.len())
        .collect();
    assert_eq!(name, "heavy_by_origin");

    // A closed tab shows the one after it, or else the one before.
    workspace.close_tab(ORIGINS).unwrap();
    assert_eq!(workspace.active().unwrap().path, WARM_DAYS);
    workspace.close_tab(WARM_DAYS).unwrap();
    assert!(workspace.active().is_none());
}

#[tokio::test]
async fn a_saved_data_script_loads_again_and_changes_the_charts_that_read_it() {
    let directory = copy();
    let mut workspace = open(directory.path()).await;
    workspace.open_tab(ORIGINS);
    workspace.open_tab(SUMMARIES);
    let chart = workspace.tab(ORIGINS).unwrap().buffer.text.clone();
    let before = run(&workspace, &chart).await;

    // An edit analyzes the tab's text, and marks it modified until it saves.
    let text = workspace.tab(SUMMARIES).unwrap().buffer.text.clone();
    let edited = text.replace("Weight_in_lbs > 3000", "Weight_in_lbs > 4000");
    workspace.edit(SUMMARIES, &edited);
    assert!(workspace.tab(SUMMARIES).unwrap().modified());
    let document = workspace.language().document(SUMMARIES).unwrap();
    assert_eq!(document.text(), edited);

    let saved = workspace.save(SUMMARIES).unwrap();
    assert_eq!(
        saved,
        Saved::Data {
            path: SUMMARIES.to_string(),
            text: edited.clone()
        }
    );
    assert!(!workspace.tab(SUMMARIES).unwrap().modified());
    let on_disk = std::fs::read_to_string(directory.path().join(SUMMARIES)).unwrap();
    assert_eq!(on_disk, edited);

    // The reload changes the tables of the script, and the chart that reads one runs again.
    let (changes, state) = (workspace.loader().reload(SUMMARIES, edited))
        .await
        .unwrap();
    let replaced = |names: &str| TableChange {
        names: names.split('.').map(str::to_string).collect(),
        change: Change::Replaced,
    };
    assert_eq!(changes, [replaced("heavy_by_origin"), replaced("wet_days")]);
    workspace.apply(state);
    assert_eq!(workspace.reruns(&changes), [ORIGINS]);
    let after = run(&workspace, &chart).await;
    assert_ne!(after.1, before.1, "the chart draws the heavier cars only");

    // A change of a table that no chart reads runs none, and a chart that imports a module runs
    // again after any change, since the module's functions can read tables.
    assert!(workspace.reruns(&[replaced("wet_days")]).is_empty());
    workspace.open_tab(WARM_DAYS);
    assert_eq!(workspace.reruns(&[replaced("wet_days")]), [WARM_DAYS]);
}

#[tokio::test]
async fn a_data_script_with_errors_has_the_errors_of_its_load() {
    let directory = copy();
    let mut workspace = open(directory.path()).await;
    workspace.open_tab(VEGA);
    let text = workspace.tab(VEGA).unwrap().buffer.text.clone();
    let broken = text.replace("seattle-weather.csv", "seattle-weather-2013.csv");
    workspace.edit(VEGA, &broken);
    let Saved::Data { path, text } = workspace.save(VEGA).unwrap() else {
        panic!("a data script");
    };
    let (_, state) = workspace.loader().reload(&path, text).await.unwrap();
    workspace.apply(state);

    let statuses: Vec<(String, Option<Status>)> = (workspace.sidebar().data.into_iter())
        .map(|(path, state)| (path, state.map(|state| state.status)))
        .collect();
    assert_eq!(
        statuses,
        [
            (REGIONS.to_string(), Some(Status::NotLoaded)),
            (SUMMARIES.to_string(), Some(Status::NotLoaded)),
            (VEGA.to_string(), Some(Status::Failed)),
        ]
    );
    // The loader's error is at the URIs of the statement, in the tab's text.
    let errors = workspace.language().diagnostics(VEGA);
    let error = (errors.iter())
        .find(|error| error.message.contains("seattle-weather-2013.csv"))
        .unwrap_or_else(|| panic!("an error of the file: {errors:?}"));
    assert_eq!(
        &broken[error.range],
        "['data/files/vega_datasets/seattle-weather-2013.csv']"
    );

    // A chart that reads a table of the script has the error that names it.
    let chart = std::fs::read_to_string(directory.path().join(WARM_DAYS)).unwrap();
    let error = (workspace.loader().run_chart(WARM_DAYS, &chart).await)
        .err()
        .unwrap();
    assert!(
        error
            .message
            .contains("defined by the data script data/vega_datasets.sql"),
        "{}",
        error.message
    );
    let range = error.range.unwrap();
    let read: String = chart.chars().skip(range.start).take(range.len()).collect();
    assert_eq!(read, "vega_datasets.seattle_weather");
}

#[tokio::test]
async fn a_read_of_a_table_that_did_not_load_has_one_error() {
    let directory = copy();
    let mut workspace = open(directory.path()).await;
    workspace.open_tab(VEGA);
    let text = workspace.tab(VEGA).unwrap().buffer.text.clone();
    workspace.edit(VEGA, &text.replace("cars.json", "autos.json"));
    let Saved::Data { path, text } = workspace.save(VEGA).unwrap() else {
        panic!("a data script");
    };
    let (_, state) = workspace.loader().reload(&path, text).await.unwrap();
    workspace.apply(state);

    // The loader's error and the language project's error at the read are one.
    let summaries = std::fs::read_to_string(directory.path().join(SUMMARIES)).unwrap();
    let errors: Vec<String> = (workspace.language().diagnostics(SUMMARIES).into_iter())
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
async fn a_save_keeps_a_change_that_another_program_made_on_disk() {
    let directory = copy();
    let mut workspace = open(directory.path()).await;
    let file = directory.path().join(SUMMARIES);
    // Another program changes the file after the project opened, and the tab's text is the text
    // that the project read.
    let theirs = "CREATE VIEW theirs AS (FROM vega_datasets.cars);\n";
    std::fs::write(&file, theirs).unwrap();
    workspace.open_tab(SUMMARIES);
    let text = workspace.tab(SUMMARIES).unwrap().buffer.text.clone();
    workspace.edit(SUMMARIES, &format!("{text}\n-- An edit.\n"));
    let error = workspace.save(SUMMARIES).unwrap_err();
    assert!(error.contains("changed on disk"), "{error}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), theirs);
    assert!(workspace.tab(SUMMARIES).unwrap().modified());
}

#[tokio::test]
async fn a_new_project_has_the_smallest_manifest() {
    let parent = tempfile::tempdir().unwrap();
    let directory = parent.path().join("Sales 2026");
    std::fs::create_dir(&directory).unwrap();
    let root = workspace::create(&directory).unwrap();
    let manifest = std::fs::read_to_string(root.join("avenger.toml")).unwrap();
    assert_eq!(manifest, "[project]\nname = \"Sales 2026\"\n");

    let workspace = open(&root).await;
    assert_eq!(workspace.name(), "Sales 2026");
    let sidebar = workspace.sidebar();
    assert!(sidebar.data.is_empty() && sidebar.charts.is_empty());

    // A directory that holds a manifest keeps it.
    std::fs::write(root.join("avenger.toml"), "[project]\nname = \"kept\"\n").unwrap();
    workspace::create(&root).unwrap();
    let manifest = std::fs::read_to_string(root.join("avenger.toml")).unwrap();
    assert_eq!(manifest, "[project]\nname = \"kept\"\n");
}
