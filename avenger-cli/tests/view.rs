//! The session of `avenger view`, without its window, on copies of avenger-sql-project's fixture
//! project: which changes of the project's files run the chart again, which restyle it, and which
//! do nothing, and the errors of a run that fails.

use std::path::{Path, PathBuf};

use avenger_chart::results::Shape;
use avenger_cli::view::{Outcome, Session};

/// A copy of the fixture project in a temporary directory, by its canonical root, with a
/// document of one labeled cell.
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
    std::fs::create_dir_all(root.join("reports")).unwrap();
    std::fs::write(root.join("reports/overview.qmd"), REPORT).unwrap();
    (directory, root)
}

const REPORT: &str = "# Report\n\n```{avenger-sql}\n--| label: fig-weather\nFROM wet_days |> AGGREGATE mark_bar(x => weather, y => COUNT(*)) AS chart\n```\n";

/// The kinds of `outcomes`, in order.
fn kinds(outcomes: &[Outcome]) -> Vec<&'static str> {
    (outcomes.iter())
        .map(|outcome| match outcome {
            Outcome::Ran(_) => "ran",
            Outcome::Failed(_) => "failed",
            Outcome::Restyled(_) => "restyled",
        })
        .collect()
}

/// Writes `text` to the file `path` of the project at `root`, and gives the session's outcomes.
async fn edit(session: &mut Session, root: &Path, path: &str, text: &str) -> Vec<Outcome> {
    std::fs::write(root.join(path), text).unwrap();
    session.changed(&[root.join(path)]).await
}

#[tokio::test]
async fn the_chart_runs_again_when_its_text_or_what_it_reads_changes() {
    let (_directory, root) = project();
    let chart = root.join("charts/origins.av.sql");
    let (mut session, first) = Session::open(chart.to_str().unwrap()).await.unwrap();
    let Outcome::Ran(result) = &first else {
        panic!("the chart runs when the window opens");
    };
    assert_eq!(result.shape(), Shape::Chart);

    // Its own text.
    let text = "FROM heavy_by_origin\n|> JOIN vega_datasets.regions USING (Origin)\n|> AGGREGATE mark_bar(x => region, y => cars) AS chart\n";
    let outcomes = edit(&mut session, &root, "charts/origins.av.sql", text).await;
    assert_eq!(kinds(&outcomes), ["ran"]);
    // A data script that defines a table that it reads.
    let summaries = std::fs::read_to_string(root.join("data/summaries.av.sql")).unwrap();
    let heavier = summaries.replace("Weight_in_lbs > 3000", "Weight_in_lbs > 4000");
    let outcomes = edit(&mut session, &root, "data/summaries.av.sql", &heavier).await;
    assert_eq!(kinds(&outcomes), ["ran"]);
    // A data file that a data script that it reads reads.
    let outcomes = edit(
        &mut session,
        &root,
        "data/files/regions.csv",
        "Origin,region\nUSA,Americas\n",
    )
    .await;
    assert_eq!(kinds(&outcomes), ["ran"]);
    // Without the join, the chart reads no table of that file, so its change does nothing.
    let text = "FROM heavy_by_origin\n|> AGGREGATE mark_bar(x => Origin, y => cars) AS chart\n";
    edit(&mut session, &root, "charts/origins.av.sql", text).await;
    let outcomes = edit(
        &mut session,
        &root,
        "data/files/regions.csv",
        "Origin,region\nUSA,North America\n",
    )
    .await;
    assert_eq!(kinds(&outcomes), Vec::<&str>::new());
    // The manifest, which opens the project again.
    let manifest = std::fs::read_to_string(root.join("avenger.toml")).unwrap();
    let outcomes = edit(&mut session, &root, "avenger.toml", &manifest).await;
    assert_eq!(kinds(&outcomes), ["restyled", "ran"]);
}

#[tokio::test]
async fn a_stylesheet_restyles_and_another_chart_does_nothing() {
    let (_directory, root) = project();
    let chart = root.join("charts/origins.av.sql");
    let (mut session, _) = Session::open(chart.to_str().unwrap()).await.unwrap();
    let css = std::fs::read_to_string(root.join("styles/main.css")).unwrap();
    let outcomes = edit(
        &mut session,
        &root,
        "styles/main.css",
        &format!("{css}\nmark {{ color: red; }}\n"),
    )
    .await;
    assert_eq!(kinds(&outcomes), ["restyled"]);
    let warm = std::fs::read_to_string(root.join("charts/weather/warm-days.av.sql")).unwrap();
    let outcomes = edit(
        &mut session,
        &root,
        "charts/weather/warm-days.av.sql",
        &format!("{warm}\n"),
    )
    .await;
    assert_eq!(kinds(&outcomes), Vec::<&str>::new());
}

#[tokio::test]
async fn a_run_that_fails_gives_its_errors() {
    let (_directory, root) = project();
    let chart = root.join("charts/origins.av.sql");
    let (mut session, _) = Session::open(chart.to_str().unwrap()).await.unwrap();
    let text = "FROM heavy_by_origin\n|> AGGREGATE mark_bar(x => Origin, y => nope) AS chart\n";
    let outcomes = edit(&mut session, &root, "charts/origins.av.sql", text).await;
    let [Outcome::Failed(errors)] = outcomes.as_slice() else {
        panic!("a failure: {:?}", kinds(&outcomes));
    };
    assert!(
        errors[0].ends_with("2:41: error: Unrecognized name: nope"),
        "{errors:?}"
    );
    // A data script that the chart reads, whose change breaks the chart.
    let fixed = "FROM heavy_by_origin\n|> AGGREGATE mark_bar(x => Origin, y => cars) AS chart\n";
    edit(&mut session, &root, "charts/origins.av.sql", fixed).await;
    let summaries = std::fs::read_to_string(root.join("data/summaries.av.sql")).unwrap();
    let renamed = summaries.replace("AS cars", "AS n");
    let outcomes = edit(&mut session, &root, "data/summaries.av.sql", &renamed).await;
    assert_eq!(kinds(&outcomes), ["failed"]);
}

#[tokio::test]
async fn a_cell_keeps_its_label_as_its_document_changes() {
    let (_directory, root) = project();
    let cell = format!(
        "{}#fig-weather",
        root.join("reports/overview.qmd").display()
    );
    let (mut session, first) = Session::open(&cell).await.unwrap();
    assert_eq!(kinds(&[first]), ["ran"]);
    // A cell before it moves its number, and its label still names it.
    let moved = REPORT.replace(
        "# Report\n\n",
        "# Report\n\n```{avenger-sql}\nFROM wet_days |> SELECT date\n```\n\n",
    );
    let outcomes = edit(&mut session, &root, "reports/overview.qmd", &moved).await;
    let [Outcome::Ran(result)] = outcomes.as_slice() else {
        panic!("a run: {:?}", kinds(&outcomes));
    };
    assert_eq!(result.shape(), Shape::Chart);
}

#[tokio::test]
async fn a_statement_at_a_line_runs_again_when_its_script_changes() {
    let (_directory, root) = project();
    let script = "FROM vega_datasets.cars |> LIMIT 2;\n\nFROM vega_datasets.cars |> AGGREGATE COUNT(*) AS cars\n";
    std::fs::write(root.join("charts/steps.av.sql"), script).unwrap();
    let target = format!("{}:1", root.join("charts/steps.av.sql").display());
    let (mut session, first) = Session::open(&target).await.unwrap();
    let Outcome::Ran(result) = &first else {
        panic!("the statement runs when the window opens");
    };
    assert_eq!(result.num_rows(), 2);
    let edited = script.replace("LIMIT 2", "LIMIT 3");
    let outcomes = edit(&mut session, &root, "charts/steps.av.sql", &edited).await;
    let [Outcome::Ran(result)] = outcomes.as_slice() else {
        panic!("{:?}", kinds(&outcomes));
    };
    assert_eq!(result.num_rows(), 3);
}

#[tokio::test]
async fn a_data_script_s_statement_shows_its_table_again_when_the_script_changes() {
    let (_directory, root) = project();
    let target = format!("{}:7", root.join("data/summaries.av.sql").display());
    let (mut session, first) = Session::open(&target).await.unwrap();
    let Outcome::Ran(result) = &first else {
        panic!("the table shows when the window opens");
    };
    assert_eq!(result.num_rows(), 2);
    let summaries = std::fs::read_to_string(root.join("data/summaries.av.sql")).unwrap();
    let lighter = summaries.replace("Weight_in_lbs > 3000", "Weight_in_lbs > 0");
    let outcomes = edit(&mut session, &root, "data/summaries.av.sql", &lighter).await;
    let [Outcome::Ran(result)] = outcomes.as_slice() else {
        panic!("{:?}", kinds(&outcomes));
    };
    assert_eq!(result.num_rows(), 3);
}

/// A chart script of an earlier query, a temporary table, and its last query.
const STEPS: &str = "FROM vega_datasets.cars |> LIMIT 2;\n\nCREATE TEMP TABLE heavy AS (FROM vega_datasets.cars |> WHERE Weight_in_lbs > 4500);\n\nFROM heavy |> AGGREGATE COUNT(*) AS cars\n";

/// The session of the statement at `line` of `charts/steps.av.sql`, with `STEPS`, and the rows
/// of its first run.
async fn steps(root: &Path, line: usize) -> (Session, usize) {
    std::fs::write(root.join("charts/steps.av.sql"), STEPS).unwrap();
    let target = format!("{}:{line}", root.join("charts/steps.av.sql").display());
    let (session, first) = Session::open(&target).await.unwrap();
    let Outcome::Ran(result) = &first else {
        panic!("the statement runs when the window opens");
    };
    (session, result.num_rows())
}

/// The rows of the one run of `outcomes`.
fn ran(outcomes: &[Outcome]) -> usize {
    match outcomes {
        [Outcome::Ran(result)] => result.num_rows(),
        _ => panic!("a run: {:?}", kinds(outcomes)),
    }
}

#[tokio::test]
async fn a_statement_keeps_its_key_as_lines_come_before_it_and_its_title_follows() {
    let (_directory, root) = project();
    let (mut session, rows) = steps(&root, 1).await;
    assert_eq!(rows, 2);
    assert_eq!(session.title(), "charts/steps.av.sql:1 · query 1");
    let moved = format!("-- Steps.\n-- Toward a chart.\n{STEPS}");
    let outcomes = edit(&mut session, &root, "charts/steps.av.sql", &moved).await;
    assert_eq!(ran(&outcomes), 2);
    assert_eq!(session.title(), "charts/steps.av.sql:3 · query 1");
    // A table keeps its name, and the script's value stays its last query when a query comes
    // before it.
    let (mut table, _) = steps(&root, 3).await;
    assert_eq!(table.title(), "charts/steps.av.sql:3 · table heavy");
    let (mut value, rows) = steps(&root, 5).await;
    assert_eq!(
        (rows, value.title().as_str()),
        (1, "charts/steps.av.sql:5 · result")
    );
    let more = STEPS.replace("\n\nFROM heavy", "\n\nFROM heavy |> LIMIT 1;\n\nFROM heavy");
    let outcomes = edit(&mut value, &root, "charts/steps.av.sql", &more).await;
    assert_eq!(ran(&outcomes), 1);
    assert_eq!(value.title(), "charts/steps.av.sql:7 · result");
    let outcomes = table.changed(&[root.join("charts/steps.av.sql")]).await;
    assert_eq!(table.title(), "charts/steps.av.sql:3 · table heavy");
    assert!(ran(&outcomes) > 1);
}

#[tokio::test]
async fn a_key_that_finds_no_statement_keeps_the_result_and_says_so() {
    let (_directory, root) = project();
    let (mut session, _) = steps(&root, 3).await;
    let renamed = STEPS.replace("heavy", "weighty");
    let outcomes = edit(&mut session, &root, "charts/steps.av.sql", &renamed).await;
    let [Outcome::Failed(errors)] = outcomes.as_slice() else {
        panic!("a failure: {:?}", kinds(&outcomes));
    };
    assert_eq!(
        errors,
        &["`charts/steps.av.sql` has no table `heavy` any more".to_string()]
    );
    // The table comes back, and shows again.
    let outcomes = edit(&mut session, &root, "charts/steps.av.sql", STEPS).await;
    assert!(ran(&outcomes) > 1);
}

#[tokio::test]
async fn a_cell_at_a_line_keeps_its_label_and_its_title_names_it() {
    let (_directory, root) = project();
    let target = format!("{}:5", root.join("reports/overview.qmd").display());
    let (mut session, first) = Session::open(&target).await.unwrap();
    assert_eq!(kinds(&[first]), ["ran"]);
    assert_eq!(
        session.title(),
        "reports/overview.qmd:5 · cell fig-weather · result"
    );
    let moved = REPORT.replace(
        "# Report\n\n",
        "# Report\n\n```{avenger-sql}\nFROM wet_days |> SELECT date\n```\n\n",
    );
    let outcomes = edit(&mut session, &root, "reports/overview.qmd", &moved).await;
    let [Outcome::Ran(result)] = outcomes.as_slice() else {
        panic!("a run: {:?}", kinds(&outcomes));
    };
    assert_eq!(result.shape(), Shape::Chart);
    assert_eq!(
        session.title(),
        "reports/overview.qmd:9 · cell fig-weather · result"
    );
    // A target without a line names its file, and its cell.
    let chart = root.join("charts/origins.av.sql");
    let (plain, _) = Session::open(chart.to_str().unwrap()).await.unwrap();
    assert_eq!(plain.title(), "charts/origins.av.sql");
}
