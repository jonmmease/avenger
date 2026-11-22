//! The data scripts of a project together: the graph of their dependencies, its errors, its
//! order, and the project catalog of their tables, without I/O.

use std::sync::{Arc, OnceLock};

use avenger_sql::{
    analysis::Options,
    analyze_data,
    catalog::{Catalog, MemoryCatalog},
    data::{file_reads, DataAnalysis},
    library::Library,
    project::{script_names, Graph, GraphError, NameKind, ProjectCatalog},
    types::TypeOptions,
};
use avenger_sql_syntax::parse_data;
use datafusion::{
    arrow::datatypes::{DataType, Field, Schema, SchemaRef},
    datasource::empty::EmptyTable,
};

fn library() -> &'static Library {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY.get_or_init(Library::new)
}

/// The schema of the files of the tests: a region, a month, and an amount.
fn sales() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, true),
        Field::new("month", DataType::Int64, true),
        Field::new("amount", DataType::Float64, true),
    ]))
}

fn names(path: &str) -> Vec<String> {
    path.split('.').map(str::to_string).collect()
}

/// The graph of the data scripts `scripts`, by path and text, which have no syntax errors, in a
/// project without modules whose default schema is `public`.
fn graph(scripts: &[(&str, &str)]) -> Graph {
    graph_with_modules(scripts, &[])
}

/// The graph of the data scripts `scripts`, as [`graph`] gives it, in a project with the modules
/// `modules`, by module path and text.
fn graph_with_modules(scripts: &[(&str, &str)], modules: &[(&str, &str)]) -> Graph {
    let roots = scripts.iter().map(|(path, text)| {
        let parse = parse_data(text);
        assert!(parse.ok(), "{path}: {:?}", parse.diagnostics());
        (*path, parse.syntax())
    });
    let module = |path: &[String]| {
        (modules.iter())
            .find(|(names, _)| names.eq_ignore_ascii_case(&path.join(".")))
            .map(|(_, text)| text.to_string())
    };
    Graph::new(roots, &module, "public")
}

/// The text of `scripts` at the path `path`.
fn text<'a>(scripts: &[(&str, &'a str)], path: &str) -> &'a str {
    scripts.iter().find(|(p, _)| *p == path).unwrap().1
}

/// `catalog`, with a table of the schema of [`sales`] for each file read of `text`.
fn with_files(text: &str, catalog: MemoryCatalog) -> MemoryCatalog {
    let reads = file_reads(&parse_data(text).syntax(), TypeOptions::default());
    (reads.into_iter()).fold(catalog, |catalog, read| {
        catalog.with_files_schema(read.files, sales())
    })
}

fn analyze(text: &str, catalog: &dyn Catalog) -> DataAnalysis {
    let parse = parse_data(text);
    assert!(parse.ok(), "{:?}", parse.diagnostics());
    analyze_data(&parse.syntax(), catalog, library(), &Options::default())
}

const SALES: &str = "CREATE EXTERNAL TABLE sales
OPTIONS (format = 'CSV', uris = ['data/files/sales.csv'], description = 'Monthly sales')";

const REGIONS: &str = "LOAD DATA OVERWRITE regions (region STRING, manager STRING)
FROM FILES (format = 'CSV', uris = ['data/files/regions.csv'], skip_leading_rows = 1)";

const DERIVED: &str = "CREATE VIEW revenue AS (
  FROM sales
  |> JOIN regions USING (region)
  |> AGGREGATE sum(amount) AS revenue GROUP BY manager
)";

#[test]
fn the_three_scripts_of_the_proposal_run_in_the_order_of_their_reads() {
    let scripts = [
        ("data/derived.sql", DERIVED),
        ("data/sales.sql", SALES),
        ("data/regions.sql", REGIONS),
    ];
    let graph = graph(&scripts);
    assert!(graph.errors().is_empty(), "{:?}", graph.errors());
    // `regions` and `sales` are ready at once, and run by path.
    assert_eq!(
        graph.order(),
        ["data/regions.sql", "data/sales.sql", "data/derived.sql"]
    );
    assert!(graph.blocked().is_empty());
    assert_eq!(
        graph.paths().collect::<Vec<_>>(),
        ["data/derived.sql", "data/regions.sql", "data/sales.sql"]
    );

    let edges: Vec<(&str, &str, &str)> = (graph.edges().iter())
        .map(|edge| {
            let read = &text(&scripts, &edge.script)[edge.read.range];
            (edge.script.as_str(), edge.dependency.as_str(), read)
        })
        .collect();
    assert_eq!(
        edges,
        [
            ("data/derived.sql", "data/sales.sql", "sales"),
            ("data/derived.sql", "data/regions.sql", "regions"),
        ]
    );
    let edge = &graph.edges()[1];
    assert_eq!(&REGIONS[edge.definition], "regions");

    assert_eq!(
        graph.dependencies("data/derived.sql"),
        ["data/regions.sql", "data/sales.sql"]
    );
    assert!(graph.dependencies("data/sales.sql").is_empty());
    assert_eq!(graph.downstream("data/sales.sql"), ["data/derived.sql"]);
    assert!(graph.downstream("data/derived.sql").is_empty());

    let derived = graph.names("data/derived.sql").unwrap();
    let defined: Vec<String> = derived
        .definitions
        .iter()
        .map(|n| n.names.join("."))
        .collect();
    assert_eq!(defined, ["revenue"]);
    assert!(graph.names("data/missing.sql").is_none());
}

#[test]
fn scripts_that_are_ready_at_once_run_by_path() {
    let scripts = [
        ("z.sql", "CREATE VIEW z AS (FROM host)"),
        ("a.sql", "CREATE VIEW a AS (FROM z)"),
        ("m.sql", "CREATE VIEW m AS (FROM host)"),
        ("b.sql", "CREATE VIEW b AS (FROM a |> JOIN m USING (k))"),
    ];
    let graph = graph(&scripts);
    assert!(graph.errors().is_empty());
    // `a` waits for `z`, so `m` runs before `z`, and `a` after it, though `a` comes first by path.
    assert_eq!(graph.order(), ["m.sql", "z.sql", "a.sql", "b.sql"]);
}

#[test]
fn a_cycle_of_three_scripts_is_an_error_at_the_read_of_each() {
    let scripts = [
        ("c.sql", "CREATE VIEW c AS (FROM a)"),
        ("a.sql", "CREATE VIEW a AS (FROM host |> JOIN b USING (k))"),
        ("b.sql", "CREATE VIEW b AS (FROM c)"),
        ("d.sql", "CREATE VIEW d AS (FROM a)"),
        ("e.sql", "CREATE VIEW e AS (FROM host)"),
    ];
    let graph = graph(&scripts);
    assert_eq!(graph.order(), ["e.sql"]);
    // `d` depends on the cycle, and is in none.
    assert_eq!(graph.blocked(), ["a.sql", "b.sql", "c.sql", "d.sql"]);
    assert_eq!(graph.errors().len(), 1);
    let error = &graph.errors()[0];
    let GraphError::Cycle { reads } = error else {
        panic!("{error:?}");
    };
    let steps: Vec<(&str, &str, &str)> = (reads.iter())
        .map(|edge| {
            let read = &text(&scripts, &edge.script)[edge.read.range];
            (edge.script.as_str(), read, edge.dependency.as_str())
        })
        .collect();
    assert_eq!(
        steps,
        [
            ("a.sql", "b", "b.sql"),
            ("b.sql", "c", "c.sql"),
            ("c.sql", "a", "a.sql"),
        ]
    );
    let places: Vec<String> = (error.places().into_iter())
        .map(|place| place.path)
        .collect();
    assert_eq!(places, ["a.sql", "b.sql", "c.sql"]);
    assert_eq!(
        error.message(),
        "Data scripts read each other's tables in a cycle: a.sql reads b of b.sql, b.sql reads c of c.sql, c.sql reads a of a.sql. A data script runs after the scripts that it reads, so none of these can run"
    );
    assert_eq!(graph.downstream("e.sql"), Vec::<&str>::new());
    assert_eq!(graph.downstream("c.sql"), ["a.sql", "b.sql", "d.sql"]);
}

/// The scripts of each cycle error of `graph`, in the order of its reads.
fn cycles(graph: &Graph) -> Vec<Vec<&str>> {
    (graph.errors().iter())
        .filter_map(|error| match error {
            GraphError::Cycle { reads } => {
                Some(reads.iter().map(|edge| edge.script.as_str()).collect())
            }
            GraphError::Duplicate { .. } => None,
        })
        .collect()
}

#[test]
fn each_script_in_a_cycle_has_an_error() {
    // `a` and `b` read each other, and so do `b` and `c`: one component of two cycles.
    let scripts = [
        ("a.sql", "CREATE VIEW a AS (FROM b)"),
        ("b.sql", "CREATE VIEW b AS (FROM a |> JOIN c USING (k))"),
        ("c.sql", "CREATE VIEW c AS (FROM b)"),
    ];
    let joined = graph(&scripts);
    assert_eq!(
        cycles(&joined),
        [vec!["a.sql", "b.sql"], vec!["c.sql", "b.sql"]]
    );
    assert_eq!(joined.blocked(), ["a.sql", "b.sql", "c.sql"]);

    // Two cycles apart are two errors.
    let scripts = [
        ("a.sql", "CREATE VIEW a AS (FROM b)"),
        ("b.sql", "CREATE VIEW b AS (FROM a)"),
        ("c.sql", "CREATE VIEW c AS (FROM d)"),
        ("d.sql", "CREATE VIEW d AS (FROM c)"),
    ];
    let apart = graph(&scripts);
    assert_eq!(
        cycles(&apart),
        [vec!["a.sql", "b.sql"], vec!["c.sql", "d.sql"]]
    );
}

#[test]
fn a_cycle_through_a_schema_is_an_error_at_the_schema() {
    let scripts = [
        (
            "a.sql",
            "CREATE SCHEMA s;
CREATE VIEW v AS (FROM s.t)",
        ),
        ("b.sql", "CREATE VIEW s.t AS (FROM host)"),
    ];
    let graph = graph(&scripts);
    let [GraphError::Cycle { reads }] = graph.errors() else {
        panic!("{:?}", graph.errors());
    };
    let steps: Vec<(&str, NameKind, &str)> = (reads.iter())
        .map(|edge| {
            let read = &text(&scripts, &edge.script)[edge.read.range];
            (edge.script.as_str(), edge.read.kind, read)
        })
        .collect();
    assert_eq!(
        steps,
        [
            ("a.sql", NameKind::Table, "s.t"),
            ("b.sql", NameKind::Schema, "s"),
        ]
    );
    assert_eq!(
        graph.errors()[0].message(),
        "Data scripts read each other's tables in a cycle: a.sql reads s.t of b.sql, b.sql reads schema s of a.sql. A data script runs after the scripts that it reads, so none of these can run"
    );
}

#[test]
fn a_name_that_two_scripts_define_is_an_error_at_both() {
    let scripts = [
        (
            "a.sql",
            "CREATE SCHEMA finance;
CREATE VIEW sales AS (FROM host)",
        ),
        (
            "b.sql",
            "CREATE SCHEMA Finance;
LOAD DATA INTO SALES FROM FILES (format = 'CSV', uris = ['b.csv'])",
        ),
        ("c.sql", "CREATE VIEW sales AS (FROM host)"),
        ("d.sql", "CREATE VIEW totals AS (FROM sales)"),
    ];
    let graph = graph(&scripts);
    let errors: Vec<(String, Vec<(&str, &str)>)> = (graph.errors().iter())
        .map(|error| {
            let places = (error.places().into_iter())
                .map(|place| {
                    let path = scripts.iter().find(|(p, _)| *p == place.path).unwrap().0;
                    (path, &text(&scripts, path)[place.range])
                })
                .collect();
            (error.message(), places)
        })
        .collect();
    assert_eq!(
        errors,
        [
            (
                "Schema finance, which BigQuery calls a dataset, is created by both a.sql and b.sql: a schema of a project is created by one data script".to_string(),
                vec![("a.sql", "finance"), ("b.sql", "Finance")],
            ),
            (
                "Table sales is defined by both a.sql and b.sql: a table of a project is defined by one data script, or is TEMP in each".to_string(),
                vec![("a.sql", "sales"), ("b.sql", "SALES")],
            ),
            (
                "Table sales is defined by both a.sql and c.sql: a table of a project is defined by one data script, or is TEMP in each".to_string(),
                vec![("a.sql", "sales"), ("c.sql", "sales")],
            ),
            (
                "Table SALES is defined by both b.sql and c.sql: a table of a project is defined by one data script, or is TEMP in each".to_string(),
                vec![("b.sql", "SALES"), ("c.sql", "sales")],
            ),
        ]
    );
    // A duplicate blocks no script, and a read of it depends on each script that defines it.
    assert_eq!(graph.order(), ["a.sql", "b.sql", "c.sql", "d.sql"]);
    assert_eq!(graph.dependencies("d.sql"), ["a.sql", "b.sql", "c.sql"]);
}

#[test]
fn temp_names_repeat_across_scripts_and_stay_in_their_script() {
    let scripts = [
        (
            "events.sql",
            "CREATE TEMP EXTERNAL TABLE raw
OPTIONS (format = 'CSV', uris = ['data/files/events/*.csv']);
CREATE TABLE events AS (FROM raw |> WHERE amount IS NOT NULL)",
        ),
        (
            "orders.sql",
            "LOAD DATA OVERWRITE TEMP TABLE raw FROM FILES (format = 'CSV', uris = ['o.csv']);
CREATE TEMP VIEW recent AS (FROM raw |> WHERE month > 6);
CREATE TABLE orders AS (FROM recent)",
        ),
        ("other.sql", "CREATE VIEW r AS (FROM raw)"),
    ];
    let graph = graph(&scripts);
    assert!(graph.errors().is_empty(), "{:?}", graph.errors());
    // No script reads a TEMP table of another, so `raw` of `other.sql` is the host's.
    assert!(graph.edges().is_empty());
    let defined = |path: &str| -> Vec<String> {
        let names = graph.names(path).unwrap();
        names
            .definitions
            .iter()
            .map(|n| n.names.join("."))
            .collect()
    };
    assert_eq!(defined("events.sql"), ["events"]);
    assert_eq!(defined("orders.sql"), ["orders"]);
    assert!(graph.names("events.sql").unwrap().reads.is_empty());
    let reads = &graph.names("other.sql").unwrap().reads;
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].names, names("raw"));
}

#[test]
fn a_table_in_a_schema_depends_on_the_script_that_creates_the_schema() {
    let scripts = [
        (
            "totals.sql",
            "CREATE VIEW totals AS (FROM finance.orders |> AGGREGATE sum(amount) AS total)",
        ),
        (
            "orders.sql",
            "CREATE EXTERNAL TABLE finance.orders
OPTIONS (format = 'CSV', uris = ['data/files/orders.csv'])",
        ),
        (
            "finance.sql",
            "CREATE SCHEMA finance OPTIONS (description = 'The books')",
        ),
    ];
    let graph = graph(&scripts);
    assert!(graph.errors().is_empty(), "{:?}", graph.errors());
    assert_eq!(graph.order(), ["finance.sql", "orders.sql", "totals.sql"]);
    let edges: Vec<(&str, &str, NameKind, &str)> = (graph.edges().iter())
        .map(|edge| {
            let read = &text(&scripts, &edge.script)[edge.read.range];
            (
                edge.script.as_str(),
                edge.dependency.as_str(),
                edge.read.kind,
                read,
            )
        })
        .collect();
    assert_eq!(
        edges,
        [
            // A definition reads the schema of its name.
            ("orders.sql", "finance.sql", NameKind::Schema, "finance"),
            // A read of a table in a schema reads the table alone.
            (
                "totals.sql",
                "orders.sql",
                NameKind::Table,
                "finance.orders"
            ),
        ]
    );
    assert_eq!(
        graph.downstream("finance.sql"),
        ["orders.sql", "totals.sql"]
    );
    assert_eq!(graph.dependencies("totals.sql"), ["orders.sql"]);
}

#[test]
fn a_read_of_a_host_table_in_a_schema_of_a_script_adds_no_dependency() {
    // `a.sql` reads a table of the host in the schema `s`, which `c.sql` creates, and `c.sql`
    // reads `v` of `a.sql`: the scripts are in no cycle.
    let scripts = [
        (
            "c.sql",
            "CREATE SCHEMA s;
CREATE VIEW x AS (FROM v)",
        ),
        ("a.sql", "CREATE VIEW v AS (FROM s.host_t)"),
    ];
    let graph = graph(&scripts);
    assert!(graph.errors().is_empty(), "{:?}", graph.errors());
    assert_eq!(graph.order(), ["a.sql", "c.sql"]);
    assert!(graph.dependencies("a.sql").is_empty());
    let reads = &graph.names("a.sql").unwrap().reads;
    assert_eq!(reads.len(), 1);
    assert_eq!(
        (reads[0].names.clone(), reads[0].kind),
        (names("s.host_t"), NameKind::Table)
    );
}

#[test]
fn a_script_reads_no_schema_that_it_creates() {
    let text = "CREATE SCHEMA finance;
CREATE VIEW finance.totals AS (FROM finance.orders)";
    let names = script_names(&parse_data(text).syntax());
    let reads: Vec<(String, NameKind)> = (names.reads.iter())
        .map(|read| (read.names.join("."), read.kind))
        .collect();
    assert_eq!(reads, [("finance.orders".to_string(), NameKind::Table)]);
}

#[test]
fn a_table_and_a_schema_of_one_name_are_different_names() {
    let scripts = [
        ("a.sql", "CREATE SCHEMA finance"),
        ("b.sql", "CREATE VIEW finance AS (FROM host)"),
        ("c.sql", "CREATE VIEW c AS (FROM finance)"),
    ];
    let graph = graph(&scripts);
    assert!(graph.errors().is_empty(), "{:?}", graph.errors());
    assert_eq!(graph.dependencies("c.sql"), ["b.sql"]);
}

#[test]
fn a_read_of_a_with_name_of_its_statement_adds_no_dependency() {
    const TOTALS: &str = "CREATE VIEW totals AS (
  FROM host
  |> WITH Sales AS (FROM host |> WHERE amount > 0)
  |> JOIN sales USING (region)
)";
    // `sales.sql` reads `totals`, and the `sales` of `totals.sql` is its `WITH` query, not the
    // table of `sales.sql`, so the scripts are in no cycle.
    let scripts = [
        ("sales.sql", "CREATE VIEW sales AS (FROM totals)"),
        ("totals.sql", TOTALS),
    ];
    let with = graph(&scripts);
    assert!(with.errors().is_empty(), "{:?}", with.errors());
    assert!(with.dependencies("totals.sql").is_empty());
    assert_eq!(with.dependencies("sales.sql"), ["totals.sql"]);
    assert_eq!(with.order(), ["totals.sql", "sales.sql"]);
    assert!(with
        .names("totals.sql")
        .unwrap()
        .reads
        .iter()
        .all(|read| read.names != names("sales")));

    // A `WITH` name is in the scope of its query only: a later statement's `sales` is the table
    // of `sales.sql`, which makes the cycle.
    let text = format!("{TOTALS};\nCREATE VIEW regions AS (FROM sales)");
    let scripts = [scripts[0], ("totals.sql", text.as_str())];
    let outside = graph(&scripts);
    let [GraphError::Cycle { reads }] = outside.errors() else {
        panic!("{:?}", outside.errors());
    };
    let read = &reads[1];
    assert_eq!(read.script, "totals.sql");
    assert_eq!(
        usize::from(read.read.range.start()),
        text.rfind("sales").unwrap()
    );
}

/// The tables that the analysis of the data script `text` reads from outside it, and the tables
/// that [`script_names`] reads from it, each with the text of its range: the two are the same.
fn table_reads(text: &str) -> Vec<(Vec<String>, String)> {
    let catalog = MemoryCatalog::new()
        .with_schema(&["host"], sales())
        .with_schema(&["sales"], sales())
        .with_schema(&["totals"], sales());
    let analysis = analyze(text, &catalog);
    let analyzed: Vec<(Vec<String>, String)> = (analysis.reads.iter())
        .map(|read| (read.names.clone(), text[read.range].to_string()))
        .collect();
    let names = script_names(&parse_data(text).syntax());
    let read: Vec<(Vec<String>, String)> = (names.reads.iter())
        .filter(|read| read.kind == NameKind::Table)
        .map(|read| (read.names.clone(), text[read.range].to_string()))
        .collect();
    assert_eq!(read, analyzed, "{text}");
    read
}

/// The offsets in `text` of each match of `name` as a word.
fn offsets(text: &str, name: &str) -> Vec<usize> {
    (text.match_indices(name).map(|(at, _)| at))
        .filter(|&at| {
            let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
            !text[..at].ends_with(word) && !text[at + name.len()..].starts_with(word)
        })
        .collect()
}

/// The offsets of the reads of the script at `path` of `graph`.
fn read_offsets(graph: &Graph, path: &str) -> Vec<usize> {
    (graph.names(path).unwrap().reads.iter())
        .map(|read| usize::from(read.range.start()))
        .collect()
}

#[test]
fn a_read_in_the_statement_that_defines_its_name_reads_another_script() {
    const CLEAN: &str = "CREATE TEMP VIEW sales AS (FROM sales |> WHERE amount > 0);
CREATE VIEW clean AS (FROM sales)";
    let scripts = [
        ("sales.sql", "CREATE VIEW sales AS (FROM host)"),
        ("clean.sql", CLEAN),
    ];
    let clean = graph(&scripts);
    assert!(clean.errors().is_empty(), "{:?}", clean.errors());
    assert_eq!(clean.dependencies("clean.sql"), ["sales.sql"]);
    assert_eq!(clean.order(), ["sales.sql", "clean.sql"]);
    // The read in the first statement, and not the read of the TEMP view after it.
    assert_eq!(
        read_offsets(&clean, "clean.sql"),
        [offsets(CLEAN, "sales")[1]]
    );
    assert_eq!(table_reads(CLEAN), [(names("sales"), "sales".to_string())]);

    // A read of a name that a later statement defines is an error of the analysis, and not a
    // read of another script.
    let text = "CREATE VIEW totals AS (FROM sales);
CREATE TEMP VIEW sales AS (FROM host)";
    let scripts = [
        ("sales.sql", "CREATE VIEW sales AS (FROM host)"),
        ("totals.sql", text),
    ];
    let totals = graph(&scripts);
    assert!(totals.dependencies("totals.sql").is_empty());
    assert_eq!(table_reads(text), [(names("host"), "host".to_string())]);
}

#[test]
fn a_with_name_hides_the_reads_after_it_in_its_query() {
    // The input of the `WITH` and the query of its name read the table `sales`, and the `JOIN`
    // after it reads the `WITH` query.
    const BEFORE: &str = "CREATE VIEW v AS (
  FROM sales
  |> WITH sales AS (FROM sales |> WHERE amount > 0)
  |> JOIN sales USING (region)
)";
    let z = ("z.sql", "CREATE VIEW sales AS (FROM host)");
    let before = graph(&[("a.sql", BEFORE), z]);
    assert_eq!(before.dependencies("a.sql"), ["z.sql"]);
    // The second `sales` names the `WITH` query, and the fourth reads it.
    let sales = offsets(BEFORE, "sales");
    assert_eq!(read_offsets(&before, "a.sql"), [sales[0], sales[2]]);
    assert_eq!(table_reads(BEFORE).len(), 2);

    // A later query of the `WITH` reads an earlier name of it.
    const LATER: &str = "CREATE VIEW v AS (
  FROM host
  |> WITH sales AS (FROM host), recent AS (FROM sales |> WHERE month > 6)
  |> JOIN recent USING (region)
)";
    let later = graph(&[("a.sql", LATER), z]);
    assert!(later.dependencies("a.sql").is_empty());
    assert_eq!(table_reads(LATER).len(), 2);

    // A read outside the subquery that holds the `WITH` reads the table.
    const OUTSIDE: &str = "CREATE VIEW v AS (
  FROM (FROM host |> WITH sales AS (FROM host) |> JOIN sales USING (region))
  |> JOIN sales USING (region)
)";
    let outside = graph(&[("a.sql", OUTSIDE), z]);
    assert_eq!(outside.dependencies("a.sql"), ["z.sql"]);
    let last = *offsets(OUTSIDE, "sales").last().unwrap();
    assert_eq!(read_offsets(&outside, "a.sql").last(), Some(&last));
    assert_eq!(
        table_reads(OUTSIDE),
        [
            (names("host"), "host".to_string()),
            (names("host"), "host".to_string()),
            (names("sales"), "sales".to_string()),
        ]
    );
}

#[test]
fn a_read_of_a_host_table_adds_no_dependency() {
    let scripts = [
        (
            "a.sql",
            "CREATE VIEW big AS (FROM penguins |> WHERE mass > 4000)",
        ),
        ("b.sql", "CREATE VIEW b AS (FROM other.penguins)"),
    ];
    let graph = graph(&scripts);
    assert!(graph.errors().is_empty());
    assert!(graph.edges().is_empty());
    assert_eq!(graph.order(), ["a.sql", "b.sql"]);
    // The read is recorded, and matches no script.
    let reads = &graph.names("a.sql").unwrap().reads;
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].names, names("penguins"));
}

#[test]
fn downstream_scripts_are_transitive_and_in_the_order_of_the_graph() {
    let scripts = [
        ("base.sql", "CREATE VIEW base AS (FROM host)"),
        ("mid.sql", "CREATE VIEW mid AS (FROM base)"),
        ("aside.sql", "CREATE VIEW aside AS (FROM base)"),
        (
            "top.sql",
            "CREATE VIEW top AS (FROM mid |> JOIN aside USING (k))",
        ),
        ("alone.sql", "CREATE VIEW alone AS (FROM host)"),
    ];
    let graph = graph(&scripts);
    assert_eq!(
        graph.order(),
        ["alone.sql", "base.sql", "aside.sql", "mid.sql", "top.sql"]
    );
    assert_eq!(
        graph.downstream("base.sql"),
        ["aside.sql", "mid.sql", "top.sql"]
    );
    assert_eq!(graph.downstream("mid.sql"), ["top.sql"]);
    assert_eq!(graph.dependencies("top.sql"), ["aside.sql", "mid.sql"]);
    assert!(graph.downstream("missing.sql").is_empty());
}

#[test]
fn the_names_of_a_script_are_the_names_that_its_analysis_exports() {
    let texts = [
        // An append defines nothing, and a later replacement, TEMP too, hides an earlier name.
        "LOAD DATA INTO sales FROM FILES (format = 'CSV', uris = ['a.csv']);
LOAD DATA INTO sales FROM FILES (format = 'CSV', uris = ['b.csv']);
CREATE VIEW v AS (FROM sales);
CREATE OR REPLACE TEMP VIEW v AS (FROM sales |> WHERE month > 1);
CREATE VIEW w AS (FROM v)",
        "CREATE SCHEMA finance;
CREATE TABLE finance.big AS (FROM t |> WHERE amount > 10);
CREATE VIEW IF NOT EXISTS finance.big AS (FROM t);
CREATE OR REPLACE VIEW w AS (FROM t);
CREATE OR REPLACE VIEW w AS (FROM finance.big)",
    ];
    for text in texts {
        let catalog = with_files(text, MemoryCatalog::new().with_schema(&["t"], sales()));
        let analysis = analyze(text, &catalog);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        let exported: Vec<(Vec<String>, usize)> = (analysis.exported())
            .map(|definition| {
                (
                    definition.names.clone(),
                    definition.name_range.start().into(),
                )
            })
            .collect();
        let names = script_names(&parse_data(text).syntax());
        let defined: Vec<(Vec<String>, usize)> = (names.definitions.iter())
            .map(|name| (name.names.clone(), name.range.start().into()))
            .collect();
        assert_eq!(defined, exported, "{text}");
    }
}

const FINANCE: &str = "CREATE SCHEMA finance OPTIONS (description = 'The books');
CREATE EXTERNAL TABLE finance.orders
OPTIONS (format = 'CSV', uris = ['data/files/orders.csv'], description = 'The orders');
CREATE TEMP VIEW scratch AS (FROM finance.orders)";

/// The project catalog of the analysis of [`FINANCE`] at `data/finance.sql`, over a base of the
/// table `penguins` and a table `orders`.
fn project_catalog() -> ProjectCatalog {
    let analysis = analyze(FINANCE, &with_files(FINANCE, MemoryCatalog::new()));
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let penguins = Arc::new(Schema::new(vec![Field::new(
        "species",
        DataType::Utf8,
        true,
    )]));
    let base = MemoryCatalog::new()
        .with_schema(&["penguins"], penguins)
        .with_schema(&["finance", "base_only"], sales());
    ProjectCatalog::new(Arc::new(base))
        .with_analysis("data/finance.sql", &analysis)
        .with_module(
            names("units"),
            "MODULE units; CREATE PUBLIC FUNCTION twice(x INT64) AS (x * 2);".to_string(),
        )
}

#[test]
fn the_project_catalog_holds_the_tables_of_the_scripts_with_their_places() {
    let catalog = project_catalog();
    let orders = catalog.table(&names("FINANCE.Orders")).unwrap();
    assert_eq!(orders.schema(), sales());
    let definition = catalog.definition(&names("finance.orders")).unwrap();
    assert_eq!(definition.file, "data/finance.sql");
    assert!(FINANCE[definition.range].starts_with("CREATE EXTERNAL TABLE finance.orders"));
    assert_eq!(definition.description.as_deref(), Some("The orders"));

    // The schema of a script, with its place.
    assert!(catalog.has_schema(&names("finance")));
    assert!(!catalog.has_schema(&names("books")));
    let schema = catalog.definition(&names("finance")).unwrap();
    assert_eq!(
        &FINANCE[schema.range],
        "CREATE SCHEMA finance OPTIONS (description = 'The books')"
    );
    assert_eq!(schema.description.as_deref(), Some("The books"));

    // A TEMP view stays in its script.
    assert!(catalog.table(&names("scratch")).is_none());
    assert!(catalog.definition(&names("scratch")).is_none());

    // The base's tables, after the scripts'.
    assert!(catalog.table(&names("penguins")).is_some());
    assert!(catalog.table(&names("finance.base_only")).is_some());
    assert_eq!(catalog.definition(&names("penguins")), None);
    let tables: Vec<String> = (catalog.tables().iter())
        .map(|table| table.names.join("."))
        .collect();
    assert_eq!(tables, ["finance.orders", "penguins", "finance.base_only"]);

    // The modules, and the type options of the base.
    let module = catalog.module(&names("UNITS")).unwrap();
    assert!(module.starts_with("MODULE units;"));
    assert!(catalog.module(&names("other")).is_none());
    assert_eq!(catalog.type_options(), TypeOptions::default());
}

#[test]
fn the_tables_of_the_scripts_hide_the_tables_of_the_base() {
    let base = MemoryCatalog::new().with_schema(
        &["orders"],
        Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)])),
    );
    let text = "CREATE EXTERNAL TABLE orders OPTIONS (format = 'CSV', uris = ['o.csv'])";
    let analysis = analyze(text, &with_files(text, MemoryCatalog::new()));
    let catalog = ProjectCatalog::new(Arc::new(base)).with_analysis("data/orders.sql", &analysis);
    assert_eq!(catalog.table(&names("orders")).unwrap().schema(), sales());
    assert_eq!(catalog.tables().len(), 1);

    // A caller's provider takes the place of the empty table of the schema.
    let definition = &analysis.definitions[0];
    let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Utf8, true)]));
    let catalog = ProjectCatalog::new(Arc::new(MemoryCatalog::new())).with_definition(
        "data/orders.sql",
        definition,
        Some(Arc::new(EmptyTable::new(schema.clone()))),
    );
    assert_eq!(catalog.table(&names("orders")).unwrap().schema(), schema);
}

#[test]
fn a_definition_without_a_table_hides_the_table_of_the_base() {
    let base = MemoryCatalog::new()
        .with_schema(
            &["orders"],
            Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)])),
        )
        .with_schema(&["penguins"], sales());
    // The catalog of the analysis reads no files, so the table has no schema.
    let text = "CREATE EXTERNAL TABLE orders OPTIONS (format = 'CSV', uris = ['o.csv'])";
    let analysis = analyze(text, &MemoryCatalog::new());
    assert_eq!(analysis.definitions[0].schema, None);
    let catalog = ProjectCatalog::new(Arc::new(base)).with_analysis("data/orders.sql", &analysis);
    assert!(catalog.table(&names("ORDERS")).is_none());
    assert_eq!(
        catalog.definition(&names("orders")).unwrap().file,
        "data/orders.sql"
    );
    let tables: Vec<String> = (catalog.tables().iter())
        .map(|table| table.names.join("."))
        .collect();
    assert_eq!(tables, ["penguins"]);

    // A read of it finds no table.
    let reader = analyze("CREATE VIEW v AS (FROM orders)", &catalog);
    let errors: Vec<&str> = (reader.diagnostics.iter())
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(errors, ["Table not found: orders"]);
}

#[test]
fn a_load_into_a_table_of_another_script_is_a_duplicate_in_either_order() {
    const CREATE: &str = "CREATE EXTERNAL TABLE orders OPTIONS (format = 'CSV', uris = ['o.csv'])";
    const LOAD: &str = "LOAD DATA INTO orders FROM FILES (format = 'CSV', uris = ['more.csv'])";
    for (first, second) in [(CREATE, LOAD), (LOAD, CREATE)] {
        let scripts = [("a.sql", first), ("b.sql", second)];
        let graph = graph(&scripts);
        let [GraphError::Duplicate {
            kind,
            names: duplicate,
            first,
            second,
        }] = graph.errors()
        else {
            panic!("{:?}", graph.errors());
        };
        assert_eq!(
            (
                *kind,
                duplicate.clone(),
                first.path.as_str(),
                second.path.as_str()
            ),
            (NameKind::Table, names("orders"), "a.sql", "b.sql")
        );
        assert!(graph.edges().is_empty());
        assert_eq!(graph.order(), ["a.sql", "b.sql"]);
    }
}

#[test]
fn the_project_catalog_gives_the_tables_of_files() {
    let text = "LOAD DATA OVERWRITE regions FROM FILES (format = 'CSV', uris = ['r.csv']);
CREATE EXTERNAL TABLE broken OPTIONS (format = 'CSV', uris = ['missing.csv'])";
    let reads = file_reads(&parse_data(text).syntax(), TypeOptions::default());
    let catalog = ProjectCatalog::new(Arc::new(MemoryCatalog::new()))
        .with_files(
            reads[0].files.clone(),
            Ok(Arc::new(EmptyTable::new(sales()))),
        )
        .with_files(reads[1].files.clone(), Err("No file matches".to_string()));
    let table = catalog.files(&reads[0].files).unwrap().unwrap();
    assert_eq!(table.schema(), sales());
    assert_eq!(
        catalog.files(&reads[1].files).unwrap().unwrap_err(),
        "No file matches"
    );

    // The analysis reads the tables of its files from the catalog, and reports their errors.
    let analysis = analyze(text, &catalog);
    assert_eq!(analysis.definitions[0].schema, Some(sales()));
    let errors: Vec<(&str, &str)> = (analysis.diagnostics.iter())
        .map(|d| (&text[d.range], d.message.as_str()))
        .collect();
    assert_eq!(errors, [("['missing.csv']", "No file matches")]);
}

#[test]
fn the_analysis_of_a_script_reads_the_tables_of_another_through_the_project_catalog() {
    let catalog = project_catalog();
    let text = "IMPORT MODULE units;
CREATE VIEW finance.big_orders OPTIONS (description = 'Large orders') AS (
  FROM finance.orders
  |> WHERE amount > 100
  |> SELECT region, units.twice(month) AS months
)";
    let analysis = analyze(text, &catalog);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let reads: Vec<String> = analysis.reads.iter().map(|r| r.names.join(".")).collect();
    assert_eq!(reads, ["finance.orders"]);

    // The project's catalog then holds both scripts' tables.
    let catalog = catalog.with_analysis("data/big.sql", &analysis);
    let big = catalog.table(&names("finance.big_orders")).unwrap();
    let columns: Vec<String> = (big.schema().fields().iter())
        .map(|field| field.name().clone())
        .collect();
    assert_eq!(columns, ["region", "months"]);
    assert_eq!(
        catalog
            .definition(&names("finance.big_orders"))
            .unwrap()
            .file,
        "data/big.sql"
    );

    // Without the project catalog, the schema and the table are missing.
    let alone = analyze(text, &MemoryCatalog::new());
    assert!(!alone.diagnostics.is_empty());
}

#[test]
fn a_table_of_one_name_is_in_the_default_schema() {
    // A read of `public.orders` reads `orders`, which the default schema holds.
    let scripts = [
        ("orders.sql", "CREATE VIEW orders AS (FROM host)"),
        ("totals.sql", "CREATE VIEW totals AS (FROM PUBLIC.orders)"),
    ];
    let read = graph(&scripts);
    assert!(read.errors().is_empty(), "{:?}", read.errors());
    assert_eq!(read.order(), ["orders.sql", "totals.sql"]);
    let edge = &read.edges()[0];
    assert_eq!(
        (edge.script.as_str(), edge.dependency.as_str()),
        ("totals.sql", "orders.sql")
    );
    assert_eq!(edge.read.names, names("PUBLIC.orders"));

    // `orders` and `public.orders` are one name, which two scripts define.
    let scripts = [
        ("a.sql", "CREATE VIEW orders AS (FROM host)"),
        ("b.sql", "CREATE VIEW public.orders AS (FROM host)"),
    ];
    let duplicate = graph(&scripts);
    let [GraphError::Duplicate { first, second, .. }] = duplicate.errors() else {
        panic!("{:?}", duplicate.errors());
    };
    assert_eq!(
        (first.path.as_str(), second.path.as_str()),
        ("a.sql", "b.sql")
    );

    // Another schema holds another table.
    let scripts = [
        ("a.sql", "CREATE VIEW orders AS (FROM host)"),
        (
            "b.sql",
            "CREATE VIEW orders_by_day AS (FROM finance.orders)",
        ),
    ];
    assert!(graph(&scripts).edges().is_empty());
}

#[test]
fn the_scripts_that_import_a_module_directly_or_through_modules() {
    let modules = [
        ("units", "MODULE units; CREATE PUBLIC FUNCTION twice(x INT64) AS (x * 2);"),
        (
            "weather",
            "MODULE weather; IMPORT MODULE units; CREATE PUBLIC FUNCTION warm(t FLOAT64) AS (t > 20);",
        ),
        ("other", "MODULE other; CREATE PUBLIC FUNCTION one() AS (1);"),
    ];
    let scripts = [
        (
            "direct.sql",
            "IMPORT MODULE units; CREATE VIEW direct AS (FROM host)",
        ),
        (
            "through.sql",
            "IMPORT MODULE Weather; CREATE VIEW through AS (FROM host)",
        ),
        ("none.sql", "CREATE VIEW none AS (FROM host)"),
        (
            "missing.sql",
            "IMPORT MODULE missing; CREATE VIEW missing AS (FROM host)",
        ),
    ];
    let graph = graph_with_modules(&scripts, &modules);
    assert_eq!(
        graph.importers(&names("units")),
        ["direct.sql", "through.sql"]
    );
    assert_eq!(graph.importers(&names("WEATHER")), ["through.sql"]);
    assert!(graph.importers(&names("other")).is_empty());
    assert!(graph.importers(&names("missing")).is_empty());
    assert_eq!(graph.imports("direct.sql"), [names("units")]);
    let mut imports = graph.imports("through.sql").to_vec();
    imports.sort();
    assert_eq!(imports, [names("Weather"), names("units")]);
    assert!(graph.imports("none.sql").is_empty());
    // A module adds no dependency between scripts.
    assert!(graph.edges().is_empty());
    assert_eq!(graph.order().len(), 4);
}

#[test]
fn a_table_that_an_imported_module_reads_is_a_read_at_the_import() {
    let scripts = [
        (
            "a_reader.sql",
            "IMPORT MODULE m;\nCREATE TABLE z AS (FROM m.f());",
        ),
        ("z_definer.sql", "CREATE TABLE t AS (FROM sales);"),
    ];
    let modules = [
        (
            "m",
            "MODULE m;\nIMPORT MODULE n;\nCREATE PUBLIC FUNCTION g() AS (1);",
        ),
        (
            "n",
            "MODULE n;\nCREATE PUBLIC TABLE FUNCTION f() AS (FROM t);",
        ),
    ];
    let graph = graph_with_modules(&scripts, &modules);
    assert_eq!(graph.order(), ["z_definer.sql", "a_reader.sql"]);
    let edges = graph.edges();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].read.names, ["t"]);
    // The read is at the import, which reads `n` through `m`.
    let reader = text(&scripts, "a_reader.sql");
    assert_eq!(
        &reader[std::ops::Range::<usize>::from(edges[0].read.range)],
        "m"
    );
}
