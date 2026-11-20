//! The requests of a project across its files, over the texts of the fixture project of
//! avenger-sql-project, `tests/projects/vega/`, with the tables of its files and the listing of
//! its paths that the tests give.

use std::{collections::BTreeMap, sync::Arc};

use avenger_sql::{
    analysis::Options,
    catalog::{FilesTable, MemoryCatalog},
    data::file_reads,
    library::Library,
    types::TypeOptions,
};
use avenger_sql_language::{
    CompletionKind, Diagnostic, FileRole, Offset, Project, ProjectLink, ProjectReference, Severity,
    Source, TextEdit, TextRange,
};
use avenger_sql_syntax::parse_data;
use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    catalog::TableProvider,
    datasource::empty::EmptyTable,
};

macro_rules! fixture {
    ($path:literal) => {
        include_str!(concat!(
            "../../avenger-sql-project/tests/projects/vega/",
            $path
        ))
    };
}

const DATASETS: &str = "data/vega_datasets.sql";
const REGIONS: &str = "data/regions.sql";
const SUMMARIES: &str = "data/summaries.sql";
const ORIGINS: &str = "charts/origins.sql";
const WARM_DAYS: &str = "charts/weather/warm-days.sql";
const WEATHER: &str = "modules/weather.sqlm";

/// The files of the fixture project, with their roles and texts.
fn files() -> Vec<(&'static str, FileRole, &'static str)> {
    let weather = FileRole::Module {
        names: vec!["weather".to_string()],
    };
    vec![
        (DATASETS, FileRole::Data, fixture!("data/vega_datasets.sql")),
        (REGIONS, FileRole::Data, fixture!("data/regions.sql")),
        (SUMMARIES, FileRole::Data, fixture!("data/summaries.sql")),
        (ORIGINS, FileRole::Chart, fixture!("charts/origins.sql")),
        (
            WARM_DAYS,
            FileRole::Chart,
            fixture!("charts/weather/warm-days.sql"),
        ),
        (WEATHER, weather, fixture!("modules/weather.sqlm")),
    ]
}

/// The schema of the files at `uri`, as the loader reads it.
fn schema(uri: &str) -> Schema {
    let f = |name: &str, ty: DataType| Field::new(name, ty, true);
    let fields = match uri {
        "data/files/vega_datasets/cars.json" => vec![
            f("Name", DataType::Utf8),
            f("Miles_per_Gallon", DataType::Int64),
            f("Cylinders", DataType::Int64),
            f("Weight_in_lbs", DataType::Int64),
            f("Origin", DataType::Utf8),
        ],
        "data/files/vega_datasets/seattle-weather.csv" => vec![
            f("date", DataType::Utf8),
            f("precipitation", DataType::Float64),
            f("temp_max", DataType::Float64),
            f("temp_min", DataType::Float64),
            f("wind", DataType::Float64),
            f("weather", DataType::Utf8),
        ],
        "data/files/regions.csv" => vec![f("Origin", DataType::Utf8), f("region", DataType::Utf8)],
        _ => panic!("no files at {uri}"),
    };
    Schema::new(fields)
}

/// The tables of the files that the data scripts `texts` read, by the files that the loader reads
/// the schema of: every file set of the fixture.
fn tables(texts: &[&str]) -> Vec<(avenger_sql::data::Files, FilesTable)> {
    let mut tables = Vec::new();
    for text in texts {
        for read in file_reads(&parse_data(text).syntax(), TypeOptions::default()) {
            if read
                .files
                .uris
                .iter()
                .all(|uri| uri.starts_with("data/files/"))
            {
                let schema = Arc::new(schema(&read.files.uris[0]));
                let table: Arc<dyn TableProvider> = Arc::new(EmptyTable::new(schema));
                tables.push((read.files, Ok(table)));
            }
        }
    }
    tables
}

/// The paths of the fixture project's files.
fn paths() -> Arc<[String]> {
    let mut paths: Vec<String> = files()
        .iter()
        .map(|(path, _, _)| path.to_string())
        .collect();
    paths.extend(
        [
            "avenger.toml",
            "data/files/regions.csv",
            "data/files/vega_datasets/cars.json",
            "data/files/vega_datasets/seattle-weather.csv",
        ]
        .map(str::to_string),
    );
    paths.sort();
    paths.into()
}

/// The project of the fixture, with the host's table `host_sales`.
fn project() -> Project {
    let host = Schema::new(vec![Field::new("amount", DataType::Float64, true)]);
    let base = MemoryCatalog::new().with_schema(&["host_sales"], Arc::new(host));
    let texts: Vec<&str> = files().iter().map(|(_, _, text)| *text).collect();
    Project::new(Arc::new(base), Arc::new(Library::new()), Options::default())
        .with_file_tables(tables(&texts))
        .with_paths(paths())
        .with_files(files())
}

/// The text of the file at `path` of `project`.
fn text<'a>(project: &'a Project, path: &str) -> &'a str {
    project.document(path).unwrap().text()
}

/// The offset of the `n`th `needle` in the file at `path`, plus `plus` bytes.
fn find(project: &Project, path: &str, needle: &str, n: usize, plus: usize) -> Offset {
    let text = text(project, path);
    let at = text.match_indices(needle).nth(n).unwrap().0 + plus;
    Offset::from(at as u32)
}

/// The range of the `n`th `needle` in the file at `path`.
fn range(project: &Project, path: &str, needle: &str, n: usize) -> TextRange {
    let start = find(project, path, needle, n, 0);
    TextRange::at(start, (needle.len() as u32).into())
}

/// The diagnostics of the file at `path`, as its source, its severity, and the text of its range.
fn diagnostics(project: &Project, path: &str) -> Vec<(Source, Severity, String)> {
    let text = text(project, path);
    (project.diagnostics(path).into_iter())
        .map(|d| {
            let at = text[std::ops::Range::<usize>::from(d.range)].to_string();
            (d.source, d.severity, at)
        })
        .collect()
}

/// The texts of `edits`, applied to the files of `project`, by path.
fn applied(project: &Project, edits: &BTreeMap<String, Vec<TextEdit>>) -> BTreeMap<String, String> {
    let mut texts = BTreeMap::new();
    for (path, edits) in edits {
        let mut text = text(project, path).to_string();
        for edit in edits.iter().rev() {
            text.replace_range(
                std::ops::Range::<usize>::from(edit.range),
                &edit.replacement,
            );
        }
        texts.insert(path.clone(), text);
    }
    texts
}

#[test]
fn the_fixture_has_no_diagnostics() {
    let project = project();
    for path in project.files() {
        assert_eq!(project.diagnostics(path), Vec::new(), "{path}");
    }
    assert_eq!(project.graph().order(), [DATASETS, REGIONS, SUMMARIES]);
}

#[test]
fn a_name_of_three_data_scripts_has_one_error_at_each_definition() {
    let mut project = project();
    let text = "CREATE VIEW heavy_by_origin AS (FROM vega_datasets.cars);";
    project.add("data/more.sql", FileRole::Data, text);
    project.add("data/most.sql", FileRole::Data, text);
    for path in ["data/more.sql", "data/most.sql", SUMMARIES] {
        let errors: Vec<String> = (project.diagnostics(path).into_iter())
            .filter(|d| d.source == Source::Project)
            .map(|d| d.message)
            .collect();
        assert_eq!(
            errors,
            ["Table heavy_by_origin is defined by data/more.sql, data/most.sql, and data/summaries.sql: a table of a project is defined by one data script, or is TEMP in each"],
            "{path}"
        );
    }
}

#[test]
fn a_cycle_is_an_error_at_each_read() {
    let mut project = project();
    project.add("data/a.sql", FileRole::Data, "CREATE VIEW a AS (FROM b);");
    project.add("data/b.sql", FileRole::Data, "CREATE VIEW b AS (FROM a);");
    assert_eq!(project.graph().blocked(), ["data/a.sql", "data/b.sql"]);
    for (path, read) in [("data/a.sql", "b"), ("data/b.sql", "a")] {
        let cycle: Vec<Diagnostic> = (project.diagnostics(path).into_iter())
            .filter(|d| d.source == Source::Project)
            .collect();
        assert_eq!(cycle.len(), 1, "{path}");
        let at = find(&project, path, &format!("FROM {read}"), 0, 5);
        assert_eq!(cycle[0].range, TextRange::at(at, 1.into()));
        assert!(cycle[0].message.contains("in a cycle"));
    }
    // A read of the cycle has the cycle's error alone: the first script of the cycle analyzes
    // before the table that it reads, and the second reads a table without a schema.
    for path in ["data/a.sql", "data/b.sql"] {
        assert_eq!(project.diagnostics(path).len(), 1, "{path}");
    }
}

#[test]
fn a_uri_outside_the_root_is_a_warning() {
    let mut project = project();
    let text = "LOAD DATA OVERWRITE vega_datasets.regions
FROM FILES (
  format = 'CSV',
  uris = ['data/files/regions.csv', '../regions.csv', 'data/../../regions.csv', '/data/regions.csv', 'file:///data/regions.csv', 'data/files/../regions.csv', 'C:/data/regions.csv', r'data\\..\\..\\regions.csv']
);";
    project.set_texts([(REGIONS, text)]);
    // A Windows path is outside the root on any machine, and `\` separates names, as `/` does.
    // Each warning is at the literal of its URI.
    assert_eq!(
        diagnostics(&project, REGIONS),
        [
            "'../regions.csv'",
            "'data/../../regions.csv'",
            "'/data/regions.csv'",
            "'file:///data/regions.csv'",
            "'C:/data/regions.csv'",
            r"r'data\..\..\regions.csv'"
        ]
        .map(|uri| (Source::Project, Severity::Warning, uri.to_string()))
    );
    // The loader has read no schema of these files, so their URIs have no error, and the chart's
    // read of their table has the error that names the data script.
    let errors = messages(&project, ORIGINS);
    assert_eq!(
        errors,
        ["Table vega_datasets.regions is defined by the data script data/regions.sql, whose files have no table, since the project has not read them"]
    );
}

/// The messages of the diagnostics of the file at `path`.
fn messages(project: &Project, path: &str) -> Vec<String> {
    (project.diagnostics(path).into_iter())
        .map(|d| d.message)
        .collect()
}

#[test]
fn the_tables_of_files_come_from_the_host() {
    // Without the tables of files, their URIs have no errors, and each read of their tables in
    // another script has the error that names the data script, without the errors of the
    // analysis that its statement would have without the table's schema.
    let base = MemoryCatalog::new();
    let mut project = Project::new(Arc::new(base), Arc::new(Library::new()), Options::default())
        .with_files(files());
    let unread = |names: &str, script: &str| {
        format!("Table {names} is defined by the data script {script}, whose files have no table, since the project has not read them")
    };
    let broken = |names: &str, script: &str| {
        format!("Table {names} is defined by the data script {script}, which has errors")
    };
    assert_eq!(messages(&project, DATASETS), Vec::<String>::new());
    assert_eq!(messages(&project, REGIONS), Vec::<String>::new());
    assert_eq!(
        messages(&project, SUMMARIES),
        [
            unread("vega_datasets.cars", DATASETS),
            unread("vega_datasets.seattle_weather", DATASETS)
        ]
    );
    assert_eq!(
        messages(&project, ORIGINS),
        [
            broken("heavy_by_origin", SUMMARIES),
            unread("vega_datasets.regions", REGIONS)
        ]
    );
    assert_eq!(
        messages(&project, WARM_DAYS),
        [unread("vega_datasets.seattle_weather", DATASETS)]
    );
    // The host's error of reading the files of `vega_datasets.regions`, at its URIs, and at the
    // chart's read of the table.
    let texts: Vec<&str> = files().iter().map(|(_, _, text)| *text).collect();
    let mut tables = tables(&texts);
    let regions = (tables.iter_mut())
        .find(|(files, _)| files.uris == ["data/files/regions.csv"])
        .unwrap();
    regions.1 = Err("No file matches the URI 'data/files/regions.csv'".to_string());
    project.set_file_tables(tables.clone());
    assert_eq!(
        diagnostics(&project, REGIONS),
        [(
            Source::Analysis,
            Severity::Error,
            "['data/files/regions.csv']".to_string()
        )]
    );
    assert_eq!(
        diagnostics(&project, ORIGINS),
        [(
            Source::Project,
            Severity::Error,
            "vega_datasets.regions".to_string()
        )]
    );
    assert_eq!(
        messages(&project, ORIGINS),
        ["Table vega_datasets.regions is defined by the data script data/regions.sql, whose files could not be read"]
    );
    // The table of the files, whose columns lack the chart's `region`.
    let schema = Schema::new(vec![Field::new("Origin", DataType::Utf8, true)]);
    let table: Arc<dyn TableProvider> = Arc::new(EmptyTable::new(Arc::new(schema)));
    let regions = (tables.iter_mut())
        .find(|(files, _)| files.uris == ["data/files/regions.csv"])
        .unwrap();
    regions.1 = Ok(table);
    project.set_file_tables(tables);
    assert_eq!(project.diagnostics(REGIONS), Vec::new());
    let errors: Vec<String> = (project.diagnostics(ORIGINS).into_iter())
        .map(|d| d.message)
        .collect();
    assert_eq!(errors, ["Unrecognized name: region"]);
}

#[test]
fn a_read_of_a_broken_data_script_s_table_names_the_script() {
    let mut project = project();
    // An error in the query of `heavy_by_origin`, which a chart and another data script read.
    let summaries = fixture!("data/summaries.sql").replace("GROUP BY Origin", "GROUP BY nope");
    project.set_texts([(SUMMARIES, summaries)]);
    let more = "CREATE VIEW many AS (\n  FROM heavy_by_origin\n  |> WHERE cars > 100\n);";
    project.add("data/more.sql", FileRole::Data, more);
    assert_eq!(messages(&project, SUMMARIES), ["Unrecognized name: nope"]);
    let message =
        "Table heavy_by_origin is defined by the data script data/summaries.sql, which has errors";
    for path in [ORIGINS, "data/more.sql"] {
        let read = range(&project, path, "heavy_by_origin", 0);
        let errors: Vec<(TextRange, String)> = (project.diagnostics(path).into_iter())
            .map(|d| (d.range, d.message))
            .collect();
        assert_eq!(errors, [(read, message.to_string())], "{path}");
    }
}

#[test]
fn a_broken_data_script_s_table_hides_the_host_s() {
    let mut project = project();
    // A view of the host's table, of the same name, whose statement has an error.
    let view = "CREATE VIEW host_sales AS (FROM host_sales |> WHERE nope > 0);";
    project.add("data/host.sql", FileRole::Data, view);
    project.add("charts/host.sql", FileRole::Chart, "FROM host_sales");
    // The view's statement reads the host's table, and has its own error alone.
    assert_eq!(
        messages(&project, "data/host.sql"),
        ["Unrecognized name: nope"]
    );
    // A chart reads the view, which has no table, and not the host's table.
    assert_eq!(
        messages(&project, "charts/host.sql"),
        ["Table host_sales is defined by the data script data/host.sql, which has errors"]
    );
}

#[test]
fn a_duplicate_appears_and_goes_in_both_scripts() {
    let mut project = project();
    let duplicates = |project: &Project, path: &str| {
        (project.diagnostics(path).into_iter())
            .filter(|d| d.source == Source::Project)
            .map(|d| d.range)
            .collect::<Vec<_>>()
    };
    let regions = fixture!("data/regions.sql");
    let edited = format!("{regions}\nCREATE VIEW wet_days AS (FROM vega_datasets.cars);\n");
    project.set_texts([(REGIONS, edited)]);
    assert_eq!(
        duplicates(&project, REGIONS),
        [range(&project, REGIONS, "wet_days", 0)]
    );
    assert_eq!(
        duplicates(&project, SUMMARIES),
        [range(&project, SUMMARIES, "wet_days", 0)]
    );
    project.set_texts([(REGIONS, regions)]);
    assert_eq!(project.diagnostics(REGIONS), Vec::new());
    assert_eq!(project.diagnostics(SUMMARIES), Vec::new());
}

#[test]
fn the_loader_diagnostics_join_the_project_s() {
    let mut project = project();
    let uris = range(&project, REGIONS, "['data/files/regions.csv']", 0);
    let error = |message: &str| Diagnostic {
        range: uris,
        message: message.to_string(),
        help: None,
        severity: Severity::Error,
        source: Source::Loader,
        code: None,
        fixes: Vec::new(),
    };
    let message = "No file matches the URI 'data/files/regions.csv'";
    // A loader's error that repeats one of the project's is left out.
    project.set_loader_diagnostics(REGIONS, vec![error(message), error(message)]);
    let loader = |project: &Project| {
        (project.diagnostics(REGIONS).into_iter())
            .filter(|d| d.source == Source::Loader)
            .count()
    };
    assert_eq!(loader(&project), 1);
    assert_eq!(project.diagnostics(REGIONS)[0], error(message));
    // An edit leaves out the loader's errors, whose ranges are in the text before the edit.
    let edited = format!("{}\n", text(&project, REGIONS));
    project.set_texts([(REGIONS, edited)]);
    assert_eq!(loader(&project), 0);
}

#[test]
fn a_read_of_a_table_that_did_not_load_has_one_error() {
    let mut project = project();
    // The files of `vega_datasets.cars` could not be read, and the loader's error at the read of
    // `summaries.sql` says why the script that defines it did not load.
    let texts: Vec<&str> = files().iter().map(|(_, _, text)| *text).collect();
    let mut tables = tables(&texts);
    let cars = (tables.iter_mut())
        .find(|(files, _)| files.uris == ["data/files/vega_datasets/cars.json"])
        .unwrap();
    cars.1 = Err("No file matches 'data/files/vega_datasets/cars.json'".to_string());
    project.set_file_tables(tables);
    let read = range(&project, SUMMARIES, "vega_datasets.cars", 0);
    let loader = Diagnostic {
        range: read,
        message: "Table vega_datasets.cars is defined by the data script data/vega_datasets.sql, which failed to load".to_string(),
        help: None,
        severity: Severity::Error,
        source: Source::Loader,
        code: None,
        fixes: Vec::new(),
    };
    project.set_loader_diagnostics(SUMMARIES, vec![loader]);
    let errors: Vec<(Source, String)> = (project.diagnostics(SUMMARIES).into_iter())
        .filter(|d| d.range == read)
        .map(|d| (d.source, d.message))
        .collect();
    assert_eq!(
        errors,
        [(
            Source::Project,
            "Table vega_datasets.cars is defined by the data script data/vega_datasets.sql, whose files could not be read".to_string()
        )]
    );
}

#[test]
fn a_module_completes_the_tables_of_the_data_scripts() {
    const STATS: &str = "modules/stats.sqlm";
    let stats = FileRole::Module {
        names: vec!["stats".to_string()],
    };
    let text = "MODULE stats;\nCREATE PUBLIC TABLE FUNCTION heavy() AS (FROM v);";
    let tables_at = |project: &Project| -> Vec<String> {
        let at = find(project, STATS, "FROM v", 0, "FROM v".len());
        let items = project.document(STATS).unwrap().completions(at).items;
        (items.into_iter())
            .filter(|item| item.kind == CompletionKind::Table)
            .map(|item| item.label)
            .collect()
    };
    // A module of the first files has the tables of the data scripts.
    let mut files = files();
    files.push((STATS, stats, text));
    let texts: Vec<&str> = files.iter().map(|(_, _, text)| *text).collect();
    let mut project = Project::new(
        Arc::new(MemoryCatalog::new()),
        Arc::new(Library::new()),
        Options::default(),
    )
    .with_file_tables(tables(&texts))
    .with_paths(paths())
    .with_files(files);
    assert!(
        tables_at(&project).contains(&"vega_datasets.cars".to_string()),
        "{:?}",
        tables_at(&project)
    );
    // And those after a change of a data script.
    let datasets =
        fixture!("data/vega_datasets.sql").replace("vega_datasets.cars", "vega_datasets.autos");
    project.set_texts([(DATASETS, datasets)]);
    let tables = tables_at(&project);
    assert!(
        tables.contains(&"vega_datasets.autos".to_string()),
        "{tables:?}"
    );
    assert!(
        !tables.contains(&"vega_datasets.cars".to_string()),
        "{tables:?}"
    );
}

#[test]
fn definition_across_files() {
    let project = project();
    let link = |path: &str, needle: &str, n: usize, plus: usize| {
        project.definition(path, find(&project, path, needle, n, plus))
    };
    let target =
        |path: &str, needle: &str, n: usize| (path.to_string(), range(&project, path, needle, n));
    let of = |link: Option<ProjectLink>| link.map(|link| (link.path, link.target));

    // From a chart script to the statements of data scripts.
    let heavy_by_origin = link(ORIGINS, "heavy_by_origin", 0, 3).unwrap();
    assert_eq!(
        heavy_by_origin.origin,
        range(&project, ORIGINS, "heavy_by_origin", 0)
    );
    assert_eq!(
        of(Some(heavy_by_origin)),
        Some(target(SUMMARIES, "heavy_by_origin", 0))
    );
    assert_eq!(
        of(link(ORIGINS, "vega_datasets.regions", 0, 15)),
        Some(target(REGIONS, "vega_datasets.regions", 0))
    );
    // From a data script to another data script, and to a TEMP table of its own.
    assert_eq!(
        of(link(SUMMARIES, "vega_datasets.cars", 0, 15)),
        Some(target(DATASETS, "vega_datasets.cars", 0))
    );
    assert_eq!(
        of(link(SUMMARIES, "FROM heavy\n", 0, 6)),
        Some(target(SUMMARIES, "heavy", 0))
    );
    // From IMPORT MODULE to the MODULE statement of the module's file.
    let import = link(WARM_DAYS, "weather;", 0, 2).unwrap();
    assert_eq!(import.origin, range(&project, WARM_DAYS, "weather", 0));
    assert_eq!(of(Some(import)), Some(target(WEATHER, "MODULE weather", 0)));
    // A table of the host has no definition in the project, and a column's is in its file.
    let mut project = project;
    let host = "FROM host_sales |> SELECT amount AS total |> WHERE total > 0";
    project.add("charts/host.sql", FileRole::Chart, host);
    assert_eq!(project.definition("charts/host.sql", 7.into()), None);
    let total = find(&project, "charts/host.sql", "total", 1, 1);
    let alias = range(&project, "charts/host.sql", "total", 0);
    let link = project.definition("charts/host.sql", total).unwrap();
    assert_eq!(
        (link.path.as_str(), link.target),
        ("charts/host.sql", alias)
    );
}

#[test]
fn references_across_files() {
    let project = project();
    let at = find(&project, DATASETS, "seattle_weather", 0, 2);
    let references = project.references(DATASETS, at);
    let reference = |path: &str, definition: bool| ProjectReference {
        path: path.to_string(),
        range: range(&project, path, "vega_datasets.seattle_weather", 0),
        definition,
    };
    assert_eq!(
        references,
        [
            reference(WARM_DAYS, false),
            reference(SUMMARIES, false),
            reference(DATASETS, true),
        ]
    );
    // The same references from a read in a chart.
    let at = find(&project, WARM_DAYS, "seattle_weather", 0, 2);
    assert_eq!(project.references(WARM_DAYS, at), references);
    // A TEMP table's are in its data script alone.
    let at = find(&project, SUMMARIES, "FROM heavy\n", 0, 6);
    let paths: Vec<String> = (project.references(SUMMARIES, at).into_iter())
        .map(|reference| reference.path)
        .collect();
    assert_eq!(paths, [SUMMARIES, SUMMARIES]);
}

#[test]
fn rename_across_files() {
    let project = project();
    // A table in a schema keeps its schema.
    let at = find(&project, SUMMARIES, "vega_datasets.cars", 0, 16);
    assert_eq!(
        project.prepare_rename(SUMMARIES, at),
        Ok(range(&project, SUMMARIES, "cars", 0))
    );
    let edits = project.rename(SUMMARIES, at, "autos").unwrap();
    let texts = applied(&project, &edits);
    assert_eq!(texts.keys().collect::<Vec<_>>(), [SUMMARIES, DATASETS]);
    assert!(texts[DATASETS].contains("CREATE EXTERNAL TABLE vega_datasets.autos\n"));
    assert!(texts[SUMMARIES].contains("FROM vega_datasets.autos\n"));

    // A table from a chart, with the appends of LOAD DATA INTO.
    let mut project = project;
    let regions = format!(
        "{}\nLOAD DATA INTO vega_datasets.regions\nFROM FILES (format = 'CSV', uris = ['data/files/regions.csv']);\n",
        fixture!("data/regions.sql")
    );
    project.set_texts([(REGIONS, regions)]);
    assert_eq!(project.diagnostics(REGIONS), Vec::new());
    let at = find(&project, ORIGINS, "regions", 0, 1);
    let edits = project.rename(ORIGINS, at, "areas").unwrap();
    assert_eq!(
        edits
            .iter()
            .map(|(path, edits)| (path.as_str(), edits.len()))
            .collect::<Vec<_>>(),
        [(ORIGINS, 1), (REGIONS, 2)]
    );
    let texts = applied(&project, &edits);
    assert!(texts[ORIGINS].contains("JOIN vega_datasets.areas USING"));
    assert_eq!(texts[REGIONS].matches("vega_datasets.areas\n").count(), 2);

    // A TEMP table renames in its data script alone.
    let at = find(&project, SUMMARIES, "FROM heavy\n", 0, 6);
    let edits = project.rename(SUMMARIES, at, "weighty").unwrap();
    assert_eq!(edits.keys().collect::<Vec<_>>(), [SUMMARIES]);
    let texts = applied(&project, &edits);
    assert!(texts[SUMMARIES].contains("CREATE TEMP TABLE weighty AS"));
    assert!(texts[SUMMARIES].contains("FROM weighty\n"));
}

#[test]
fn refused_renames() {
    let mut project = project();
    project.add("charts/host.sql", FileRole::Chart, "FROM host_sales");
    // The name of another table of the project, of the host, or of the script of a TEMP table.
    let at = find(&project, ORIGINS, "heavy_by_origin", 0, 1);
    for name in ["wet_days", "host_sales"] {
        let error = project.rename(ORIGINS, at, name).unwrap_err();
        assert!(error.contains("the name of another table"), "{error}");
    }
    let at = find(&project, SUMMARIES, "FROM heavy\n", 0, 6);
    let error = project
        .rename(SUMMARIES, at, "heavy_by_origin")
        .unwrap_err();
    assert!(error.contains("the name of another table"), "{error}");
    // A table of the host, and a schema.
    let host = project.prepare_rename("charts/host.sql", 7.into());
    assert_eq!(host, Err("A table of the host has no rename".to_string()));
    let at = find(&project, ORIGINS, "vega_datasets", 0, 2);
    let schema = project.prepare_rename(ORIGINS, at);
    assert_eq!(schema, Err("A schema has no rename".to_string()));
    // A rename that adds an error, with errors of the loader that the renamed project lacks.
    let loader = Diagnostic {
        range: TextRange::default(),
        message: "The data script did not load".to_string(),
        help: None,
        severity: Severity::Error,
        source: Source::Loader,
        code: None,
        fixes: Vec::new(),
    };
    project.set_loader_diagnostics(ORIGINS, vec![loader.clone(), loader]);
    let at = find(&project, SUMMARIES, "AS cars", 0, 4);
    let error = project.rename(SUMMARIES, at, "n").unwrap_err();
    assert!(
        error.contains("makes an error: Unrecognized name: cars"),
        "{error}"
    );
    // A rename whose reads would read another table: a temporary table of a chart.
    let temp = "CREATE TEMP TABLE dry AS (FROM host_sales);\nFROM wet_days";
    project.add("charts/temp.sql", FileRole::Chart, temp);
    assert_eq!(project.diagnostics("charts/temp.sql"), Vec::new());
    let at = find(&project, "charts/temp.sql", "wet_days", 0, 1);
    let error = project.rename("charts/temp.sql", at, "dry").unwrap_err();
    assert!(error.contains("changes the table"), "{error}");
}

#[test]
fn an_edit_analyzes_its_dependents_again() {
    let mut project = project();
    let summaries = fixture!("data/summaries.sql");
    // The chart reads the column `cars` of `heavy_by_origin`, which the edit renames.
    let edited = summaries.replace("COUNT(*) AS cars", "COUNT(*) AS n");
    project.set_texts([(SUMMARIES, edited)]);
    let errors: Vec<String> = (project.diagnostics(ORIGINS).into_iter())
        .map(|d| d.message)
        .collect();
    assert_eq!(errors, ["Unrecognized name: cars"]);
    assert_eq!(project.diagnostics(WARM_DAYS), Vec::new());
    // And back.
    project.set_texts([(SUMMARIES, summaries)]);
    assert_eq!(project.diagnostics(ORIGINS), Vec::new());
    // A removed data script removes its tables from the charts that read them.
    assert!(project.remove(REGIONS));
    let errors: Vec<String> = (project.diagnostics(ORIGINS).into_iter())
        .map(|d| d.message)
        .collect();
    assert_eq!(
        errors.first().map(String::as_str),
        Some("Table not found: vega_datasets.regions")
    );
}

#[test]
fn completion_offers_the_project_s_paths() {
    let project = project();
    let document = project.document(REGIONS).unwrap();
    let at = find(
        &project,
        REGIONS,
        "data/files/regions.csv",
        0,
        "data/files/".len(),
    );
    let labels: Vec<String> = (document.completions(at).items.into_iter())
        .map(|item| item.label)
        .collect();
    assert!(
        labels.iter().any(|label| label == "data/files/regions.csv"),
        "{labels:?}"
    );
}

#[test]
fn a_change_of_a_module_analyzes_the_scripts_that_import_it_through_other_modules() {
    const A: &str = "modules/a.sqlm";
    const B: &str = "modules/b.sqlm";
    const USES: &str = "data/uses.sql";
    const CHART: &str = "charts/uses.sql";
    let module = |name: &str| FileRole::Module {
        names: vec![name.to_string()],
    };
    let a = "MODULE a;\nCREATE PUBLIC FUNCTION f() AS (1);";
    let b = "MODULE b;\nIMPORT MODULE a;\nCREATE PUBLIC FUNCTION g() AS (a.f());";
    let renamed = "MODULE a;\nCREATE PUBLIC FUNCTION other() AS (1);";
    let errors = |project: &Project, path: &str| project.diagnostics(path).len();
    // A project whose files are the fixture's and `extra`.
    let project = |extra: Vec<(&'static str, FileRole, &'static str)>| {
        let mut files = files();
        files.extend(extra);
        let texts: Vec<&str> = files.iter().map(|(_, _, text)| *text).collect();
        Project::new(
            Arc::new(MemoryCatalog::new()),
            Arc::new(Library::new()),
            Options::default(),
        )
        .with_file_tables(tables(&texts))
        .with_paths(paths())
        .with_files(files)
    };

    // A data script that imports `b`, which imports `a`.
    let uses =
        "IMPORT MODULE b;\nCREATE VIEW uses AS (FROM vega_datasets.cars |> EXTEND b.g() AS g);";
    let mut data = project(vec![
        (A, module("a"), a),
        (B, module("b"), b),
        (USES, FileRole::Data, uses),
    ]);
    assert_eq!(errors(&data, USES), 0, "{:?}", data.diagnostics(USES));
    data.set_texts([(A, renamed)]);
    assert!(errors(&data, USES) > 0, "the data script analyzes again");

    // A chart that imports `b`, while no data script changes.
    let chart = "IMPORT MODULE b;\nFROM vega_datasets.cars |> EXTEND b.g() AS g";
    let mut charts = project(vec![
        (A, module("a"), a),
        (B, module("b"), b),
        (CHART, FileRole::Chart, chart),
    ]);
    assert_eq!(errors(&charts, CHART), 0, "{:?}", charts.diagnostics(CHART));
    charts.set_texts([(A, renamed)]);
    assert!(errors(&charts, CHART) > 0, "the chart analyzes again");
}

/// A project of `files` over the host's table `host_sales`, without files to read.
fn small(files: Vec<(&'static str, FileRole, &'static str)>) -> Project {
    let host = Schema::new(vec![Field::new("amount", DataType::Float64, true)]);
    let base = MemoryCatalog::new().with_schema(&["host_sales"], Arc::new(host));
    Project::new(Arc::new(base), Arc::new(Library::new()), Options::default()).with_files(files)
}

/// A project of the texts and roles of `project`'s files, built fresh, with the fixture's tables.
fn fresh(project: &Project) -> Project {
    let files: Vec<(String, FileRole, String)> = (project.files())
        .map(|path| {
            let role = project.role(path).unwrap().clone();
            (path.to_string(), role, text(project, path).to_string())
        })
        .collect();
    let texts: Vec<&str> = files.iter().map(|(_, _, text)| text.as_str()).collect();
    let host = Schema::new(vec![Field::new("amount", DataType::Float64, true)]);
    let base = MemoryCatalog::new().with_schema(&["host_sales"], Arc::new(host));
    Project::new(Arc::new(base), Arc::new(Library::new()), Options::default())
        .with_file_tables(tables(&texts))
        .with_paths(paths())
        .with_files(files)
}

fn module(name: &str) -> FileRole {
    FileRole::Module {
        names: vec![name.to_string()],
    }
}

/// The table function of a module that reads the table `t`.
const READS_T: &str = "MODULE m;\nCREATE PUBLIC TABLE FUNCTION f() AS (FROM t);";

#[test]
fn a_table_that_a_module_reads_orders_the_scripts_that_import_it() {
    const READER: &str = "data/a_reader.sql";
    const DEFINER: &str = "data/z_definer.sql";
    let reader = "IMPORT MODULE m;\nCREATE TABLE z AS (FROM m.f());";
    let mut project = small(vec![
        ("modules/m.sqlm", module("m"), READS_T),
        (READER, FileRole::Data, reader),
    ]);
    assert!(!project.diagnostics(READER).is_empty());
    // The script that defines `t` comes first, though its path comes after the reader's.
    project.add(
        DEFINER,
        FileRole::Data,
        "CREATE TABLE t AS (FROM host_sales);",
    );
    assert_eq!(project.graph().order(), [DEFINER, READER]);
    assert_eq!(project.diagnostics(READER), Vec::new());
    // Its removal analyzes the reader again.
    project.remove(DEFINER);
    assert!(!project.diagnostics(READER).is_empty());
}

#[test]
fn an_import_of_a_module_is_not_a_read_of_its_tables() {
    const DEFINER: &str = "data/t.sql";
    const CHART: &str = "charts/k.sql";
    let project = small(vec![
        ("modules/m.sqlm", module("m"), READS_T),
        (
            DEFINER,
            FileRole::Data,
            "CREATE TABLE t AS (FROM host_sales);",
        ),
        (CHART, FileRole::Chart, "IMPORT MODULE m;\nFROM m.f()"),
    ]);
    let at = find(&project, DEFINER, "t AS", 0, 0);
    let references = project.references(DEFINER, at);
    // The module reads `t` in its function, and the chart's import reads none.
    let paths: Vec<&str> = (references.iter())
        .map(|found| found.path.as_str())
        .collect();
    assert_eq!(paths, [DEFINER, "modules/m.sqlm"]);
    // A module reads `t`, and a rename changes no module.
    let error = project.rename(DEFINER, at, "t2").unwrap_err();
    assert_eq!(
        error,
        "The module modules/m.sqlm reads t, and a rename changes no module"
    );
}

#[test]
fn a_rename_that_moves_an_error_is_refused() {
    const DEFINER: &str = "data/a.sql";
    let project = small(vec![
        (
            DEFINER,
            FileRole::Data,
            "CREATE TABLE t AS (FROM host_sales |> SELECT amount AS a);",
        ),
        ("charts/k1.sql", FileRole::Chart, "FROM t |> SELECT b"),
        ("charts/k2.sql", FileRole::Chart, "FROM t |> SELECT a"),
    ]);
    // The rename of the column fixes `k1` and breaks `k2`: as many errors, and other ones.
    let at = find(&project, DEFINER, "AS a", 0, 3);
    let error = project.rename(DEFINER, at, "b").unwrap_err();
    assert!(error.contains("makes an error"), "{error}");
}

#[test]
fn a_table_of_a_chart_without_a_schema_has_its_names() {
    const CHART: &str = "charts/k.sql";
    let chart =
        "CREATE TEMP EXTERNAL TABLE zz OPTIONS (format = 'CSV', uris = ['z.csv']);\nFROM zz";
    let project = small(vec![(CHART, FileRole::Chart, chart)]);
    let read = find(&project, CHART, "FROM zz", 0, 5);
    let link = project.definition(CHART, read).unwrap();
    assert_eq!(link.target, range(&project, CHART, "zz", 0));
    assert_eq!(project.references(CHART, read).len(), 2);
    let edits = project.rename(CHART, read, "sales").unwrap();
    assert_eq!(edits[CHART].len(), 2);
}

#[test]
fn a_read_of_a_script_that_would_not_load_names_why() {
    let project = small(vec![
        // A script with a good table and a broken one.
        (
            "data/a.sql",
            FileRole::Data,
            "CREATE TABLE x AS (FROM host_sales);\nCREATE TABLE y AS (FROM nothing);",
        ),
        // A script that reads a table of the broken script.
        (
            "data/b.sql",
            FileRole::Data,
            "CREATE VIEW v AS (FROM host_sales);\nCREATE VIEW w AS (FROM x);",
        ),
        // Two scripts of one name.
        (
            "data/d1.sql",
            FileRole::Data,
            "CREATE VIEW d AS (FROM host_sales);",
        ),
        (
            "data/d2.sql",
            FileRole::Data,
            "CREATE VIEW d AS (FROM host_sales);",
        ),
        // Scripts in a cycle, one with a table of its own.
        (
            "data/c1.sql",
            FileRole::Data,
            "CREATE VIEW c AS (FROM host_sales);\nCREATE VIEW c1 AS (FROM c2);",
        ),
        (
            "data/c2.sql",
            FileRole::Data,
            "CREATE VIEW c2 AS (FROM c1);",
        ),
        (
            "charts/k.sql",
            FileRole::Chart,
            "FROM x\n|> JOIN v USING (amount)\n|> JOIN d USING (amount)\n|> JOIN c USING (amount)",
        ),
    ]);
    let errors: Vec<String> = (project.diagnostics("charts/k.sql").into_iter())
        .filter(|d| d.source == Source::Project)
        .map(|d| d.message)
        .collect();
    assert_eq!(
        errors,
        [
            "Table x is defined by the data script data/a.sql, which failed to load",
            "Table v is defined by the data script data/b.sql, which did not load, since data/a.sql failed to load",
            "Table d is defined by the data script data/d1.sql, which failed to load",
            "Table c is defined by the data script data/c1.sql, which can't run, since it is in a cycle of data scripts that read each other's tables, or reads one",
        ]
    );
}

#[test]
fn a_table_of_one_name_is_in_the_default_schema() {
    const DEFINER: &str = "data/a.sql";
    const CHART: &str = "charts/k.sql";
    let project = small(vec![
        (
            DEFINER,
            FileRole::Data,
            "CREATE TABLE t AS (FROM host_sales);",
        ),
        (CHART, FileRole::Chart, "FROM public.t"),
    ]);
    assert_eq!(project.diagnostics(CHART), Vec::new());
    let read = find(&project, CHART, "t", 0, 0);
    assert_eq!(project.definition(CHART, read).unwrap().path, DEFINER);
    let edits = project.rename(CHART, read, "t2").unwrap();
    assert_eq!(
        applied(&project, &edits)[CHART],
        "FROM public.t2",
        "{edits:?}"
    );
}

#[test]
fn every_file_completes_the_tables_after_a_change() {
    const CHART: &str = "charts/k.sql";
    let mut project = small(vec![(CHART, FileRole::Chart, "FROM host_sales")]);
    project.add(
        "data/a.sql",
        FileRole::Data,
        "CREATE TABLE added AS (FROM host_sales);",
    );
    let document = project.document(CHART).unwrap();
    let completions = document.completions(Offset::from(5));
    assert!(
        (completions.items.iter()).any(|item| item.label == "added"),
        "{:?}",
        completions.items
    );
}

#[test]
fn a_rename_takes_a_temp_name_of_a_chart_that_does_not_read_the_table() {
    const DEFINER: &str = "data/a.sql";
    let project =
        small(vec![
        (DEFINER, FileRole::Data, "CREATE TABLE t AS (FROM host_sales);"),
        (
            "charts/other.sql",
            FileRole::Chart,
            "CREATE TEMP EXTERNAL TABLE zz OPTIONS (format = 'CSV', uris = ['z.csv']);\nFROM zz",
        ),
    ]);
    let at = find(&project, DEFINER, "t AS", 0, 0);
    assert!(project.rename(DEFINER, at, "zz").is_ok());
    // A read of no table has no rename.
    let project = small(vec![("charts/k.sql", FileRole::Chart, "FROM nothing")]);
    let at = find(&project, "charts/k.sql", "nothing", 0, 0);
    assert_eq!(
        project.prepare_rename("charts/k.sql", at),
        Err("No table nothing to rename".to_string())
    );
}

#[test]
fn edits_leave_the_project_that_a_fresh_one_makes() {
    let mut project = project();
    let check = |project: &Project, step: &str| {
        let fresh = fresh(project);
        for path in project.files() {
            assert_eq!(
                project.diagnostics(path),
                fresh.diagnostics(path),
                "{step}: {path}"
            );
        }
        assert_eq!(project.graph().order(), fresh.graph().order(), "{step}");
    };
    // A table moves to another script, and a cycle comes and goes.
    let moved = fixture!("data/summaries.sql").replace("CREATE VIEW wet_days", "CREATE VIEW dry");
    project.set_texts([(SUMMARIES, moved)]);
    check(&project, "rename");
    project.add(
        "data/wet.sql",
        FileRole::Data,
        "CREATE VIEW wet_days AS (FROM heavy_by_origin);",
    );
    check(&project, "move");
    project.add(
        "data/loop.sql",
        FileRole::Data,
        "CREATE VIEW looped AS (FROM looped_back);",
    );
    project.add(
        "data/back.sql",
        FileRole::Data,
        "CREATE VIEW looped_back AS (FROM looped);",
    );
    check(&project, "cycle");
    project.remove("data/back.sql");
    check(&project, "no cycle");
    // A module that reads a table, a script that imports it, and a role that changes.
    project.add(
        "modules/m.sqlm",
        module("m"),
        "MODULE m;\nCREATE PUBLIC TABLE FUNCTION f() AS (FROM wet_days);",
    );
    project.add(
        "data/uses.sql",
        FileRole::Data,
        "IMPORT MODULE m;\nCREATE TABLE used AS (FROM m.f());",
    );
    check(&project, "module");
    project.remove("data/wet.sql");
    check(&project, "removed");
    project.add("data/loop.sql", FileRole::Chart, "FROM used");
    check(&project, "role");
}
