//! The gallery of a project, and the charts of the sample project: how a chart script's path and
//! text give its section, title, and description, and that every chart of the sample project runs
//! against its tables, draws each of its charts at the size of a thumbnail, lifts to itself, is in
//! the style, and has the depth at each line that Enter and Tab give it.

use avenger_sql_editor::charts::{engine, ChartTheme, StaticChart};
use avenger_sql_editor::gallery::{description, entries, heading, title, Entry};
use avenger_sql_editor::workspace::{self, Workspace};
use avenger_sql_project::Manifest;

/// The size of a thumbnail's evaluation, in logical pixels.
const THUMBNAIL: [f32; 2] = [480.0, 300.0];

/// The sections of the sample project, in the order of their folders.
const SECTIONS: [&str; 12] = [
    "Penguins",
    "Cars",
    "Movies",
    "Iris",
    "Seattle weather",
    "Sales",
    "Maps",
    "Variables",
    "Grouped charts",
    "Composition",
    "Math",
    "Pan zoom and tooltips",
];

async fn sample() -> Workspace {
    workspace::sample().await.unwrap()
}

#[test]
fn a_title_is_the_name_with_spaces_and_a_capital() {
    assert_eq!(title("donut-of-species"), "Donut of species");
    assert_eq!(title("élan"), "Élan");
    // The case of the other letters stays.
    assert_eq!(title("MPG-vs-weight"), "MPG vs weight");
    // A leading number and `-` of a folder or a file order it, and are not part of its heading
    // or title, unless nothing follows them.
    assert_eq!(heading("01-penguins"), "Penguins");
    assert_eq!(heading("12-pan-zoom-and-tooltips"), "Pan zoom and tooltips");
    assert_eq!(heading("2024"), "2024");
    assert_eq!(heading("3d-scatter"), "3d scatter");
    assert_eq!(heading("02-MPG-vs-weight"), "MPG vs weight");
}

#[test]
fn a_description_is_the_comment_block_at_the_start() {
    let text = "\
-- The number of each species,
-- as bars.
--
-- A second paragraph.
FROM samples.penguins
-- Not the description.
|> AGGREGATE mark_bar(x => species, y => count(*))
";
    assert_eq!(
        description(text),
        "The number of each species, as bars.\n\nA second paragraph."
    );
    assert_eq!(
        description("FROM samples.penguins\n-- After the query.\n"),
        ""
    );
    assert_eq!(
        description("--No space\n--   Indented\nFROM t"),
        "No space Indented"
    );
}

#[test]
fn a_section_is_the_folder_under_the_directory_of_the_charts() {
    let entry = |manifest: &Manifest, path: &str| Entry::new(manifest, path, String::new());
    let default = Manifest::new("sales");
    assert_eq!(
        entry(&default, "charts/01-penguins/13-donut-of-species.sql").section,
        "Penguins"
    );
    assert_eq!(entry(&default, "charts/origins.sql").section, "Charts");
    assert_eq!(
        entry(&default, "charts/regions/02-west-coast/by-month.sql").section,
        "Regions / West coast"
    );
    let views =
        Manifest::parse("[project]\nname = \"sales\"\n[charts]\nscripts = [\"views/**/*.sql\"]\n")
            .unwrap();
    let revenue = entry(&views, "views/finance/revenue.sql");
    assert_eq!(
        (revenue.section.as_str(), revenue.title.as_str()),
        ("Finance", "Revenue")
    );

    // The charts of a folder follow the folder's own charts, and each section's charts are in
    // the order of their paths.
    let charts = [
        "charts/b/one.sql",
        "charts/zeta.sql",
        "charts/a/two.sql",
        "charts/alpha.sql",
    ];
    let paths: Vec<String> = entries(
        &default,
        charts.map(|path| (path.to_string(), String::new())),
    )
    .into_iter()
    .map(|entry| entry.path)
    .collect();
    assert_eq!(
        paths,
        [
            "charts/alpha.sql",
            "charts/zeta.sql",
            "charts/a/two.sql",
            "charts/b/one.sql"
        ]
    );
}

#[tokio::test]
async fn the_sample_project_has_a_chart_for_each_example_by_section() {
    let workspace = sample().await;
    let entries = workspace.gallery();
    assert_eq!(entries.len(), 100);
    let mut sections: Vec<&str> = entries.iter().map(|entry| entry.section.as_str()).collect();
    sections.dedup();
    assert_eq!(sections, SECTIONS);
    for entry in &entries {
        assert!(!entry.description.is_empty(), "{}", entry.path);
        assert!(!entry.description.ends_with(':'), "{}", entry.path);
        // A sample table reads by its schema, as `samples.penguins`.
        assert!(!entry.text.contains("FROM penguins"), "{}", entry.path);
    }
}

#[tokio::test]
async fn the_sample_project_loads_without_diagnostics() {
    let workspace = sample().await;
    let sidebar = workspace.sidebar();
    let paths = (sidebar.data.iter().map(|(path, _)| path))
        .chain(sidebar.charts.iter().flat_map(|(_, paths)| paths));
    let mut failures = Vec::new();
    for path in paths {
        for diagnostic in workspace.language().diagnostics(path) {
            failures.push(format!("{path}: {}", diagnostic.message));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn every_chart_runs_and_draws_its_charts() {
    let workspace = sample().await;
    let (cascade, engine) = (ChartTheme::default().cascade(None), engine());
    let mut failures = Vec::new();
    for entry in workspace.gallery() {
        let name = &entry.path;
        let outcome = match workspace.loader().run_chart(name, &entry.text).await {
            Ok(outcome) => outcome,
            Err(error) => {
                failures.push(format!("{name}: {}", error.message));
                continue;
            }
        };
        let result = &outcome.result;
        let mut charts = 0;
        for row in 0..result.num_rows() {
            for column in result.chart_columns() {
                let Some(chart) = result.chart(row, column) else {
                    continue;
                };
                charts += 1;
                let scene = StaticChart::new(
                    chart.clone(),
                    cascade.clone(),
                    ChartTheme::default().preferences(),
                    engine.clone(),
                )
                .and_then(|mut chart| chart.scene(THUMBNAIL));
                if let Err(error) = scene {
                    failures.push(format!("{name}, row {row}, column {column}: {error}"));
                }
            }
        }
        if charts == 0 {
            failures.push(format!("{name}: no chart"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The lifted form of each chart lifts to itself, with nothing more to write.
#[tokio::test]
async fn every_chart_lifts() {
    let workspace = sample().await;
    let mut failures = Vec::new();
    for entry in workspace.gallery() {
        let name = &entry.path;
        let lifted = match workspace.loader().lift(name, &entry.text).await {
            Ok(lifted) => lifted,
            Err(error) => {
                failures.push(format!("{name}: {}", error.message));
                continue;
            }
        };
        match workspace.loader().lift(name, &lifted.text).await {
            Ok(again) if again.text == lifted.text && again.generated.is_empty() => {}
            Ok(again) => failures.push(format!("{name}: lifts again to\n{}", again.text)),
            Err(error) => failures.push(format!("{name}: its lifting fails: {}", error.message)),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn every_chart_is_in_the_style() {
    for entry in sample().await.gallery() {
        let text = entry.text.trim_end_matches('\n');
        assert_eq!(
            avenger_sql_syntax::format::format(text, avenger_sql_syntax::Role::Script),
            text,
            "{} is not formatted",
            entry.path
        );
    }
}

#[tokio::test]
async fn every_line_of_the_charts_has_the_depth_that_enter_and_tab_give_it() {
    for entry in sample().await.gallery() {
        let mut start = 0;
        for line in entry.text.split_inclusive('\n') {
            let content = line.trim_end_matches('\n');
            if !content.trim().is_empty() {
                let indentation = content.len() - content.trim_start().len();
                let depth = avenger_sql_syntax::indent::depth(&entry.text, start);
                assert!(
                    depth.is_none_or(|depth| depth == indentation),
                    "{depth:?} for {content:?} in {}",
                    entry.path
                );
            }
            start += line.len();
        }
    }
}
