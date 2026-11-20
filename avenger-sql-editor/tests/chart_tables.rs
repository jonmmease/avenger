//! A chart script's own tables of files in the editor: the sidebar lists them under the chart,
//! with their columns once the chart runs and the loader has read their files.

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
