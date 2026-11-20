//! The sample project: the files that the editor embeds, which are those of `samples/` on disk, a
//! table of the schema `samples` for each
//! sample file, a project that can't be saved, and a project of the user's, which does not see
//! the samples.

use std::path::Path;

use arrow::array::AsArray;
use arrow::datatypes::{DataType, Int64Type};
use avenger_sql_editor::{samples, workspace};
use avenger_sql_project::{FileSource, LoadedProject, ProjectFiles};
use datafusion::prelude::SessionContext;

/// The number of rows of `table` in `project`.
async fn rows(project: &LoadedProject, table: &str) -> i64 {
    let batches = (project.session())
        .sql(&format!("SELECT count(*) AS n FROM {table}"))
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let column = batches[0].column(0).as_primitive::<Int64Type>();
    assert_eq!(column.len(), 1);
    column.value(0)
}

#[test]
fn the_editor_embeds_the_files_of_the_sample_project_and_the_geographic_files() {
    let paths: Vec<&str> = samples::FILES.iter().map(|(path, _)| *path).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted);
    for path in [
        "avenger.toml",
        "data/samples.sql",
        "data/files/README.md",
        "data/files/penguins.csv",
        "data/files/geo/countries.arrows",
        "data/files/geo/earthquakes.arrows",
        "data/files/geo/plate_boundaries.arrows",
        "charts/01-penguins/13-donut-of-species.sql",
    ] {
        assert!(paths.contains(&path), "{path}");
    }
    // Each file of `samples/` on disk, and no hidden file.
    let on_disk = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/data/files/penguins.csv");
    let embedded = samples::FILES
        .iter()
        .find(|(path, _)| *path == "data/files/penguins.csv");
    assert_eq!(embedded.unwrap().1, std::fs::read(on_disk).unwrap());
    assert!(paths
        .iter()
        .all(|path| !path.split('/').any(|name| name.starts_with('.'))));
}

#[tokio::test]
async fn the_folder_of_the_samples_on_disk_is_the_project_that_the_editor_embeds() {
    // The geographic files are in the folder through the link `data/files/geo`.
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples");
    let source = FileSource::directory(folder).unwrap();
    let paths = source.list("").await.unwrap();
    let embedded: Vec<&str> = samples::FILES.iter().map(|(path, _)| *path).collect();
    assert_eq!(paths, embedded);
    for (path, bytes) in samples::FILES {
        assert_eq!(&source.read(path).await.unwrap()[..], *bytes, "{path}");
    }
}

#[tokio::test]
async fn each_sample_is_a_table_of_the_samples_schema() {
    let files = ProjectFiles::open(samples::source().await.unwrap())
        .await
        .unwrap();
    let project = LoadedProject::load(files, SessionContext::new())
        .await
        .unwrap();
    for (table, count) in [
        ("penguins", 344),
        ("cars", 406),
        ("movies", 2259),
        ("iris", 150),
        ("seattle_weather", 1461),
        ("sales", 203),
        ("earthquakes", 3729),
        ("countries", 177),
        ("plate_boundaries", 241),
    ] {
        assert_eq!(
            rows(&project, &format!("samples.{table}")).await,
            count,
            "{table}"
        );
    }
    // An earthquake's point is its longitude and latitude, which `mark_geo_symbol` takes.
    let earthquakes = project
        .session()
        .table("samples.earthquakes")
        .await
        .unwrap();
    for name in ["longitude", "latitude"] {
        let field = earthquakes
            .schema()
            .field_with_unqualified_name(name)
            .unwrap();
        assert_eq!(field.data_type(), &DataType::Float64, "{name}");
    }
    assert!(earthquakes
        .schema()
        .field_with_unqualified_name("geometry")
        .is_err());
}

#[tokio::test]
async fn the_sample_project_runs_its_edits_and_saves_none() {
    let mut workspace = workspace::sample().await.unwrap();
    assert_eq!(workspace.name(), samples::NAME);
    assert!(workspace.read_only() && workspace.root().is_none());
    let path = "charts/01-penguins/03-penguins-per-species.sql";
    assert_eq!(workspace.open_tab(path), Some(true));
    let text = "FROM samples.penguins\n|> WHERE island = 'Dream'\n|> AGGREGATE mark_bar(x => species, y => count(*))\n";
    workspace.edit(path, text);
    let outcome = workspace.loader().run_chart(path, text).await.unwrap();
    assert!(outcome.result.chart(0, 0).is_some());
    let error = workspace.save(path).unwrap_err();
    assert!(error.contains("can't be saved"), "{error}");
    assert!(workspace.tab(path).unwrap().modified());
}

#[tokio::test]
async fn a_project_of_the_user_does_not_see_the_samples() {
    let vega =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../avenger-sql-project/tests/projects/vega");
    let (workspace, _) = workspace::open(&vega).await.unwrap();
    let text = "FROM samples.penguins\n|> AGGREGATE mark_bar(x => species, y => count(*))\n";
    let error = (workspace.loader().run_chart("charts/run.sql", text).await)
        .err()
        .unwrap();
    assert!(
        error.message.contains("samples.penguins"),
        "{}",
        error.message
    );
    let sidebar = workspace.sidebar();
    let tables =
        (sidebar.data.iter()).flat_map(|(_, state)| state.iter().flat_map(|state| &state.tables));
    assert!(tables.into_iter().all(|table| table.names[0] != "samples"));
}
