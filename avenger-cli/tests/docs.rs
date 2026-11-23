//! `docs`, `complete`, and `signature` on a copy of avenger-sql-project's fixture project: the
//! pages of functions, constructors, parameter paths, style entries, and syntax forms, search and
//! its filters, the hover, the completions, and the signature help at a position of a script and
//! of a cell, and the suggestions of `check`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A copy of the fixture project in a temporary directory, by its canonical root, with a
/// document of one cell.
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
    std::fs::write(
        root.join("reports/overview.qmd"),
        "# Report\n\n```{avenger-sql}\nFROM wet_days |> AGGREGATE mark_bar(x => weather, y => COUNT(*)) AS chart\n```\n",
    )
    .unwrap();
    (directory, root)
}

/// The exit status, standard output, and standard error of `avenger` with `args` in `directory`.
fn avenger(directory: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_avenger"))
        .args(args)
        .current_dir(directory)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

/// `avenger --json` with `args` in `directory`, with its standard output as JSON.
fn json(directory: &Path, args: &[&str]) -> (i32, serde_json::Value) {
    let args: Vec<&str> = std::iter::once("--json")
        .chain(args.iter().copied())
        .collect();
    let (status, stdout, stderr) = avenger(directory, &args);
    let value = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout} {stderr}"));
    (status, value)
}

#[test]
fn a_name_prints_the_page_of_its_function_or_its_constructors() {
    let here = std::env::temp_dir();
    let (status, stdout, _) = avenger(&here, &["docs", "date_trunc"]);
    assert_eq!(status, 0);
    assert!(
        stdout.starts_with("## `DATE_TRUNC`\n\n```googlesql\nDATE_TRUNC("),
        "{stdout}"
    );
    assert!(stdout.contains("**Parameters**"), "{stdout}");
    // A constructor under the name that a call writes, with its places and its unique name.
    let (_, stdout, _) = avenger(&here, &["docs", "pin"]);
    assert!(
        stdout.starts_with("## `pin`\n\n```googlesql\npin("),
        "{stdout}"
    );
    assert!(
        stdout.contains("Builds `pin => pin(...)` in `scale(...)`."),
        "{stdout}"
    );
    assert!(
        stdout.contains("Its unique name is `domain_pin`."),
        "{stdout}"
    );
    // A name of a function and of a constructor prints both, and a written name of several
    // constructors prints each.
    let (_, stdout, _) = avenger(&here, &["docs", "log"]);
    assert!(
        stdout.starts_with("## `LOG`") && stdout.contains("\n## `log`\n"),
        "{stdout}"
    );
    let (_, value) = json(&here, &["docs", "title"]);
    let kinds: Vec<&str> = (value["entries"].as_array().unwrap().iter())
        .map(|entry| entry["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.iter().filter(|kind| **kind == "constructor").count() >= 2,
        "{kinds:?}"
    );
    // `--brief` and `--examples`.
    let (_, stdout, _) = avenger(&here, &["docs", "DATE_TRUNC", "--brief"]);
    assert!(
        stdout.starts_with("```googlesql\nDATE_TRUNC(") && !stdout.contains("**"),
        "{stdout}"
    );
    let (_, stdout, _) = avenger(&here, &["docs", "DATE_TRUNC", "--examples"]);
    assert!(
        stdout.starts_with("```googlesql\n") && !stdout.contains("## "),
        "{stdout}"
    );
}

#[test]
fn a_dotted_path_walks_the_parameters_and_their_constructors() {
    let here = std::env::temp_dir();
    let (status, stdout, _) = avenger(&here, &["docs", "mark_bar.x.scale"]);
    assert_eq!(status, 0);
    assert!(
        stdout.starts_with("## `mark_bar.x.scale`\n\nA parameter of `x(...)`.\n"),
        "{stdout}"
    );
    assert!(stdout.contains("scale => scale(...)"), "{stdout}");
    let (_, stdout, _) = avenger(&here, &["docs", "mark_bar.x.scale.type"]);
    assert!(stdout.contains("A parameter of `scale(...)`."), "{stdout}");
    // A parameter of a parameter's own constructor comes before a constructor of the parameter.
    let (_, stdout, _) = avenger(&here, &["docs", "mark_bar.x.scale.type.log"]);
    assert!(stdout.contains("A parameter of `type(...)`."), "{stdout}");
    let (_, stdout, _) = avenger(&here, &["docs", "mark_bar.x.scale.pin"]);
    assert!(stdout.contains("A parameter of `scale(...)`."), "{stdout}");
    let (status, _, stderr) = avenger(&here, &["docs", "mark_bar.nope"]);
    assert_eq!(status, 1);
    assert!(
        stderr.contains("`mark_bar` has no parameter `nope`"),
        "{stderr}"
    );
}

#[test]
fn style_entries_and_syntax_forms_have_pages() {
    let here = std::env::temp_dir();
    let (_, stdout, _) = avenger(&here, &["docs", "style:stroke-width"]);
    assert!(stdout.contains("stroke-width: <length [0,∞]>"), "{stdout}");
    assert!(stdout.contains("Initial: `1`. Inherited: no."), "{stdout}");
    let (_, stdout, _) = avenger(&here, &["docs", "style:axis"]);
    assert!(
        stdout.starts_with("## `axis`") && stdout.contains("Parts: "),
        "{stdout}"
    );
    let (_, stdout, _) = avenger(&here, &["docs", "style:padding"]);
    assert!(stdout.contains("A shorthand of `padding-top`"), "{stdout}");
    let (status, stdout, _) = avenger(&here, &["docs", "LOAD DATA"]);
    assert_eq!(status, 0);
    assert!(stdout.starts_with("## Data scripts\n"), "{stdout}");
    // The whole reference holds every function.
    let (_, stdout, _) = avenger(&here, &["docs", "--all"]);
    for name in [
        "## `COUNT`",
        "## `mark_bar`",
        "# Constructors",
        "## `stroke-width`",
        "## `padding`\n\nA shorthand of ",
        "## Data scripts",
    ] {
        assert!(stdout.contains(name), "{name}");
    }
}

#[test]
fn search_ranks_the_entries_and_names_an_alias() {
    let here = std::env::temp_dir();
    let (status, stdout, _) = avenger(&here, &["docs", "search", "truncate", "date"]);
    assert_eq!(status, 0);
    assert!(stdout.starts_with("date_trunc "), "{stdout}");
    assert!(!stdout.lines().next().unwrap().contains('('), "{stdout}");
    let (_, stdout, _) = avenger(&here, &["docs", "search", "strftime"]);
    assert!(
        stdout.starts_with("format_timestamp ")
            && stdout.lines().next().unwrap().ends_with("  (for strftime)"),
        "{stdout}"
    );
    let (_, value) = json(&here, &["docs", "search", "strftime"]);
    let first = &value["results"][0];
    assert_eq!(
        (first["name"].as_str(), first["alias"].as_str()),
        (Some("format_timestamp"), Some("strftime"))
    );
    // A name of no entry searches.
    let (status, stdout, stderr) = avenger(&here, &["docs", "date_trunk"]);
    assert_eq!(status, 0);
    assert!(stdout.starts_with("date_trunc "), "{stdout}");
    assert!(stderr.starts_with("no entry `date_trunk`"), "{stderr}");
    // Filters, with words and alone.
    let (_, value) = json(&here, &["docs", "search", "date", "--kind", "scalar"]);
    let results = value["results"].as_array().unwrap();
    assert!(results.iter().all(|r| r["kind"] == "scalar"), "{value}");
    assert!(results.iter().any(|r| r["name"] == "date_trunc"), "{value}");
    let (_, value) = json(&here, &["docs", "--family", "mark"]);
    let results = value["results"].as_array().unwrap();
    assert!(
        !results.is_empty() && results.iter().all(|r| r["kind"] == "mark"),
        "{value}"
    );
    let (_, value) = json(&here, &["docs", "search", "date", "--returns", "INT64"]);
    let names: Vec<&str> = (value["results"].as_array().unwrap().iter())
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"date_diff") && !names.contains(&"date_trunc"),
        "{names:?}"
    );
    // Filters alone, and after `search` without words, list the constructors too.
    let (_, value) = json(&here, &["docs", "--family", "constructor"]);
    let results = value["results"].as_array().unwrap();
    assert!(results.iter().any(|r| r["name"] == "pin"), "{value}");
    let (_, value) = json(&here, &["docs", "search", "--family", "mark"]);
    assert!(!value["results"].as_array().unwrap().is_empty(), "{value}");
    // Nothing matches.
    let (status, _, _) = avenger(&here, &["docs", "search", "zzzzzz"]);
    assert_eq!(status, 1);
}

#[test]
fn positions_give_the_hover_the_completions_and_the_signature_help() {
    let (_directory, root) = project();
    // `|> AGGREGATE mark_bar(x => region, y => cars) AS chart` is line 4.
    let (status, stdout, _) = avenger(&root, &["docs", "--at", "charts/origins.av.sql:4:20"]);
    assert_eq!(status, 0);
    assert!(stdout.starts_with("```googlesql\nmark_bar("), "{stdout}");
    // The columns of the project's tables complete.
    let (status, value) = json(&root, &["complete", "charts/origins.av.sql:4:41"]);
    assert_eq!(status, 0);
    let labels: Vec<&str> = (value["items"].as_array().unwrap().iter())
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(
        labels.contains(&"cars") && labels.contains(&"region"),
        "{labels:?}"
    );
    assert_eq!(value["range"]["start"]["line"], 4);
    let (_, stdout, _) = avenger(
        &root,
        &["complete", "charts/origins.av.sql:4:41", "--limit", "2"],
    );
    assert_eq!(stdout.lines().count(), 2, "{stdout}");
    let (status, stdout, _) = avenger(&root, &["signature", "charts/origins.av.sql:4:24"]);
    assert_eq!(status, 0);
    assert!(stdout.starts_with("> mark_bar("), "{stdout}");
    assert!(stdout.contains("\n`x: ANY`\n"), "{stdout}");
    // A column one past the end of its line, where an editor asks.
    std::fs::write(
        root.join("charts/typing.av.sql"),
        "FROM vega_datasets.cars\n|> SELECT Hor",
    )
    .unwrap();
    let (status, stdout, _) = avenger(&root, &["complete", "charts/typing.av.sql:2:14"]);
    assert_eq!(status, 0);
    assert!(stdout.starts_with("Horsepower "), "{stdout}");
    // A cell of a document: `FROM wet_days |> AGGREGATE mark_bar(...)` is line 4.
    let (status, stdout, _) = avenger(&root, &["docs", "--at", "reports/overview.qmd:4:33"]);
    assert_eq!(status, 0);
    assert!(stdout.starts_with("```googlesql\nmark_bar("), "{stdout}");
    let (_, value) = json(&root, &["complete", "reports/overview.qmd:4:43"]);
    let labels: Vec<&str> = (value["items"].as_array().unwrap().iter())
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(labels.contains(&"weather"), "{labels:?}");
    assert_eq!(value["range"]["start"]["line"], 4);
    // A value of a set, in an open string and in a closed one.
    for text in ["'ho", "'ho')"] {
        let chart = format!("FROM vega_datasets.cars\n|> AGGREGATE mark_bar(orient => {text}");
        std::fs::write(root.join("charts/typing.av.sql"), chart).unwrap();
        let (status, stdout, _) = avenger(&root, &["complete", "charts/typing.av.sql:2:35"]);
        assert_eq!(
            (status, stdout.split_whitespace().next()),
            (0, Some("'horizontal'")),
            "{text}"
        );
    }
    // A data script, a module, a loose script, and a loose document.
    let (status, stdout, _) = avenger(&root, &["docs", "--at", "data/summaries.av.sql:4:14"]);
    assert_eq!(status, 0, "{stdout}");
    let (status, stdout, _) = avenger(&root, &["signature", "modules/weather.av.sqlm:3:44"]);
    assert_eq!(status, 1, "{stdout}");
    let (status, stdout, _) = avenger(&root, &["complete", "modules/weather.av.sqlm:3:45"]);
    assert!(status == 0 && stdout.contains("TIMESTAMP "), "{stdout}");
    let loose = tempfile::tempdir().unwrap();
    std::fs::write(
        loose.path().join("alone.av.sql"),
        "FROM UNNEST([1, 2]) AS v\n|> SELECT ABS(v)\n",
    )
    .unwrap();
    let (status, stdout, _) = avenger(loose.path(), &["signature", "alone.av.sql:2:15"]);
    assert!(status == 0 && stdout.starts_with("> ABS("), "{stdout}");
    std::fs::write(
        loose.path().join("alone.qmd"),
        "```{avenger-sql}\nFROM UNNEST([1, 2]) AS v\n|> SELECT ABS(v)\n```\n",
    )
    .unwrap();
    let (status, stdout, _) = avenger(loose.path(), &["signature", "alone.qmd:3:15"]);
    assert!(status == 0 && stdout.starts_with("> ABS("), "{stdout}");
    // Outside every cell, and a position that is no position.
    let (status, _, stderr) = avenger(&root, &["complete", "reports/overview.qmd:1:1"]);
    assert_eq!(status, 2);
    assert!(
        stderr.contains("not in an `{avenger-sql}` cell"),
        "{stderr}"
    );
    let (status, _, _) = avenger(&root, &["signature", "charts/origins.av.sql:0:1"]);
    assert_eq!(status, 2);
    // Nothing there.
    let (status, _, _) = avenger(&root, &["signature", "charts/origins.av.sql:1:1"]);
    assert_eq!(status, 1);
}

#[test]
fn check_suggests_the_closest_function_and_column() {
    let (_directory, root) = project();
    std::fs::write(
        root.join("charts/typo.av.sql"),
        "FROM vega_datasets.cars\n|> SELECT date_trunk(CURRENT_DATE(), MONTH) AS d, Horsepowr\n",
    )
    .unwrap();
    let (status, stdout, _) = avenger(&root, &["check", "charts/typo.av.sql"]);
    assert_eq!(status, 1);
    assert!(
        stdout.contains("Function not found: date_trunk. Did you mean date_trunc?"),
        "{stdout}"
    );
    std::fs::write(
        root.join("charts/typo.av.sql"),
        "FROM vega_datasets.cars\n|> SELECT Horsepowr\n",
    )
    .unwrap();
    let (_, stdout, _) = avenger(&root, &["check", "charts/typo.av.sql"]);
    assert!(
        stdout.contains("Unrecognized name: Horsepowr. Did you mean Horsepower?"),
        "{stdout}"
    );
}
