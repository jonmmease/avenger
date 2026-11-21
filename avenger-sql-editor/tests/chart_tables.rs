//! A chart script's own tables of files in the editor: the sidebar lists them under the chart,
//! with their columns once the chart runs and the loader has read their files, Add file writes
//! them into the active chart, and a change of their files runs the chart again.

use std::path::Path;

use avenger_sql_editor::workspace::{self, Workspace};

const CHART: &str = "charts/own.sql";
const TEXT: &str = "CREATE TEMP EXTERNAL TABLE own_regions
OPTIONS (format = 'CSV', uris = ['data/files/regions.csv']);
FROM own_regions |> AGGREGATE mark_bar(x => region, y => COUNT(*)) AS chart";

/// A project in a temporary directory, with the chart [`CHART`], another chart, and a file of
/// regions.
async fn project() -> (tempfile::TempDir, Workspace) {
    let directory = tempfile::tempdir().unwrap();
    let files = [
        ("avenger.toml", "[project]\nname = \"own\"\nformat = 1\n"),
        (
            "data/files/regions.csv",
            "Origin,region\nUSA,North America\n",
        ),
        ("charts/other.sql", "FROM UNNEST([1]) AS x"),
        (CHART, TEXT),
    ];
    for (path, text) in files {
        let path = directory.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let (workspace, _) = workspace::open(Path::new(directory.path())).await.unwrap();
    (directory, workspace)
}

#[tokio::test]
async fn the_sidebar_lists_a_chart_s_own_tables_under_the_chart() {
    let (_directory, mut workspace) = project().await;
    let names = |workspace: &Workspace| {
        let sidebar = workspace.sidebar();
        assert_eq!(sidebar.chart_tables.keys().collect::<Vec<_>>(), [CHART]);
        let tables = &sidebar.chart_tables[CHART];
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].names, ["own_regions"]);
        (tables[0].columns.iter())
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>()
    };
    // The loader has not read the chart's files before it runs.
    assert!(names(&workspace).is_empty());
    let outcome = workspace.loader().run_chart(CHART, TEXT).await.unwrap();
    assert_eq!(outcome.result.num_rows(), 1);
    workspace.set_file_tables(workspace.loader().file_tables().await);
    assert_eq!(names(&workspace), ["Origin", "region"]);
    // The chart's table is not a table of the session.
    let other = workspace
        .loader()
        .run_chart("charts/other.sql", "FROM own_regions")
        .await;
    assert!(other.is_err());
}

#[tokio::test]
async fn add_file_writes_a_table_of_the_active_chart_before_its_query() {
    let (directory, mut workspace) = project().await;
    let root = directory.path();
    // A file of a name that the chart's table has takes a number.
    let file = root.join("data/files/own_regions.csv");
    std::fs::write(&file, "Origin,region\nJapan,Asia\n").unwrap();
    workspace.open_tab(CHART);
    let copied = workspace.adding(&[file]).unwrap().copy();
    let added = workspace.add(copied).unwrap();
    assert_eq!(added.script, CHART);
    assert_eq!(added.tables, ["own_regions_2"]);
    assert_eq!((added.load, added.run.as_deref()), (None, Some(CHART)));
    let text = std::fs::read_to_string(root.join(CHART)).unwrap();
    let statement = "CREATE TEMP EXTERNAL TABLE own_regions_2\nOPTIONS (format = 'CSV', uris = ['data/files/own_regions.csv']);";
    assert!(
        text.contains(&format!(
            "uris = ['data/files/regions.csv']);\n\n{statement}\nFROM"
        )),
        "{text}"
    );
    // The chart runs with its tables, and no data script took the statement.
    let outcome = workspace.loader().run_chart(CHART, &text).await;
    assert!(
        outcome.is_ok(),
        "{:?}",
        outcome.err().map(|error| error.message)
    );
    assert!(workspace.files().data_scripts().is_empty());
}

#[tokio::test]
async fn a_changed_file_of_a_chart_runs_the_chart_again() {
    let (directory, mut workspace) = project().await;
    workspace.open_tab(CHART);
    workspace.loader().run_chart(CHART, TEXT).await.unwrap();
    std::fs::write(
        directory.path().join("data/files/regions.csv"),
        "Origin,region\nUSA,North America\nJapan,Asia\n",
    )
    .unwrap();
    let path = "data/files/regions.csv".to_string();
    let (reload, state) = workspace.loader().reload_files(&[path]).await.unwrap();
    assert!(reload.tables.is_empty());
    assert_eq!(reload.charts, [CHART]);
    workspace.apply(state);
    assert_eq!(workspace.reruns(&reload.tables, &reload.charts), [CHART]);
    let outcome = workspace.loader().run_chart(CHART, TEXT).await.unwrap();
    assert_eq!(outcome.result.num_rows(), 1);
}
