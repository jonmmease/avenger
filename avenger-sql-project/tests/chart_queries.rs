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
use avenger_sql_project::{FileSource, LoadedProject, ProjectFiles};
use datafusion::{
    arrow::{
        array::{Int64Array, RecordBatch, StringArray},
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
    let files = ProjectFiles::open(FileSource::directory(root.path()).unwrap())
        .await
        .unwrap();
    let project = LoadedProject::load(files, ctx).await.unwrap();
    (root, project, scans)
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
