//! The stylesheets of a project in the editor, without a window: the cascade of the project's
//! charts with its entry stylesheet, a saved stylesheet that changes the cascade, the errors of a
//! stylesheet and of its imports on their own files, the warning of a stylesheet that the entry
//! stylesheet does not reach, a stylesheet's tab, and the classes of a stylesheet's tokens.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use avenger_scenegraph::marks::mark::SceneMark;
use avenger_sql_editor::charts::{engine, ChartTheme, StaticChart};
use avenger_sql_editor::highlight::{css, Class};
use avenger_sql_editor::workspace::{self, Saved, Styles, TabKind, Workspace};
use avenger_style::{Cascade, Severity};
use eframe::egui;

const ORIGINS: &str = "charts/origins.sql";
const MAIN: &str = "styles/main.css";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../avenger-sql-project/tests/projects/vega")
}

/// A copy of the fixture in a temporary directory, with the stylesheets `stylesheets`, each a
/// path and its text, written over the fixture's.
fn copy(stylesheets: &[(&str, &str)]) -> tempfile::TempDir {
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
    for (path, text) in stylesheets {
        std::fs::write(directory.path().join(path), text).unwrap();
    }
    directory
}

async fn open(path: &Path) -> Workspace {
    workspace::open(path).await.unwrap().0
}

/// The fills of the bars of the chart of `origins.sql`, in the cascade `cascade`, and the fill
/// of the chart's background, without the plot's background.
async fn fills(workspace: &Workspace, cascade: Arc<Cascade>) -> (Vec<[f32; 4]>, [f32; 4]) {
    fn rects(marks: &[SceneMark], found: &mut Vec<(String, [f32; 4])>) {
        for mark in marks {
            match mark {
                SceneMark::Rect(rect) => {
                    let fill = rect.fill.first().expect("a fill").color_or_transparent();
                    found.push((rect.name.clone(), fill));
                }
                SceneMark::Group(group) => rects(&group.marks, found),
                _ => {}
            }
        }
    }
    let text = std::fs::read_to_string(workspace.root().unwrap().join(ORIGINS)).unwrap();
    let outcome = workspace.loader().run_chart(ORIGINS, &text).await.unwrap();
    let chart = outcome.result.chart(0, 0).unwrap().clone();
    let preferences = ChartTheme::default().preferences();
    let mut chart = StaticChart::new(chart, cascade, preferences, engine()).unwrap();
    let scene = chart.scene([400.0, 300.0]).unwrap();
    let mut found = Vec::new();
    rects(&scene.marks, &mut found);
    let background = (found.iter())
        .find(|(name, _)| name == "background")
        .map(|(_, fill)| *fill)
        .expect("a background");
    let bars = (found.into_iter())
        .filter(|(name, _)| !name.ends_with("background"))
        .map(|(_, fill)| fill)
        .collect();
    (bars, background)
}

/// The cascade of the charts of `workspace`, as the editor builds it for `theme`.
fn cascade(workspace: &Workspace, theme: ChartTheme) -> Arc<Cascade> {
    theme.cascade(workspace.styles().sheet())
}

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

#[tokio::test]
async fn the_charts_of_a_project_take_its_entry_stylesheet() {
    let directory = copy(&[(
        MAIN,
        "@import 'std:base.css';\nmark[type=bar] { fill: #ff0000; }\n",
    )]);
    let workspace = open(directory.path()).await;
    assert!(workspace.styles().diagnostics(MAIN).is_empty());

    let (bars, _) = fills(&workspace, cascade(&workspace, ChartTheme::default())).await;
    assert!(!bars.is_empty());
    assert!(bars.iter().all(|fill| *fill == RED), "{bars:?}");

    // Without the project's stylesheet, the base stylesheet colors the bars.
    let (bars, _) = fills(&workspace, ChartTheme::default().cascade(None)).await;
    assert!(bars.iter().all(|fill| *fill != RED), "{bars:?}");

    // The background stays the panel's color in either theme, with the project's stylesheet
    // after the base stylesheet.
    for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
        let theme = ChartTheme::of(&visuals);
        let (_, background) = fills(&workspace, cascade(&workspace, theme)).await;
        let [r, g, b, _] = visuals.panel_fill.to_normalized_gamma_f32();
        for (actual, expected) in background.iter().zip([r, g, b]) {
            assert!((actual - expected).abs() < 0.01, "{background:?}");
        }
    }
}

#[tokio::test]
async fn a_saved_stylesheet_restyles_the_charts() {
    let directory = copy(&[(MAIN, "mark[type=bar] { fill: #ff0000; }\n")]);
    let mut workspace = open(directory.path()).await;
    workspace.open_tab(ORIGINS);

    // A stylesheet opens in a tab of its own, and the result pane keeps the chart's result.
    assert_eq!(workspace.open_tab(MAIN), Some(true));
    let tab = workspace.active().unwrap();
    assert_eq!((tab.path.as_str(), &tab.kind), (MAIN, &TabKind::Stylesheet));
    assert_eq!(workspace.shown().unwrap().path, ORIGINS);

    let edited = "mark[type=bar] { fill: #0000ff; }\n";
    workspace.edit(MAIN, edited);
    assert!(workspace.tab(MAIN).unwrap().modified());
    assert_eq!(workspace.save(MAIN).unwrap(), Saved::Stylesheet);
    assert_eq!(
        std::fs::read_to_string(directory.path().join(MAIN)).unwrap(),
        edited
    );

    // The stylesheets load again from their files, and the cascade takes the saved fill.
    let styles = Styles::load(workspace.files()).await.unwrap();
    assert_eq!(styles.text(MAIN), Some(edited));
    workspace.set_styles(styles);
    let (bars, _) = fills(&workspace, cascade(&workspace, ChartTheme::default())).await;
    assert!(!bars.is_empty());
    assert!(bars.iter().all(|fill| *fill == BLUE), "{bars:?}");

    // Closing the stylesheet's tab shows the chart's tab again.
    workspace.close_tab(MAIN).unwrap();
    assert_eq!(workspace.active().unwrap().path, ORIGINS);
}

#[tokio::test]
async fn the_errors_of_a_stylesheet_and_its_imports_are_on_their_files() {
    let brand = "mark[type=bar] { fill: #ff0000; }\ntitle[type=bar] { color: red; }\n";
    let main = "@import 'brand.css';\n@import 'missing.css';\n";
    let directory = copy(&[(MAIN, main), ("styles/brand.css", brand)]);
    let workspace = open(directory.path()).await;
    let styles = workspace.styles();

    // The error of an import is in the stylesheet that holds it, at its URL.
    let errors = styles.diagnostics(MAIN);
    assert_eq!(errors.len(), 1, "{errors:?}");
    let span = errors[0].span;
    assert_eq!(&main[span.start..span.end], "'missing.css'");

    // The error of an imported stylesheet is in its own file.
    let errors = styles.diagnostics("styles/brand.css");
    assert_eq!(errors.len(), 1, "{errors:?}");
    let span = errors[0].span;
    assert_eq!(&brand[span.start..span.end], "title[type=bar]");
    assert_eq!(
        errors[0].message,
        "the element `title` has no attribute `type`"
    );

    // The rules of the import that loads still style the charts.
    let (bars, _) = fills(&workspace, cascade(&workspace, ChartTheme::default())).await;
    assert!(bars.iter().all(|fill| *fill == RED), "{bars:?}");
}

#[tokio::test]
async fn a_stylesheet_that_the_entry_stylesheet_does_not_reach_has_a_warning() {
    let extra = "mark { fill: #0000ff; }\naxis { color: red; }\n";
    let directory = copy(&[("styles/extra.css", extra)]);
    let workspace = open(directory.path()).await;
    let styles = workspace.styles();
    assert!(styles.diagnostics(MAIN).is_empty());

    let warnings = styles.diagnostics("styles/extra.css");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let warning = &warnings[0];
    assert_eq!(warning.severity, Severity::Warning);
    assert_eq!(
        warning.message,
        "this stylesheet is not imported by the entry stylesheet `styles/main.css`, so it styles \
         no chart"
    );
    assert_eq!(
        warning.help.as_deref(),
        Some("Add `@import 'extra.css';` to `styles/main.css`.")
    );
    assert_eq!(
        &extra[warning.span.start..warning.span.end],
        "mark { fill: #0000ff; }"
    );

    // The unreached stylesheet styles no chart.
    let (bars, _) = fills(&workspace, cascade(&workspace, ChartTheme::default())).await;
    assert!(bars.iter().all(|fill| *fill != BLUE), "{bars:?}");
}

/// The text and the class of each token of the stylesheet `text`.
fn classes(text: &str) -> Vec<(String, Class)> {
    let chars: Vec<char> = text.chars().collect();
    (css(text).into_iter())
        .map(|span| (chars[span.range].iter().collect(), span.class))
        .collect()
}

#[test]
fn a_stylesheet_colors_by_its_tokens() {
    let text = "/* bars */\n@import 'brand.css' layer(brand);\n\
                plot > mark[type=bar].wide:first-child, #main {\n  \
                fill: var(--accent, #4c78a8) !important;\n  --gap: calc(2px * 3);\n  \
                font-weight: bold;\n  label { angle: 45deg; }\n}\n\
                @media (prefers-color-scheme: dark) { axis { color: env(foreground); } }";
    use Class::*;
    let expected = [
        ("/* bars */", Comment),
        ("@import", Keyword),
        ("'brand.css'", Path),
        ("layer(", Function),
        ("brand", Constant),
        ("plot", Type),
        (">", Operator),
        ("mark", Type),
        ("type", Column),
        ("=", Operator),
        ("bar", String),
        (".", Table),
        ("wide", Table),
        (":", Function),
        ("first-child", Function),
        ("#main", Table),
        ("fill", Argument),
        ("var(", Function),
        ("--accent", Variable),
        ("#4c78a8", Number),
        ("!", Keyword),
        ("important", Keyword),
        ("--gap", Variable),
        ("calc(", Function),
        ("2px", Number),
        ("*", Operator),
        ("3", Number),
        ("font-weight", Argument),
        ("bold", Constant),
        ("label", Type),
        ("angle", Argument),
        ("45deg", Number),
        ("@media", Keyword),
        ("prefers-color-scheme", Argument),
        ("dark", Constant),
        ("axis", Type),
        ("color", Argument),
        ("env(", Function),
        ("foreground", Constant),
    ];
    let expected: Vec<(std::string::String, Class)> = (expected.into_iter())
        .map(|(text, class)| (text.to_string(), class))
        .collect();
    assert_eq!(classes(text), expected);
}

/// A comment or a string that is still being typed keeps its class to the end of the text.
#[test]
fn an_unterminated_comment_or_string_keeps_its_class() {
    let found = classes("mark { fill: red; }\n/* more");
    assert_eq!(
        found.last().unwrap(),
        &("/* more".to_string(), Class::Comment)
    );
    let found = classes("@import 'brand");
    assert_eq!(found.last().unwrap(), &("'brand".to_string(), Class::Path));
}
