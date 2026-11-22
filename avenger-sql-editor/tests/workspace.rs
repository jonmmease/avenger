//! A project open in the editor, without a window: the fixture project of avenger-sql-project,
//! its files by role in the sidebar with the tables of its data scripts, its tabs, a chart that
//! runs against its tables, a definition in another file, a saved data script that loads again
//! and changes a chart, a new project, added files, and a data file that changes on disk.

use std::path::{Path, PathBuf};

use avenger_sql_editor::add_file::{self, DataFile};
use avenger_sql_editor::assist::{definition, Analysis, Target};
use avenger_sql_editor::result::Shape;
use avenger_sql_editor::workspace::{self, Added, Load, Saved, Status, Watched, Workspace};
use avenger_sql_project::{Change, TableChange};
use avenger_sql_syntax::Role;

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
    assert_eq!(workspace.reruns(&changes, &[]), [ORIGINS]);
    let after = run(&workspace, &chart).await;
    assert_ne!(after.1, before.1, "the chart draws the heavier cars only");

    // A change of a table that no chart reads runs none, and a chart that imports a module runs
    // again after any change, since the module's functions can read tables.
    assert!(workspace.reruns(&[replaced("wet_days")], &[]).is_empty());
    workspace.open_tab(WARM_DAYS);
    assert_eq!(workspace.reruns(&[replaced("wet_days")], &[]), [WARM_DAYS]);
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
    // The error of the chart's run follows the error of the read.
    assert!(error.message.lines().count() > 1, "{}", error.message);
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

/// The tables of the data script at `path` in the sidebar, by their paths.
fn tables(workspace: &Workspace, path: &str) -> Vec<String> {
    let sidebar = workspace.sidebar();
    let (_, state) = (sidebar.data.iter())
        .find(|(script, _)| script == path)
        .unwrap();
    (state.iter().flat_map(|state| &state.tables))
        .map(|table| table.names.join("."))
        .collect()
}

const SALES: &str = "region,amount\nNorth,3\nSouth,5\n";

/// Adds the data files at `paths` to the project, as Add file does: the copies, which the editor
/// makes in the background, and then the statements.
fn add_files(workspace: &mut Workspace, paths: &[PathBuf]) -> Result<Added, String> {
    let copied = workspace.adding(paths)?.copy();
    workspace.add(copied)
}

#[tokio::test]
async fn added_files_become_tables_of_a_data_script() {
    let directory = copy();
    let root = directory.path();
    let mut workspace = open(root).await;
    let outside = tempfile::tempdir().unwrap();
    let sales = outside.path().join("Sales.csv");
    std::fs::write(&sales, SALES).unwrap();
    let points = root.join("data/files/extra/heavy-by-origin.csv");
    std::fs::create_dir_all(points.parent().unwrap()).unwrap();
    std::fs::write(&points, SALES).unwrap();
    let notes = outside.path().join("notes.txt");
    std::fs::write(&notes, "").unwrap();

    // A file outside the root is copied into data/files, and a file under it keeps its place.
    // The statements go into the first data script, which is saved, and a name that the
    // project has takes a number.
    let added = add_files(&mut workspace, &[sales.clone(), points, notes]).unwrap();
    assert_eq!(added.script, REGIONS);
    assert_eq!(added.tables, ["sales", "heavy_by_origin_2"]);
    assert_eq!(added.copied, ["data/files/Sales.csv"]);
    assert_eq!(added.errors.len(), 1);
    assert!(added.errors[0].starts_with("notes.txt has no format"));
    assert_eq!(
        std::fs::read_to_string(root.join("data/files/Sales.csv")).unwrap(),
        SALES
    );
    let text = std::fs::read_to_string(root.join(REGIONS)).unwrap();
    let csv = DataFile::of("x.csv").unwrap();
    let statements = [
        add_file::statement("sales", &csv, "data/files/Sales.csv", Role::Data),
        add_file::statement(
            "heavy_by_origin_2",
            &csv,
            "data/files/extra/heavy-by-origin.csv",
            Role::Data,
        ),
    ];
    let original = std::fs::read_to_string(fixture().join(REGIONS)).unwrap();
    assert_eq!(text, add_file::append(&original, &statements));
    assert_eq!(
        added.load,
        Some(Load::Script {
            path: REGIONS.to_string(),
            text: text.clone()
        })
    );
    assert_eq!(added.run, None);
    let tab = workspace.active().unwrap();
    assert_eq!(tab.path, REGIONS);
    assert!(!tab.modified());

    // The load of the script shows its new tables in the sidebar.
    let (_, state) = workspace.loader().reload(REGIONS, text).await.unwrap();
    workspace.apply(state);
    assert_eq!(
        tables(&workspace, REGIONS),
        ["vega_datasets.regions", "sales", "heavy_by_origin_2"]
    );
    let rows = workspace.loader().run_chart(ORIGINS, "FROM sales |> AGGREGATE SUM(amount) AS total |> AGGREGATE mark_text(text => total) AS chart").await;
    assert!(rows.is_ok(), "{:?}", rows.err().map(|error| error.message));

    // The same file again is a copy of its own, and a tab with edits that are not saved takes
    // the statement and stays modified.
    let edited = format!("{}\n-- An edit.\n", workspace.active().unwrap().buffer.text);
    workspace.edit(REGIONS, &edited);
    let added = add_files(&mut workspace, std::slice::from_ref(&sales)).unwrap();
    assert_eq!(added.tables, ["sales_2"]);
    assert_eq!(added.copied, ["data/files/Sales-2.csv"]);
    assert_eq!(added.load, None);
    let tab = workspace.tab(REGIONS).unwrap();
    assert!(tab.modified());
    assert!(tab.buffer.text.contains("-- An edit."));
    assert!(tab
        .buffer
        .text
        .ends_with("uris = ['data/files/Sales-2.csv']);\n"));
    assert!(!std::fs::read_to_string(root.join(REGIONS))
        .unwrap()
        .contains("Sales-2.csv"));

    // The active data script takes the statements.
    workspace.open_tab(SUMMARIES);
    let added = add_files(&mut workspace, std::slice::from_ref(&sales)).unwrap();
    assert_eq!(added.script, SUMMARIES);
}

#[tokio::test]
async fn a_project_without_data_scripts_gets_one_for_added_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = workspace::create(directory.path()).unwrap();
    let mut workspace = open(&root).await;
    let outside = tempfile::tempdir().unwrap();
    let sales = outside.path().join("sales.csv");
    std::fs::write(&sales, SALES).unwrap();

    let added = add_files(&mut workspace, &[sales]).unwrap();
    assert_eq!(added.script, "data/tables.sql");
    assert_eq!(added.load, Some(Load::All));
    assert_eq!(
        std::fs::read_to_string(root.join("data/tables.sql")).unwrap(),
        "CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/files/sales.csv']);\n"
    );
    assert_eq!(workspace.active().unwrap().path, "data/tables.sql");
    assert!(workspace
        .files()
        .files()
        .contains(&"data/files/sales.csv".to_string()));

    // The load of the whole project finds the new script, and its table.
    let (_, state) = workspace.loader().reload_all().await.unwrap();
    workspace.apply(state);
    assert_eq!(tables(&workspace, "data/tables.sql"), ["sales"]);
}

#[tokio::test]
async fn the_sample_project_takes_no_files() {
    let mut workspace = workspace::sample().await.unwrap();
    let error = add_files(&mut workspace, &[PathBuf::from("sales.csv")]).unwrap_err();
    assert!(error.contains("read-only"), "{error}");
}

#[tokio::test]
async fn a_data_file_that_changes_loads_the_scripts_that_read_it() {
    let directory = copy();
    let root = directory.path();
    let mut workspace = open(root).await;
    workspace.open_tab(ORIGINS);
    let chart = workspace.tab(ORIGINS).unwrap().buffer.text.clone();
    let before = run(&workspace, &chart).await;

    // A file under the root that is not hidden: a data file, or a file of the project's own.
    let regions = root.join("data/files/regions.csv");
    let data = |path: &Path| match workspace.watched(path) {
        Some(Watched::Data(path)) => Some(path),
        _ => None,
    };
    let project = |path: &str| Some(Watched::Project(path.to_string()));
    assert_eq!(data(&regions).as_deref(), Some("data/files/regions.csv"));
    assert_eq!(workspace.watched(&root.join(REGIONS)), project(REGIONS));
    assert_eq!(
        workspace.watched(&root.join("styles/main.css")),
        project("styles/main.css")
    );
    let neither = [
        root.join("avenger.toml"),
        root.join(".git/index"),
        root.join("data/files/.regions.csv.swp"),
        std::env::temp_dir().join("regions.csv"),
    ];
    for path in neither {
        assert_eq!(workspace.watched(&path), None, "{}", path.display());
    }
    // A removed file, whose directory is still there.
    assert_eq!(
        data(&root.join("data/files/gone.csv")).as_deref(),
        Some("data/files/gone.csv")
    );

    std::fs::write(
        &regions,
        "Origin,region\nUSA,North America\nEurope,Europe\nJapan,Asia\n",
    )
    .unwrap();
    // A file that no data script reads loads nothing.
    let notes = data(&root.join("data/files/notes.txt")).unwrap();
    assert!(workspace.loader().reload_files(&[notes]).await.is_none());

    let path = data(&regions).unwrap();
    let (reload, state) = workspace.loader().reload_files(&[path]).await.unwrap();
    let changes = reload.tables;
    assert_eq!(reload.charts, Vec::<String>::new());
    assert_eq!(
        changes,
        [TableChange {
            names: vec!["vega_datasets".to_string(), "regions".to_string()],
            change: Change::Replaced,
        }]
    );
    workspace.apply(state);
    assert_eq!(workspace.reruns(&changes, &[]), [ORIGINS]);
    let after = run(&workspace, &chart).await;
    assert_ne!(after.1, before.1, "the chart names the regions anew");
}

#[tokio::test]
async fn a_file_of_the_project_that_changes_on_disk_is_read_again() {
    let directory = copy();
    let root = directory.path();
    let mut workspace = open(root).await;
    let theirs = |text: &str| format!("{text}\n-- Another program's edit.\n");

    // A script without a tab: the language project analyzes the text on disk, which loads.
    let summaries = theirs(&std::fs::read_to_string(root.join(SUMMARIES)).unwrap());
    std::fs::write(root.join(SUMMARIES), &summaries).unwrap();
    assert_eq!(
        workspace.reread(SUMMARIES),
        Some(Saved::Data {
            path: SUMMARIES.to_string(),
            text: summaries.clone()
        })
    );
    assert_eq!(
        workspace.language().document(SUMMARIES).unwrap().text(),
        summaries
    );
    workspace.open_tab(SUMMARIES);
    assert_eq!(workspace.tab(SUMMARIES).unwrap().buffer.text, summaries);
    // The text that the editor holds is not a change, as after its own save.
    assert_eq!(workspace.reread(SUMMARIES), None);
    workspace.edit(SUMMARIES, &format!("{summaries}-- An edit.\n"));
    workspace.save(SUMMARIES).unwrap();
    assert_eq!(workspace.reread(SUMMARIES), None);

    // A tab without edits that are not saved shows the text on disk, and a chart runs.
    workspace.open_tab(ORIGINS);
    let origins = theirs(&workspace.tab(ORIGINS).unwrap().buffer.text);
    std::fs::write(root.join(ORIGINS), &origins).unwrap();
    assert_eq!(
        workspace.reread(ORIGINS),
        Some(Saved::Chart {
            path: ORIGINS.to_string()
        })
    );
    let tab = workspace.tab(ORIGINS).unwrap();
    assert_eq!(tab.buffer.text, origins);
    assert!(!tab.modified());

    // A tab with edits keeps them, and its save keeps the text on disk.
    let regions = workspace
        .language()
        .document(REGIONS)
        .unwrap()
        .text()
        .to_string();
    workspace.open_tab(REGIONS);
    let edited = format!("{regions}\n-- An edit.\n");
    workspace.edit(REGIONS, &edited);
    std::fs::write(root.join(REGIONS), theirs(&regions)).unwrap();
    assert_eq!(workspace.reread(REGIONS), None);
    assert_eq!(workspace.tab(REGIONS).unwrap().buffer.text, edited);
    assert!(workspace.save(REGIONS).is_err());

    // A stylesheet without a tab restyles.
    std::fs::write(root.join("styles/main.css"), "chart { width: 300px; }\n").unwrap();
    assert_eq!(workspace.reread("styles/main.css"), Some(Saved::Stylesheet));
}

#[tokio::test]
async fn a_saved_module_loads_the_data_scripts_that_import_it() {
    const WEATHER: &str = "modules/weather.sqlm";
    const WARM: &str = "data/warm.sql";
    let directory = copy();
    let root = directory.path();
    let warm = "IMPORT MODULE weather;
CREATE VIEW warm_days AS (FROM vega_datasets.seattle_weather |> WHERE weather.warm(temp_max));
";
    std::fs::write(root.join(WARM), warm).unwrap();
    let mut workspace = open(root).await;
    workspace.open_tab(WEATHER);
    let text = workspace.tab(WEATHER).unwrap().buffer.text.clone();
    let edited = text.replace("t >= 10", "t >= 100");
    assert_ne!(edited, text);
    workspace.edit(WEATHER, &edited);
    let saved = workspace.save(WEATHER).unwrap();
    assert_eq!(
        saved,
        Saved::Module {
            path: WEATHER.to_string(),
            text: edited.clone()
        }
    );
    // The module loads again with the script that imports it, and no other script.
    let (changes, _) = workspace.loader().reload(WEATHER, edited).await.unwrap();
    let replaced = TableChange {
        names: vec!["warm_days".to_string()],
        change: Change::Replaced,
    };
    assert_eq!(changes, [replaced]);
}
