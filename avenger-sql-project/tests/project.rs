//! Projects over embedded files and a directory on disk: the manifest, the file source and the
//! session that reads it, the roles of files, and the entry stylesheet.

use avenger_sql::data::{file_table, CsvOptions, Files, Format};
use avenger_sql_project::{
    FileSource, Manifest, ManifestError, ModuleFile, ProjectError, ProjectFiles, Role, SourceError,
    FORMAT,
};
use avenger_style::SourceId;
use datafusion::prelude::SessionContext;

const MANIFEST: &str = "[project]\nname = \"sales\"\n";

/// The project of the files `files`, each a path relative to the root and its text, with the
/// manifest `MANIFEST` unless `files` has one.
async fn project(files: &[(&str, &str)]) -> Result<ProjectFiles, ProjectError> {
    ProjectFiles::open(source(files).await).await
}

async fn source(files: &[(&str, &str)]) -> FileSource {
    let mut files: Vec<(String, String)> = (files.iter())
        .map(|(path, text)| (path.to_string(), text.to_string()))
        .collect();
    if !files.iter().any(|(path, _)| path == "avenger.toml") {
        files.push(("avenger.toml".into(), MANIFEST.into()));
    }
    FileSource::embedded("sales", files).await.unwrap()
}

fn manifest_error(text: &str) -> ManifestError {
    Manifest::parse(text).unwrap_err()
}

fn strings(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|path| path.to_string()).collect()
}

/// The table of the CSV file at `uri`, relative to the root, as a data script reads it in a
/// session where `source` is registered, and its rows.
async fn read_csv(source: &FileSource, uri: &str) -> usize {
    let ctx = SessionContext::new();
    source.register(&ctx);
    let files = Files {
        format: Format::Csv,
        uris: vec![uri.into()],
        columns: None,
        csv: Some(CsvOptions {
            header: true,
            delimiter: b',',
            quote: b'"',
            quoted_newlines: false,
        }),
        compression: None,
        hive_partition_uri_prefix: None,
        partitions: None,
    };
    let table = file_table(&ctx.state(), &files, source.base().as_str())
        .await
        .unwrap();
    let batches = ctx.read_table(table).unwrap().collect().await.unwrap();
    batches.iter().map(|batch| batch.num_rows()).sum()
}

// The manifest.

#[test]
fn manifest_defaults() {
    let manifest = Manifest::parse(MANIFEST).unwrap();
    assert_eq!(manifest, Manifest::new("sales"));
    assert_eq!(manifest.project.format, FORMAT);
    let patterns = |patterns: &[avenger_sql_project::Pattern]| -> Vec<String> {
        patterns.iter().map(ToString::to_string).collect()
    };
    assert_eq!(patterns(&manifest.data.scripts), ["data/**/*.av.sql"]);
    assert_eq!(patterns(&manifest.data.exclude), ["data/files/**"]);
    assert_eq!(patterns(&manifest.charts.scripts), ["charts/**/*.av.sql"]);
    assert_eq!(manifest.style.stylesheet, "styles/main.css");
    assert_eq!(manifest.modules.root, "modules");
    assert_eq!(manifest.data.scripts[0].range(), None);
}

#[test]
fn smallest_manifest() {
    assert_eq!(Manifest::smallest("sales"), MANIFEST);
    for name in [
        "my sales",
        "say \"hi\"",
        "back\\slash",
        "tab\tand line\n",
        "ünïcode",
    ] {
        let text = Manifest::smallest(name);
        assert_eq!(Manifest::parse(&text), Ok(Manifest::new(name)), "{text}");
    }
}

#[test]
fn manifest_every_field() {
    let text = r#"
[project]
name = "finance"
format = 1

[data]
scripts = ["./sql//**/*.av.sql", "extra.av.sql"]
exclude = []

[charts]
scripts = ["views/*.av.sql"]

[style]
stylesheet = "./css/app.css"

[modules]
root = "lib/"
"#;
    let manifest = Manifest::parse(text).unwrap();
    assert_eq!(manifest.project.name, "finance");
    // A pattern reads without its `.` names and its empty names, as a path does.
    let texts: Vec<&str> = manifest.data.scripts.iter().map(|p| p.as_str()).collect();
    assert_eq!(texts, ["sql/**/*.av.sql", "extra.av.sql"]);
    assert!(manifest.data.exclude.is_empty());
    assert_eq!(manifest.charts.scripts[0].as_str(), "views/*.av.sql");
    assert_eq!(manifest.style.stylesheet, "css/app.css");
    assert_eq!(manifest.modules.root, "lib");
    // A pattern keeps the range of its string and its line, and so does the entry stylesheet.
    let range = manifest.data.scripts[1].range().unwrap();
    assert_eq!(&text[range], "\"extra.av.sql\"");
    assert_eq!(manifest.data.scripts[1].line(), Some(7));
    let range = manifest.style.range.clone().unwrap();
    assert_eq!(&text[range], "\"./css/app.css\"");
    assert_eq!(manifest.style.line, Some(14));
    assert_eq!(Manifest::new("a").style.range, None);
    // The directory of a pattern is its names before the first glob or the last name.
    let directories: Vec<&str> = (manifest.data.scripts.iter())
        .chain(&manifest.charts.scripts)
        .map(|pattern| pattern.directory())
        .collect();
    assert_eq!(directories, ["sql", "", "views"]);
    let manifest = Manifest::parse(
        "[project]\nname = \"a\"\n[charts]\nscripts = [\"*.av.sql\", \"a/b/c[0-9]/*.av.sql\", \"a/b.av.sql\"]\n",
    )
    .unwrap();
    let directories: Vec<&str> = (manifest.charts.scripts.iter())
        .map(|pattern| pattern.directory())
        .collect();
    assert_eq!(directories, ["", "a/b", "a"]);
}

#[test]
fn manifest_errors() {
    // An unknown key, at its line.
    let error = manifest_error("[project]\nname = \"sales\"\n\n[data]\nscripts = []\nextra = 1\n");
    assert_eq!(error.line, Some(6), "{error}");
    assert!(error.message.contains("unknown field `extra`"), "{error}");
    // An unknown section.
    let error =
        manifest_error("[project]\nname = \"sales\"\n\n[charts]\n\n[deploy]\nhost = \"x\"\n");
    assert_eq!(error.line, Some(6), "{error}");
    assert!(error.message.contains("unknown field `deploy`"), "{error}");
    // An unknown key of `[project]`.
    let error = manifest_error("[project]\nname = \"sales\"\nversion = 2\n");
    assert_eq!(error.line, Some(3), "{error}");
    // A format that this crate doesn't read.
    let error = manifest_error("[project]\nname = \"sales\"\nformat = 2\n");
    assert_eq!(error.line, Some(3), "{error}");
    assert!(error.message.contains("project format 2"), "{error}");
    assert_eq!(
        error.to_string(),
        "avenger.toml:3: the project format 2 is not one that this avenger reads, which is 1"
    );
    // A value of the wrong type.
    let error = manifest_error("[project]\nname = \"sales\"\nformat = \"1\"\n");
    assert_eq!(error.line, Some(3), "{error}");
    // `[project]` and its `name` are required.
    let error = manifest_error("[data]\nscripts = []\n");
    assert!(error.message.contains("missing field `project`"), "{error}");
    let error = manifest_error("[project]\nformat = 1\n");
    assert!(error.message.contains("missing field `name`"), "{error}");
    // An empty name, at its value.
    let error = manifest_error("[project]\nname = \"\"\n");
    assert_eq!(error.line, Some(2), "{error}");
    assert_eq!(
        error.to_string(),
        "avenger.toml:2: the name of the project is empty"
    );
    // An invalid pattern.
    let error = manifest_error(
        "[project]\nname = \"sales\"\n[charts]\nscripts = [\n  \"charts/[*.av.sql\",\n]\n",
    );
    assert_eq!(error.line, Some(5), "{error}");
    // A pattern that is not relative to the root, or that names no file, at its value.
    for pattern in [
        "/abs/*.av.sql",
        "../x/*.av.sql",
        "charts/../../x/*.av.sql",
        "charts\\\\*.av.sql",
        "",
        "./",
    ] {
        let text = format!(
            "[project]\nname = \"sales\"\n[data]\nscripts = [\n  \"data/*.av.sql\",\n  \"{pattern}\",\n]\n"
        );
        let error = manifest_error(&text);
        assert_eq!(error.line, Some(6), "{pattern}: {error}");
        assert_eq!(
            &text[error.range.clone().unwrap()],
            format!("\"{pattern}\""),
            "{error}"
        );
    }
    // A path that leaves the project.
    let error =
        manifest_error("[project]\nname = \"sales\"\n[style]\nstylesheet = \"../main.css\"\n");
    assert_eq!(error.line, Some(4), "{error}");
    let error = manifest_error("[project]\nname = \"sales\"\n[modules]\nroot = \"/usr/lib\"\n");
    assert_eq!(error.line, Some(4), "{error}");
    // Text that is not TOML.
    let error = manifest_error("[project\nname = \"sales\"\n");
    assert_eq!(error.line, Some(1), "{error}");
}

// The file source.

#[tokio::test]
async fn embedded_source() {
    let source = source(&[
        ("data/files/sales.csv", "region,revenue\neast,10\nwest,20\n"),
        ("data/files/[raw]/x?.csv", "a\n1\n"),
        ("charts/a.av.sql", "FROM sales"),
    ])
    .await;
    assert_eq!(source.base().as_str(), "memory://sales/");
    assert_eq!(
        source.read_text("charts/a.av.sql").await.unwrap(),
        "FROM sales"
    );
    assert_eq!(
        source.list("data").await.unwrap().paths,
        ["data/files/[raw]/x?.csv", "data/files/sales.csv"]
    );
    let listing = source.list("").await.unwrap();
    assert_eq!(
        listing.paths,
        [
            "avenger.toml",
            "charts/a.av.sql",
            "data/files/[raw]/x?.csv",
            "data/files/sales.csv"
        ]
    );
    assert!(listing.skipped.is_empty());
    assert!(source.list("styles").await.unwrap().paths.is_empty());
    assert!(matches!(
        source.list("../x").await,
        Err(SourceError::Path(_))
    ));
    assert!(matches!(
        source.read("charts/b.av.sql").await,
        Err(SourceError::NotFound(path)) if path == "charts/b.av.sql"
    ));
    // A path reads without its `.` names and its empty names, by the manifest's rules.
    for path in ["./charts/a.av.sql", "charts//a.av.sql", "charts/./a.av.sql"] {
        assert_eq!(
            source.read_text(path).await.unwrap(),
            "FROM sales",
            "{path}"
        );
    }
    for path in [
        "",
        ".",
        "/charts/a.av.sql",
        "charts/../a.av.sql",
        "charts\\a.av.sql",
    ] {
        assert!(
            matches!(source.read(path).await, Err(SourceError::Path(_))),
            "{path}"
        );
    }
    assert_eq!(
        source.url("data/files/sales.csv").unwrap().as_str(),
        "memory://sales/data/files/sales.csv"
    );
    // A session where the source is registered reads the files at their URIs.
    assert_eq!(read_csv(&source, "data/files/sales.csv").await, 2);
    assert_eq!(read_csv(&source, "data/files/*.csv").await, 2);
    // A name of the base URL is a host name.
    assert!(matches!(
        FileSource::embedded("a/b", [("x", "y")]).await,
        Err(SourceError::Base(_))
    ));
}

// Roles.

#[tokio::test]
async fn roles() {
    let project = project(&[
        ("data/sources.av.sql", ""),
        ("data/finance/derived.av.sql", ""),
        ("data/files/dump.av.sql", ""),
        ("data/files/sales.csv", ""),
        ("data/notes.txt", ""),
        ("charts/revenue.av.sql", ""),
        ("charts/regions/by-month.av.sql", ""),
        ("scratch.av.sql", ""),
        ("styles/main.css", ""),
        ("styles/brand.css", ""),
        ("modules/stats.av.sqlm", ""),
        ("modules/geo/projections.av.sqlm", ""),
        ("charts/local.av.sqlm", ""),
        ("charts/.hidden.av.sql", ""),
        (".git/HEAD", ""),
        (".git/refs/main.css", ""),
        ("data/.cache/stale.av.sql", ""),
    ])
    .await
    .unwrap();
    assert_eq!(
        project.data_scripts(),
        ["data/finance/derived.av.sql", "data/sources.av.sql"]
    );
    assert_eq!(
        project.chart_scripts(),
        ["charts/regions/by-month.av.sql", "charts/revenue.av.sql"]
    );
    assert_eq!(
        project.modules(),
        [
            ModuleFile {
                path: "modules/geo/projections.av.sqlm".into(),
                names: strings(&["geo", "projections"]),
            },
            ModuleFile {
                path: "modules/stats.av.sqlm".into(),
                names: strings(&["stats"]),
            },
        ]
    );
    assert_eq!(
        project.stylesheets(),
        ["styles/brand.css", "styles/main.css"]
    );
    assert_eq!(project.files().len(), 14);
    // Hidden files and directories are not listed, so they have no role.
    assert!(!(project.files().iter()).any(|path| path.contains("/.") || path.starts_with('.')));
    let manifest = project.manifest();
    assert_eq!(manifest.role("data/files/dump.av.sql"), Ok(None));
    assert_eq!(manifest.role("data/files/events/old.av.sql"), Ok(None));
    assert_eq!(manifest.role("data/files/sales.csv"), Ok(None));
    assert_eq!(manifest.role("scratch.av.sql"), Ok(None));
    assert_eq!(manifest.role("charts/local.av.sqlm"), Ok(None));
    assert_eq!(manifest.role("data/new.av.sql"), Ok(Some(Role::DataScript)));
    // A plain `.sql` or `.sqlm` file has no role, wherever it is.
    assert_eq!(manifest.role("data/new.sql"), Ok(None));
    assert_eq!(manifest.role("charts/new.sql"), Ok(None));
    assert_eq!(manifest.role("modules/stats.sqlm"), Ok(None));
    // A `.css` file among the data is data, not a stylesheet.
    assert_eq!(manifest.role("data/files/report.css"), Ok(None));
    assert_eq!(manifest.role("data/report.css"), Ok(Some(Role::Stylesheet)));
    // A hidden file, or a file under a hidden directory, has no role.
    for path in [
        "charts/.hidden.av.sql",
        ".git/refs/main.css",
        "data/.cache/stale.av.sql",
        "modules/.cache/stats.av.sqlm",
        ".styles/main.css",
    ] {
        assert_eq!(manifest.role(path), Ok(None), "{path}");
    }
    // The patterns match with case.
    assert_eq!(manifest.role("Charts/revenue.av.sql"), Ok(None));
    assert_eq!(manifest.role("DATA/sources.av.sql"), Ok(None));
}

#[tokio::test]
async fn role_patterns() {
    let manifest = r#"
[project]
name = "sales"
[data]
scripts = ["sql/*.av.sql"]
exclude = ["sql/old*.av.sql"]
[charts]
scripts = ["**/*.av.sql"]
[modules]
root = ""
"#;
    let manifest = Manifest::parse(manifest).unwrap();
    // `*` matches within a name, and `**` any number of directories.
    assert_eq!(
        manifest.role("sql/a.av.sql"),
        Err(conflict(&manifest, "sql/a.av.sql"))
    );
    assert_eq!(
        manifest.role("sql/a.av.sql").unwrap_err().to_string(),
        "'sql/a.av.sql' is a data script by the pattern 'sql/*.av.sql' at avenger.toml:5 and a chart \
         script by the pattern '**/*.av.sql' at avenger.toml:8"
    );
    assert_eq!(
        manifest.role("sql/old1.av.sql"),
        Ok(Some(Role::ChartScript))
    );
    assert_eq!(
        manifest.role("sql/nested/a.av.sql"),
        Ok(Some(Role::ChartScript))
    );
    assert_eq!(manifest.role("top.av.sql"), Ok(Some(Role::ChartScript)));
    // The modules of the root.
    assert_eq!(
        manifest.role("a/b.av.sqlm"),
        Ok(Some(Role::Module(strings(&["a", "b"]))))
    );
    assert_eq!(
        manifest.role("stats.av.sqlm"),
        Ok(Some(Role::Module(strings(&["stats"]))))
    );

    fn conflict(manifest: &Manifest, path: &str) -> avenger_sql_project::RoleConflict {
        avenger_sql_project::RoleConflict {
            path: path.into(),
            data: manifest.data.scripts[0].clone(),
            chart: manifest.charts.scripts[0].clone(),
        }
    }
}

#[tokio::test]
async fn role_conflict() {
    let manifest = "[project]\nname = \"sales\"\n[charts]\nscripts = [\"**/*.av.sql\"]\n";
    let error = project(&[
        ("avenger.toml", manifest),
        ("data/sources.av.sql", ""),
        ("data/files/dump.av.sql", ""),
        ("charts/revenue.av.sql", ""),
    ])
    .await
    .unwrap_err();
    let ProjectError::Roles(conflicts) = &error else {
        panic!("{error}");
    };
    assert_eq!(conflicts.len(), 1);
    // The message says where each pattern is: its line, or that it is a default.
    assert_eq!(
        error.to_string(),
        "'data/sources.av.sql' is a data script by the default pattern 'data/**/*.av.sql' and a chart \
         script by the pattern '**/*.av.sql' at avenger.toml:4"
    );
    // The chart pattern is at its place in the manifest, and the data pattern is the default.
    let range = conflicts[0].chart.range().unwrap();
    assert_eq!(&manifest[range], "\"**/*.av.sql\"");
    assert_eq!(conflicts[0].data.range(), None);
}

#[tokio::test]
async fn modules_of_one_module_path() {
    let error = project(&[
        ("modules/geo/States.av.sqlm", ""),
        ("modules/Geo/states.av.sqlm", ""),
        ("modules/geo/states.av.sqlm", ""),
    ])
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the modules 'modules/geo/States.av.sqlm' and 'modules/Geo/states.av.sqlm' have the same module \
         path, which ignores case\nthe modules 'modules/geo/states.av.sqlm' and \
         'modules/Geo/states.av.sqlm' have the same module path, which ignores case"
    );
    // A module path finds its module without regard to case.
    let files = project(&[("modules/geo/States.av.sqlm", "")])
        .await
        .unwrap();
    let names = ["GEO".to_string(), "states".to_string()];
    assert_eq!(
        files.module(&names).unwrap().path,
        "modules/geo/States.av.sqlm"
    );
}

#[tokio::test]
async fn no_manifest() {
    let source = FileSource::embedded("sales", [("charts/a.av.sql", "")])
        .await
        .unwrap();
    let error = ProjectFiles::open(source).await.unwrap_err();
    assert!(matches!(error, ProjectError::NoManifest), "{error}");
    let error = project(&[("avenger.toml", "[project]\nname = 1\n")])
        .await
        .unwrap_err();
    assert!(matches!(error, ProjectError::Manifest(_)), "{error}");
}

// The entry stylesheet.

#[tokio::test]
async fn stylesheet_imports() {
    let project = project(&[
        (
            "styles/main.css",
            "@import 'std:base.css';\n@import 'brand.css' layer(brand);\n@import './charts/bars.css';\nchart { background: white; }\n",
        ),
        ("styles/brand.css", "@import '../shared/colors.css';\nmark { fill: teal; }\n"),
        ("shared/colors.css", ":root { --accent: teal; }\n"),
        ("styles/charts/bars.css", "mark[type=bar] { corner-radius: 2; }\n"),
        ("styles/unused.css", "mark { fill: red; }\n"),
        ("data/files/report.css", ""),
    ])
    .await
    .unwrap();
    let style = project.load_stylesheet().await.unwrap();
    assert!(style.diagnostics.is_empty(), "{:#?}", style.diagnostics);
    let sheet = style.sheet.as_ref().unwrap();
    assert_eq!(sheet.imports().len(), 3);
    assert_eq!(sheet.imports()[1].imports().len(), 1);
    assert_eq!(sheet.imports()[1].imports()[0].stylesheet().rules.len(), 1);
    // A `.css` file among the data is not a stylesheet.
    assert!(project
        .files()
        .contains(&"data/files/report.css".to_string()));
    assert_eq!(style.unreached, ["styles/unused.css"]);
    assert_eq!(
        style.sources,
        [
            "shared/colors.css",
            "styles/brand.css",
            "styles/charts/bars.css",
            "styles/main.css",
            "styles/unused.css",
        ]
    );
    assert_eq!(style.path(SourceId(3)), Some("styles/main.css"));
}

#[tokio::test]
async fn stylesheet_errors() {
    let project = project(&[
        (
            "styles/main.css",
            "@import 'missing.css';\n@import 'a.css';\n@import '../../outside.css';\n@import 'https://example.com/x.css';\n@import 'c.css';\n@import 'a\\\\b.css';\n",
        ),
        ("styles/a.css", "@import 'b.css';\n"),
        ("styles/b.css", "@import 'a.css';\n"),
        ("styles/c.css", "@import 'main.css';\n"),
    ])
    .await
    .unwrap();
    let style = project.load_stylesheet().await.unwrap();
    let messages: Vec<(Option<&str>, &str)> = (style.diagnostics.iter())
        .map(|d| (style.path(d.span.source), d.message.as_str()))
        .collect();
    let main = Some("styles/main.css");
    let b = Some("styles/b.css");
    assert!(
        messages.contains(&(
            main,
            "cannot load `missing.css`: the project has no stylesheet 'styles/missing.css'"
        )),
        "{messages:#?}"
    );
    assert!(
        messages.contains(&(
            b,
            "`styles/a.css` imports itself: styles/a.css → styles/b.css → styles/a.css"
        )),
        "{messages:#?}"
    );
    assert!(
        messages.contains(&(
            main,
            "cannot load `../../outside.css`: the path goes above the project root"
        )),
        "{messages:#?}"
    );
    assert!(
        messages.contains(&(
            main,
            "cannot load `https://example.com/x.css`: an import is a path relative to the \
             stylesheet that holds it"
        )),
        "{messages:#?}"
    );
    // An import whose path holds `\` is not a path relative to the stylesheet.
    assert!(
        messages.contains(&(
            main,
            "cannot load `a\\b.css`: an import is a path relative to the stylesheet that holds it"
        )),
        "{messages:#?}"
    );
    // An import of the entry stylesheet is a cycle too.
    let c = Some("styles/c.css");
    assert!(
        messages.contains(&(
            c,
            "`styles/main.css` imports itself: styles/main.css → styles/c.css → styles/main.css"
        )),
        "{messages:#?}"
    );
    assert!(style.unreached.is_empty());
}

#[tokio::test]
async fn no_entry_stylesheet() {
    let project = project(&[("styles/brand.css", "mark { fill: teal; }\n")])
        .await
        .unwrap();
    let style = project.load_stylesheet().await.unwrap();
    assert!(style.sheet.is_none());
    assert!(style.diagnostics.is_empty());
    assert_eq!(style.unreached, ["styles/brand.css"]);
}

#[tokio::test]
async fn missing_entry_stylesheet() {
    // An entry stylesheet that the manifest names is an error of the manifest at its value
    // when the project lacks it.
    let manifest = "[project]\nname = \"sales\"\n[style]\nstylesheet = \"css/typo.css\"\n";
    let project = project(&[
        ("avenger.toml", manifest),
        ("css/main.css", "mark { fill: teal; }\n"),
    ])
    .await
    .unwrap();
    let error = project.load_stylesheet().await.unwrap_err();
    let ProjectError::Manifest(error) = &error else {
        panic!("{error}");
    };
    assert_eq!(&manifest[error.range.clone().unwrap()], "\"css/typo.css\"");
    assert_eq!(
        error.to_string(),
        "avenger.toml:4: the project has no entry stylesheet 'css/typo.css'"
    );
}

// A project on disk.

#[cfg(not(target_arch = "wasm32"))]
#[tokio::test]
async fn directory() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("sales");
    let files = [
        ("avenger.toml", MANIFEST),
        ("data/sources.av.sql", ""),
        (
            "data/files/sales.csv",
            "region,revenue\neast,10\nwest,20\nnorth,5\n",
        ),
        ("charts/regions/by-month.av.sql", "FROM sales"),
        ("styles/main.css", "@import 'brand.css';\n"),
        ("styles/brand.css", "mark { fill: teal; }\n"),
        (".git/HEAD", "ref: refs/heads/main\n"),
        (".git/objects/style.css", ""),
        ("charts/.hidden.av.sql", ""),
        (".DS_Store", ""),
    ];
    for (path, text) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    // The root of a nested file, and of the root itself.
    let canonical = std::fs::canonicalize(&root).unwrap();
    let nested = root.join("charts/regions/by-month.av.sql");
    assert_eq!(
        avenger_sql_project::find_root(&nested),
        Some(canonical.clone())
    );
    assert_eq!(
        avenger_sql_project::find_root(&root),
        Some(canonical.clone())
    );
    assert_eq!(avenger_sql_project::find_root(dir.path()), None);
    assert!(matches!(
        ProjectFiles::find(dir.path()).await,
        Err(ProjectError::NotInProject(_))
    ));
    // The root of a path that does not exist yet is that of its nearest directory that does.
    assert_eq!(
        avenger_sql_project::find_root(&root.join("charts/new/chart.av.sql")),
        Some(canonical.clone())
    );
    assert_eq!(
        avenger_sql_project::find_root(&dir.path().join("elsewhere/a.av.sql")),
        None
    );
    // A file is not the directory of a source.
    assert!(matches!(
        FileSource::directory(&nested),
        Err(SourceError::Base(_))
    ));

    let project = ProjectFiles::find(root.join("charts/regions"))
        .await
        .unwrap();
    let source = project.source();
    assert_eq!(
        source.base().to_file_path().unwrap(),
        canonical,
        "{}",
        source.base()
    );
    assert!(source.base().as_str().ends_with('/'));
    assert_eq!(project.data_scripts(), ["data/sources.av.sql"]);
    assert_eq!(project.chart_scripts(), ["charts/regions/by-month.av.sql"]);
    // Hidden files and directories are not listed.
    assert_eq!(
        project.files(),
        [
            "avenger.toml",
            "charts/regions/by-month.av.sql",
            "data/files/sales.csv",
            "data/sources.av.sql",
            "styles/brand.css",
            "styles/main.css",
        ]
    );
    assert_eq!(
        source
            .read_text("charts/regions/by-month.av.sql")
            .await
            .unwrap(),
        "FROM sales"
    );
}

/// A project on disk at `root` with the files `files`, each a path relative to the root and its
/// text, and the manifest `MANIFEST`.
#[cfg(unix)]
fn write_project(root: &std::path::Path, files: &[(&str, &str)]) {
    for (path, text) in [("avenger.toml", MANIFEST)].iter().chain(files) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn skipped_directories() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("sales");
    write_project(
        &root,
        &[
            ("charts/a.av.sql", "FROM sales"),
            ("notes/readme.txt", ""),
            ("data/sources.av.sql", ""),
        ],
    );
    // A name that the store's paths can't hold, as macOS's `Icon\r`, fails the listing of its
    // directory.
    std::fs::write(root.join("notes/Icon\r"), "").unwrap();
    let project = ProjectFiles::find(&root).await.unwrap();
    // The project opens without the files of the directories that don't list.
    assert_eq!(project.chart_scripts(), ["charts/a.av.sql"]);
    assert_eq!(project.data_scripts(), ["data/sources.av.sql"]);
    let skipped: Vec<&str> = (project.skipped().iter())
        .map(|skipped| skipped.path.as_str())
        .collect();
    assert_eq!(skipped, ["notes"]);
    assert!(!(project.files().iter()).any(|path| path.starts_with("notes/")));
    let notes = project.skipped().last().unwrap();
    assert!(notes.error.contains("Icon"), "{}", notes.error);
    assert!(
        notes.to_string().starts_with("cannot list 'notes': "),
        "{notes}"
    );
    // A failure to list the listed directory itself is the error.
    assert!(matches!(
        project.source().list("notes").await,
        Err(SourceError::Store(_))
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn links_to_directories() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("sales");
    write_project(
        &root,
        &[
            ("charts/a.av.sql", "FROM sales"),
            ("shared/b.av.sql", "FROM sales"),
        ],
    );
    std::fs::create_dir_all(dir.path().join("outside")).unwrap();
    std::fs::write(dir.path().join("outside/c.av.sql"), "FROM sales").unwrap();
    // A link to a directory above it, a link to a directory that the listing reads at another
    // path, and a link to a directory outside the project.
    std::os::unix::fs::symlink("..", root.join("charts/up")).unwrap();
    std::os::unix::fs::symlink("../shared", root.join("charts/shared")).unwrap();
    std::os::unix::fs::symlink(dir.path().join("outside"), root.join("charts/outside")).unwrap();
    let project = ProjectFiles::find(&root).await.unwrap();
    // The listing reads each directory once, and follows the link out of the project.
    let skipped: Vec<(&str, &str)> = (project.skipped().iter())
        .map(|skipped| (skipped.path.as_str(), skipped.error.as_str()))
        .collect();
    let error = "links to a folder that the listing already reads";
    assert_eq!(skipped, [("charts/shared", error), ("charts/up", error)]);
    assert_eq!(
        project.files(),
        [
            "avenger.toml",
            "charts/a.av.sql",
            "charts/outside/c.av.sql",
            "shared/b.av.sql",
        ]
    );
    assert_eq!(
        project.chart_scripts(),
        ["charts/a.av.sql", "charts/outside/c.av.sql"]
    );
    // An embedded source has no links.
    let source = source(&[("charts/a.av.sql", "")]).await;
    assert!(source.list("").await.unwrap().skipped.is_empty());
}
