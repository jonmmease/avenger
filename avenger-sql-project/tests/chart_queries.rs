//! The runs of a project's chart queries through its store of results: the rows that a collect
//! gives, equal to its frame's, the results that a later run reuses, and the files that a run
//! checks before it reuses a result that reads them.

use std::{
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use async_trait::async_trait;
use avenger_sql::analysis::Overrides;
use avenger_sql_project::{ChartQuery, FileSource, LoadedProject, ProjectFiles};
use datafusion::{
    arrow::{
        array::{Date32Array, Int64Array, RecordBatch, StringArray},
        datatypes::{DataType, Field, Schema, SchemaRef},
        util::pretty::pretty_format_batches,
    },
    catalog::{Session, TableProvider},
    common::{Result, ScalarValue},
    datasource::{MemTable, TableType},
    logical_expr::Expr,
    physical_plan::ExecutionPlan,
    prelude::SessionContext,
};

/// A table of the host that counts the scans that runs plan of it.
#[derive(Debug)]
struct Counted {
    table: MemTable,
    scans: Arc<AtomicUsize>,
}

#[async_trait]
impl TableProvider for Counted {
    fn schema(&self) -> SchemaRef {
        self.table.schema()
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        self.table.scan(state, projection, filters, limit).await
    }
}

/// A project in a temporary directory, with a data script of the table `regions` of a CSV file
/// and of the view `east` over it, and the host's table `host`, whose scans count in the
/// returned counter.
async fn project() -> (tempfile::TempDir, LoadedProject, Arc<AtomicUsize>) {
    project_in(SessionContext::new()).await
}

/// The [`project`] in the session `ctx`.
async fn project_in(ctx: SessionContext) -> (tempfile::TempDir, LoadedProject, Arc<AtomicUsize>) {
    let root = tempfile::tempdir().unwrap();
    let files = [
        ("avenger.toml", "[project]\nname = \"charts\"\nformat = 1\n"),
        ("data/files/regions.csv", "region,name\neast,East\nwest,West\n"),
        ("data/files/sales.csv", "k,amount\na,10\nb,20\n"),
        (
            "data/regions.av.sql",
            "CREATE EXTERNAL TABLE regions OPTIONS (format = 'CSV', uris = ['data/files/regions.csv']);\n\
             CREATE VIEW east AS (FROM regions |> WHERE region = 'east');\n",
        ),
    ];
    for (path, text) in files {
        write(root.path(), path, text);
    }
    let schema = Arc::new(Schema::new(vec![
        Field::new("k", DataType::Utf8, false),
        Field::new("v", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["a", "b", "a", "c"])),
            Arc::new(Int64Array::from(vec![1, 2, 3, 4])),
        ],
    )
    .unwrap();
    let scans = Arc::new(AtomicUsize::new(0));
    let host = Counted {
        table: MemTable::try_new(schema, vec![vec![batch]]).unwrap(),
        scans: scans.clone(),
    };
    ctx.register_table("host", Arc::new(host)).unwrap();
    ctx.register_table("events", Arc::new(events())).unwrap();
    let files = ProjectFiles::open(FileSource::directory(root.path()).unwrap())
        .await
        .unwrap();
    let project = LoadedProject::load(files, ctx).await.unwrap();
    (root, project, scans)
}

/// The host's table `events`: a key, a value, and a date in each of four months.
fn events() -> MemTable {
    let schema = Arc::new(Schema::new(vec![
        Field::new("k", DataType::Utf8, false),
        Field::new("v", DataType::Int64, false),
        Field::new("d", DataType::Date32, false),
    ]));
    // Days since 1970-01-01: 2024-01-15, 2024-02-03, 2024-02-20, 2024-03-01, 2024-04-11, 2024-04-30.
    let days = vec![19737, 19756, 19773, 19783, 19824, 19843];
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["a", "b", "a", "c", "b", "a"])),
            Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5, 6])),
            Arc::new(Date32Array::from(days)),
        ],
    )
    .unwrap();
    MemTable::try_new(schema, vec![vec![batch]]).unwrap()
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

const CHART: &str = "charts/chart.av.sql";

/// The rows of the chart script `text`, collected through the project's store, as a table.
async fn collected(project: &LoadedProject, text: &str, overrides: &Overrides) -> String {
    let query = project
        .run_chart_with(CHART, text, overrides)
        .await
        .unwrap();
    let batches = query.collect().await.unwrap();
    for batch in &batches {
        assert_eq!(batch.schema(), query.schema());
    }
    pretty_format_batches(&batches).unwrap().to_string()
}

/// The rows of the chart script `text`, collected from its frame, without the store, as a table.
async fn direct(project: &LoadedProject, text: &str) -> String {
    direct_with(project, text, &Overrides::new()).await
}

#[tokio::test]
async fn a_collect_gives_the_rows_of_its_frame() {
    let (_root, project, _) = project().await;
    let texts = [
        // A variable, which the graph takes as a parameter.
        "DECLARE least INT64 DEFAULT 2;\nFROM host |> WHERE v >= least |> ORDER BY v",
        // A lump of a column whose rows a variable filters, so the graph cuts between them.
        "DECLARE least INT64 DEFAULT 2;\nFROM host |> WHERE v >= least |> SET k = lump(k, n => 1) OVER () |> ORDER BY v",
        // A selection, whose empty state the graph binds, so it keeps every row.
        "DECLARE brush SELECTION;\nFROM host |> WHERE brush AND v > 1 |> ORDER BY v",
        // An order, and projections after it.
        "FROM host |> ORDER BY v DESC |> SELECT k |> SELECT k AS key",
        // Two columns of one name.
        "FROM host |> SELECT k, v AS k |> ORDER BY 2",
    ];
    for text in texts {
        let overrides = Overrides::new();
        assert_eq!(
            collected(&project, text, &overrides).await,
            direct(&project, text).await,
            "{text}"
        );
    }
}

#[tokio::test]
async fn a_collect_of_a_shared_input_gives_the_rows_of_its_frame() {
    // One partition, so the groups of the mark's items keep one order from run to run.
    let config = datafusion::prelude::SessionConfig::new().with_target_partitions(1);
    let (_root, project, _) = project_in(SessionContext::new_with_config(config)).await;
    // A mark whose channels compute statistics reads a shared input.
    let text = "FROM host |> AGGREGATE mark_bar(x => k, y => SUM(v)) AS chart";
    assert_eq!(
        collected(&project, text, &Overrides::new()).await,
        direct(&project, text).await
    );
}

#[tokio::test]
async fn a_chart_column_keeps_its_extension_type() {
    let (_root, project, _) = project().await;
    let text = "FROM host |> AGGREGATE mark_bar(x => k, y => SUM(v)) AS chart";
    let query = project.run_chart(CHART, text).await.unwrap();
    let batches = query.collect().await.unwrap();
    let schema = batches[0].schema();
    assert!(avenger_chart::results::is_chart_field(
        schema.field_with_name("chart").unwrap()
    ));
}

#[tokio::test]
async fn an_unchanged_chart_runs_again_from_its_kept_result() {
    let (_root, project, scans) = project().await;
    let texts = [
        "FROM host |> AGGREGATE SUM(v) AS total GROUP BY k |> ORDER BY k",
        // Charts whose channels call `value`, `datum`, a channel function, and a restyle, over
        // the rows of the table and over an aggregate of them.
        "DECLARE brush SELECTION;\nFROM host |> AGGREGATE mark_symbol(x => x(v, title => 'V'), y => v, \
         fill => IF(brush, value('red'), value('gray')), opacity => datum(0.5)) AS chart",
        "FROM host |> AGGREGATE mark_bar(x => k, y => SUM(v), fill => value('red')) AS chart",
        // A table of files, whose listing a run checks before it reuses a result that reads it.
        "CREATE TEMP EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/files/sales.csv']);\n\
         FROM host JOIN sales USING (k) |> SELECT k, v, amount |> ORDER BY v",
    ];
    for text in texts {
        let overrides = Overrides::new();
        let first = collected(&project, text, &overrides).await;
        let scanned = scans.load(Ordering::SeqCst);
        assert_eq!(collected(&project, text, &overrides).await, first, "{text}");
        assert_eq!(scans.load(Ordering::SeqCst), scanned, "{text}");
    }
}

#[tokio::test]
async fn a_new_value_of_a_variable_reruns_what_reads_it() {
    let (_root, project, scans) = project().await;
    let text = "DECLARE least INT64 DEFAULT 0;\n\
                FROM host |> AGGREGATE SUM(v) AS total GROUP BY k |> WHERE total >= least |> ORDER BY k";
    let least =
        |value: i64| Overrides::from([("least".to_string(), ScalarValue::Int64(Some(value)))]);
    let all = collected(&project, text, &least(0)).await;
    let scanned = scans.load(Ordering::SeqCst);
    let some = collected(&project, text, &least(3)).await;
    // The aggregate below the filter is kept, so the new value reads no table.
    assert_eq!(scans.load(Ordering::SeqCst), scanned);
    assert_ne!(some, all);
    assert_eq!(some, direct_with(&project, text, &least(3)).await);
}

/// The rows of the chart script `text` with `overrides`, collected from its frame, as a table.
async fn direct_with(project: &LoadedProject, text: &str, overrides: &Overrides) -> String {
    let query = project
        .run_chart_with(CHART, text, overrides)
        .await
        .unwrap();
    let batches = query.into_frame().collect().await.unwrap();
    pretty_format_batches(&batches).unwrap().to_string()
}

#[tokio::test]
async fn a_changed_file_gives_its_new_rows() {
    let (root, mut project, _) = project().await;
    let texts = [
        "FROM regions |> ORDER BY region",
        "FROM east",
        "CREATE TEMP EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/files/sales.csv']);\n\
         FROM sales |> AGGREGATE SUM(amount) AS total",
    ];
    let overrides = Overrides::new();
    let before: Vec<String> = collect_all(&project, &texts, &overrides).await;
    // Without a reload, as when no watcher sees the change, a run lists the files again.
    write(
        root.path(),
        "data/files/regions.csv",
        "region,name\neast,Eastern\nwest,West\nnorth,North\n",
    );
    write(
        root.path(),
        "data/files/sales.csv",
        "k,amount\na,10\nb,20\nc,30\n",
    );
    let after = collect_all(&project, &texts, &overrides).await;
    for ((text, before), after) in texts.iter().zip(&before).zip(&after) {
        assert_ne!(before, after, "{text}");
        assert_eq!(*after, direct(&project, text).await, "{text}");
    }
    // After a reload, which registers the tables again.
    project
        .reload_files(&["data/files/regions.csv", "data/files/sales.csv"])
        .await;
    for (text, after) in texts.iter().zip(&after) {
        assert_eq!(
            collected(&project, text, &overrides).await,
            *after,
            "{text}"
        );
    }
}

/// The collected rows of each of `texts`, in order.
async fn collect_all(
    project: &LoadedProject,
    texts: &[&str],
    overrides: &Overrides,
) -> Vec<String> {
    let mut rows = Vec::new();
    for text in texts {
        rows.push(collected(project, text, overrides).await);
    }
    rows
}

#[tokio::test]
async fn a_chart_reruns_with_the_clauses_of_its_selections() {
    use avenger_selection::{Clause, Condition};
    let (_root, project, _) = project().await;
    let text = "DECLARE brush SELECTION;\nDECLARE other SELECTION;\n\
                FROM host |> WHERE brush |> ORDER BY v";
    let query = project.run_chart(CHART, text).await.unwrap();
    // The query reads `brush`, so only a new state of `brush` reruns it.
    assert_eq!(query.reads().into_iter().collect::<Vec<_>>(), ["brush"]);
    let values = Arc::new(StringArray::from(vec!["a"]));
    let clauses = vec![Clause::new("scatter").condition(Condition::values("k", None, values))];
    let states = [("brush".to_string(), clauses)];
    let query = query
        .with_clauses(states.iter().map(|(name, clauses)| (name, clauses)))
        .await
        .unwrap();
    let rows = pretty_format_batches(&query.collect().await.unwrap())
        .unwrap()
        .to_string();
    let framed = pretty_format_batches(&query.frame().clone().collect().await.unwrap())
        .unwrap()
        .to_string();
    let expected = "\
+---+---+
| k | v |
+---+---+
| a | 1 |
| a | 3 |
+---+---+";
    assert_eq!(rows, expected);
    assert_eq!(framed, expected);
}

/// The chart bytes of the first chart column of `batches`.
fn chart_bytes(batches: &[RecordBatch]) -> Vec<u8> {
    use datafusion::arrow::array::AsArray;
    let column = batches[0].column(0);
    match column.data_type() {
        DataType::BinaryView => column.as_binary_view().value(0).to_vec(),
        _ => column.as_binary::<i32>().value(0).to_vec(),
    }
}

/// The clauses of a brush of the values `v` from `low` to `high`.
fn brushed(low: i64, high: i64) -> Vec<avenger_selection::Clause> {
    use avenger_selection::{Clause, Condition};
    let ends = Arc::new(Int64Array::from(vec![low, high]));
    vec![Clause::new("scatter").condition(Condition::Range {
        column: "v".into(),
        channel: None,
        ends,
        exclude_end: false,
        cells: None,
    })]
}

/// The page of `query`, its graph's file, and the chart that the page builds for each of a series
/// of states of the selection `brush`, each equal to the native run's chart, or the page's
/// warnings.
async fn page_charts(query: &ChartQuery) -> Result<(Vec<u8>, Vec<Vec<u8>>), Vec<String>> {
    use avenger_physical_graph::{
        codec::Hosts,
        page::{task_context, PageGraph},
    };
    let page = query.page().await.unwrap();
    if !page.warnings.is_empty() {
        return Err(page.warnings);
    }
    // A page that knows avenger-sql's library, as avenger-query-web's does.
    let hosts = Hosts::default().with(avenger_sql::library::codec::host());
    let graph = PageGraph::decode(&page.bytes, Arc::new(task_context(hosts))).unwrap();
    let mut charts = Vec::new();
    for clauses in [Vec::new(), brushed(2, 3), brushed(1, 1), Vec::new()] {
        let selections = std::collections::BTreeMap::from([("brush".to_string(), clauses)]);
        let native = (query.clone().with_clauses(&selections).await.unwrap())
            .collect()
            .await
            .unwrap();
        let ran = graph.run(&selections).await.unwrap();
        assert_eq!(chart_bytes(&ran[0]), chart_bytes(&native));
        charts.push(chart_bytes(&ran[0]));
    }
    Ok((page.bytes, charts))
}

/// A session of one partition, so the groups of the marks' items keep one order from run to run.
async fn one_partition() -> (tempfile::TempDir, LoadedProject) {
    let config = datafusion::prelude::SessionConfig::new().with_target_partitions(1);
    let (root, project, _) = project_in(SessionContext::new_with_config(config)).await;
    (root, project)
}

#[tokio::test]
async fn a_page_builds_the_charts_of_the_native_run() {
    let (_root, project) = one_partition().await;
    let text = "DECLARE brush SELECTION;\n\
                FROM host |> WHERE brush |> AGGREGATE mark_bar(x => k, y => SUM(v)) AS chart";
    let query = project.run_chart(CHART, text).await.unwrap();
    let (bytes, charts) = page_charts(&query).await.unwrap();
    assert_ne!(charts[0], charts[1]);
    // The page's file and a brushed chart, which the browser's test of avenger-physical-graph
    // runs, written beside its baselines; accept them by copying the output over the baselines.
    let output =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../avenger-physical-graph/tests/output");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(output.join("bars.graph"), &bytes).unwrap();
    std::fs::write(output.join("bars-brushed.chart"), &charts[1]).unwrap();
}

/// The cases of the survey: a chart that filters `events` by `WHERE brush` and calls one function
/// above the filter, by the function's name.
const SURVEY: &[(&str, &str)] = &[
    ("UPPER", "|> SELECT UPPER(k) AS x, v |> AGGREGATE mark_bar(x => x, y => SUM(v)) AS chart"),
    (
        "layer",
        "|> AGGREGATE layer(mark_bar(x => k, y => SUM(v)), mark_symbol(x => k, y => SUM(v))) AS chart",
    ),
    (
        "FORMAT_DATE",
        "|> SELECT FORMAT_DATE('%b', d) AS x, v |> AGGREGATE mark_bar(x => x, y => SUM(v)) AS chart",
    ),
    (
        "PARSE_DATE",
        "|> SELECT PARSE_DATE('%Y-%m-%d', FORMAT_DATE('%Y-%m-01', d)) AS x, v \
         |> AGGREGATE mark_bar(x => x, y => SUM(v)) AS chart",
    ),
    (
        "CAST AS STRING",
        "|> SELECT CAST(v * 1.5 AS STRING) AS x, v |> AGGREGATE mark_bar(x => x, y => SUM(v)) AS chart",
    ),
    (
        "TO_HEX",
        "|> SELECT TO_HEX(CAST(k AS BYTES)) AS x, v |> AGGREGATE mark_bar(x => x, y => SUM(v)) AS chart",
    ),
    (
        "INTERVAL",
        "|> SELECT DATE_ADD(d, INTERVAL v DAY) AS x, v |> AGGREGATE mark_bar(x => x, y => SUM(v)) AS chart",
    ),
    (
        "REGEXP_CONTAINS",
        "|> SELECT IF(REGEXP_CONTAINS(k, r'^a'), 'a', 'other') AS x, v \
         |> AGGREGATE mark_bar(x => x, y => SUM(v)) AS chart",
    ),
    (
        "FORMAT",
        "|> SELECT FORMAT('%d', v) AS x, v |> AGGREGATE mark_bar(x => x, y => SUM(v)) AS chart",
    ),
    (
        "INITCAP",
        "|> SELECT INITCAP(k) AS x, v |> AGGREGATE mark_bar(x => x, y => SUM(v)) AS chart",
    ),
    ("bin", "|> AGGREGATE mark_bar(x => bin(v), y => COUNT(*)) AS chart"),
    (
        "timeunit",
        "|> AGGREGATE mark_bar(x => timeunit(d, unit => 'month'), y => COUNT(*)) AS chart",
    ),
    ("lump", "|> AGGREGATE mark_bar(x => lump(k, n => 1), y => COUNT(*)) AS chart"),
    (
        "stack",
        "|> AGGREGATE mark_area(x => v, y => stack(COUNT(*)), color => k) AS chart",
    ),
    (
        "impute",
        "|> AGGREGATE COUNT(*) AS c GROUP BY k, v |> CALL impute(v, [k], c, value => 0) \
         |> AGGREGATE mark_line(x => v, y => c, color => k) AS chart",
    ),
    (
        "MATCH_RECOGNIZE",
        "|> MATCH_RECOGNIZE(PARTITION BY k ORDER BY v MEASURES first(v) AS low, last(v) AS high \
         PATTERN (up+) DEFINE up AS v > 0) |> AGGREGATE mark_bar(x => k, y => SUM(high)) AS chart",
    ),
];


#[tokio::test]
async fn a_page_builds_the_charts_of_each_function() {
    let (_root, project) = one_partition().await;
    let mut wrong = Vec::new();
    for (name, pipes) in SURVEY {
        let text = format!("DECLARE brush SELECTION;\nFROM events |> WHERE brush {pipes}");
        let query = project.run_chart(CHART, &text).await.unwrap();
        if let Err(warnings) = page_charts(&query).await {
            wrong.push(format!("{name}: {warnings:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
