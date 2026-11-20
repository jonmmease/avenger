//! Lowering data scripts to DataFusion: the tables of files of each format and layout, which
//! `file_table` builds over an `InMemory` object store, and the tables, views and schemas that
//! `lower` registers, which scripts then read with `avenger_sql::sql`.

use std::sync::{Arc, OnceLock};

use arrow::{
    array::{Float64Array, Int64Array, RecordBatch, StringArray},
    datatypes::{DataType, Field, Schema},
    util::pretty::pretty_format_batches,
};
use avenger_sql::{
    analysis::Options,
    analyze_data,
    catalog::{references, Catalog, SessionCatalog},
    data::{
        file_reads, file_table, lower, DataAnalysis, Files, LoweringError, Registered, Registration,
    },
    library::Library,
    project::ProjectCatalog,
};
use avenger_sql_syntax::parse_data;
use datafusion::{
    catalog::{SchemaProvider, TableProvider},
    common::TableReference,
    datasource::MemTable,
    execution::object_store::ObjectStoreUrl,
    object_store::{memory::InMemory, path::Path, ObjectStoreExt, PutPayload},
    prelude::SessionContext,
};

/// The URL of the project's root, at the object store of each test's files.
const BASE: &str = "memory://project/";

const SALES_CSV: &[u8] = include_bytes!("data/files/sales.csv");
const CARS_JSON: &[u8] = include_bytes!("data/files/cars.json");
const CARS_LINES_JSON: &[u8] = include_bytes!("data/files/cars_lines.json");

fn library() -> &'static Library {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY.get_or_init(Library::new)
}

/// A project: a session that reads the files of an `InMemory` store at [`BASE`].
struct Project {
    ctx: SessionContext,
    store: Arc<InMemory>,
}

impl Project {
    /// A project of the files `files`, each a path relative to the root and its bytes.
    async fn new(files: &[(&str, &[u8])]) -> Project {
        Project::with_session(SessionContext::new(), files).await
    }

    /// A project of the files `files` in the session `ctx`.
    async fn with_session(ctx: SessionContext, files: &[(&str, &[u8])]) -> Project {
        avenger_sql::register(&ctx).unwrap();
        let store = Arc::new(InMemory::new());
        let url = ObjectStoreUrl::parse("memory://project").unwrap();
        ctx.register_object_store(url.as_ref(), store.clone());
        let project = Project { ctx, store };
        for (path, bytes) in files {
            project.put(path, bytes).await;
        }
        project
    }

    async fn put(&self, path: &str, bytes: &[u8]) {
        let payload = PutPayload::from(bytes.to_vec());
        self.store
            .put(&Path::parse(path).unwrap(), payload)
            .await
            .unwrap();
    }

    async fn delete(&self, path: &str) {
        self.store
            .delete(&Path::parse(path).unwrap())
            .await
            .unwrap();
    }

    /// The analysis of the data script `text`, whose catalog holds the tables of the session
    /// that the script reads, and the table of each of its file reads, or the error of reading
    /// its files.
    async fn analyze(&self, text: &str) -> (DataAnalysis, ProjectCatalog) {
        let parse = parse_data(text);
        assert!(parse.ok(), "{:?}", parse.diagnostics());
        let root = parse.syntax();
        let base = SessionCatalog::load(&self.ctx, &references(&root))
            .await
            .unwrap();
        let types = base.type_options();
        let mut catalog = ProjectCatalog::new(Arc::new(base));
        let state = self.ctx.state();
        for read in file_reads(&root, types) {
            let table = file_table(&state, &read.files, BASE).await;
            catalog = catalog.with_files(read.files, table);
        }
        let analysis = analyze_data(&root, &catalog, library(), &Options::default());
        (analysis, catalog)
    }

    /// Analyzes and lowers the data script `text`, which has no errors.
    async fn load(&self, text: &str) -> Loaded {
        let (analysis, catalog) = self.analyze(text).await;
        let messages: Vec<&str> = (analysis.diagnostics.iter())
            .map(|diagnostic| diagnostic.message.as_str())
            .collect();
        assert!(messages.is_empty(), "{messages:?}");
        let registered = lower(&self.ctx, &analysis, &catalog).await.unwrap();
        Loaded {
            analysis,
            registered,
        }
    }

    /// The messages of the errors of the analysis of `text`.
    async fn errors(&self, text: &str) -> Vec<String> {
        let (analysis, _) = self.analyze(text).await;
        (analysis.diagnostics.into_iter())
            .map(|diagnostic| diagnostic.message)
            .collect()
    }

    /// The rows of the script `query`, as a table.
    async fn rows(&self, query: &str) -> String {
        let frame = avenger_sql::sql(&self.ctx, query).await.unwrap();
        let batches = frame.collect().await.unwrap();
        pretty_format_batches(&batches).unwrap().to_string()
    }

    /// The names and the types of the columns of the table `name` of the session.
    async fn columns(&self, name: &str) -> Vec<(String, DataType)> {
        let table = self.ctx.table_provider(name).await.unwrap();
        (table.schema().fields().iter())
            .map(|field| (field.name().clone(), field.data_type().clone()))
            .collect()
    }
}

/// `text` without the white space at the start of its lines, as a table of rows prints.
fn table(text: &str) -> String {
    (text.trim().lines())
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
}

fn columns(columns: &[(&str, DataType)]) -> Vec<(String, DataType)> {
    (columns.iter())
        .map(|(name, data_type)| (name.to_string(), data_type.clone()))
        .collect()
}

/// What the lowering of a script registered, with the analysis whose definitions name it.
struct Loaded {
    analysis: DataAnalysis,
    registered: Vec<Registered>,
}

/// The names of the definitions that `loaded` registered, each as its path.
fn names(loaded: &Loaded) -> Vec<String> {
    (loaded.registered.iter())
        .map(|registered| {
            loaded.analysis.definitions[registered.definition]
                .names
                .join(".")
        })
        .collect()
}

/// The sales of [`SALES_CSV`] as a batch, for the formats that a test writes.
fn sales_batch() -> RecordBatch {
    let schema = Schema::new(vec![
        Field::new("region", DataType::Utf8, true),
        Field::new("month", DataType::Int64, true),
        Field::new("amount", DataType::Float64, true),
    ]);
    RecordBatch::try_new(
        Arc::new(schema),
        vec![
            Arc::new(StringArray::from(vec!["east", "west", "east"])),
            Arc::new(Int64Array::from(vec![1, 1, 2])),
            Arc::new(Float64Array::from(vec![10.5, 20.0, 7.25])),
        ],
    )
    .unwrap()
}

const SALES_ROWS: &str = "
    +--------+-------+--------+
    | region | month | amount |
    +--------+-------+--------+
    | east   | 1     | 10.5   |
    | west   | 1     | 20.0   |
    | east   | 2     | 7.25   |
    +--------+-------+--------+";

const SALES_COLUMNS: [(&str, DataType); 3] = [
    ("region", DataType::Utf8),
    ("month", DataType::Int64),
    ("amount", DataType::Float64),
];

#[tokio::test]
async fn csv_with_a_header() {
    let project = Project::new(&[("data/sales.csv", SALES_CSV)]).await;
    let registered = project
        .load("CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/sales.csv']);")
        .await;
    assert_eq!(names(&registered), ["sales"]);
    assert!(matches!(
        registered.registered[0].registration,
        Registration::Table(_)
    ));
    assert_eq!(project.columns("sales").await, columns(&SALES_COLUMNS));
    assert_eq!(
        project.rows("FROM sales |> ORDER BY month, region").await,
        table(SALES_ROWS)
    );
}

#[tokio::test]
async fn csv_with_a_column_list() {
    let rows = b"east;1;\"10,5\"\nwest;1;20\n";
    let project = Project::new(&[("sales.csv", rows), ("header.csv", SALES_CSV)]).await;
    project
        .load(
            "CREATE EXTERNAL TABLE sales (region STRING, month INT64, amount STRING)
            OPTIONS (format = 'CSV', uris = ['sales.csv'], field_delimiter = ';');

            CREATE EXTERNAL TABLE skipped (place STRING, month INT64, total FLOAT64)
            OPTIONS (format = 'CSV', uris = ['header.csv'], skip_leading_rows = 1);",
        )
        .await;
    assert_eq!(
        project.rows("FROM sales |> ORDER BY region").await,
        table(
            "
            +--------+-------+--------+
            | region | month | amount |
            +--------+-------+--------+
            | east   | 1     | 10,5   |
            | west   | 1     | 20     |
            +--------+-------+--------+"
        )
    );
    // The header row is skipped, and the column list names the columns.
    assert_eq!(
        project
            .rows("FROM skipped |> AGGREGATE count(*) AS n, sum(total) AS total")
            .await,
        table(
            "
            +---+-------+
            | n | total |
            +---+-------+
            | 3 | 37.75 |
            +---+-------+"
        )
    );
}

#[tokio::test]
async fn csv_with_quoted_newlines() {
    let rows = b"name,note\na,\"one\ntwo\"\nb,three\n";
    let project = Project::new(&[("notes.csv", rows)]).await;
    project
        .load(
            "CREATE EXTERNAL TABLE notes
            OPTIONS (format = 'CSV', uris = ['notes.csv'], allow_quoted_newlines = TRUE);",
        )
        .await;
    assert_eq!(
        project
            .rows("FROM notes |> SELECT name, length(note) AS n |> ORDER BY name")
            .await,
        table(
            "
            +------+---+
            | name | n |
            +------+---+
            | a    | 7 |
            | b    | 5 |
            +------+---+"
        )
    );
}

#[tokio::test]
async fn json_arrays_and_lines() {
    let project = Project::new(&[
        ("cars.json", CARS_JSON),
        ("cars_lines.json", CARS_LINES_JSON),
    ])
    .await;
    project
        .load(
            "CREATE EXTERNAL TABLE array_cars OPTIONS (format = 'JSON', uris = ['cars.json']);
            CREATE EXTERNAL TABLE line_cars
            OPTIONS (format = 'NEWLINE_DELIMITED_JSON', uris = ['cars_lines.json']);",
        )
        .await;
    assert_eq!(
        project
            .rows(
                "FROM array_cars |> SELECT name, mpg |> UNION ALL (FROM line_cars |> SELECT name, mpg) |> ORDER BY name"
            )
            .await,
        table(
            "
            +----------+-----+
            | name     | mpg |
            +----------+-----+
            | chevelle | 18  |
            | rebel    | 16  |
            | skylark  | 15  |
            | torino   | 17  |
            +----------+-----+"
        )
    );
}

#[tokio::test]
async fn arrow_ipc_files() {
    use arrow::ipc::{
        writer::{FileWriter, IpcWriteOptions, StreamWriter},
        MetadataVersion,
    };
    let batch = sales_batch();
    // DataFusion 55 reads the schema of an IPC file outside the local file system only when its
    // messages align to 8 bytes, as pyarrow writes them, and not to arrow-rs's default of 64.
    let options = IpcWriteOptions::try_new(8, false, MetadataVersion::V5).unwrap();
    let mut file = Vec::new();
    let mut writer = FileWriter::try_new_with_options(&mut file, &batch.schema(), options).unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
    drop(writer);
    let mut stream = Vec::new();
    let mut writer = StreamWriter::try_new(&mut stream, &batch.schema()).unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
    drop(writer);
    let project = Project::new(&[("sales.arrow", &file), ("stream.arrow", &stream)]).await;
    project
        .load(
            "CREATE EXTERNAL TABLE sales OPTIONS (format = 'ARROW', uris = ['sales.arrow']);
            CREATE EXTERNAL TABLE stream OPTIONS (format = 'ARROW', uris = ['stream.arrow']);",
        )
        .await;
    for name in ["sales", "stream"] {
        assert_eq!(project.columns(name).await, columns(&SALES_COLUMNS));
        let rows = project
            .rows(&format!("FROM {name} |> ORDER BY month, region"))
            .await;
        assert_eq!(rows, table(SALES_ROWS), "{name}");
    }
}

#[cfg(feature = "parquet")]
fn parquet(batch: &RecordBatch) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut writer =
        datafusion::parquet::arrow::ArrowWriter::try_new(&mut bytes, batch.schema(), None).unwrap();
    writer.write(batch).unwrap();
    writer.close().unwrap();
    bytes
}

#[cfg(feature = "parquet")]
#[tokio::test]
async fn parquet_files() {
    let bytes = parquet(&sales_batch());
    let project = Project::new(&[("sales.parquet", &bytes)]).await;
    project
        .load("CREATE EXTERNAL TABLE sales OPTIONS (format = 'PARQUET', uris = ['sales.parquet']);")
        .await;
    assert_eq!(
        project.rows("FROM sales |> ORDER BY month, region").await,
        table(SALES_ROWS)
    );
}

/// An Avro object container file of `rows` of the record `{name: string, mpg: long}`, without
/// compression.
#[cfg(feature = "avro")]
fn avro(rows: &[(&str, i64)]) -> Vec<u8> {
    fn long(value: i64, out: &mut Vec<u8>) {
        let mut zigzag = ((value << 1) ^ (value >> 63)) as u64;
        loop {
            let byte = (zigzag & 0x7f) as u8;
            zigzag >>= 7;
            if zigzag == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }
    fn bytes(value: &[u8], out: &mut Vec<u8>) {
        long(value.len() as i64, out);
        out.extend_from_slice(value);
    }
    let schema = r#"{"type": "record", "name": "car", "fields": [{"name": "name", "type": "string"}, {"name": "mpg", "type": "long"}]}"#;
    let sync = [7u8; 16];
    let mut out = b"Obj\x01".to_vec();
    long(2, &mut out);
    bytes(b"avro.schema", &mut out);
    bytes(schema.as_bytes(), &mut out);
    bytes(b"avro.codec", &mut out);
    bytes(b"null", &mut out);
    long(0, &mut out);
    out.extend_from_slice(&sync);
    let mut block = Vec::new();
    for (name, mpg) in rows {
        bytes(name.as_bytes(), &mut block);
        long(*mpg, &mut block);
    }
    long(rows.len() as i64, &mut out);
    long(block.len() as i64, &mut out);
    out.extend_from_slice(&block);
    out.extend_from_slice(&sync);
    out
}

#[cfg(feature = "avro")]
#[tokio::test]
async fn avro_files() {
    let bytes = avro(&[("chevelle", 18), ("skylark", 15)]);
    let project = Project::new(&[("cars.avro", &bytes)]).await;
    project
        .load("CREATE EXTERNAL TABLE cars OPTIONS (format = 'AVRO', uris = ['cars.avro']);")
        .await;
    assert_eq!(
        project.rows("FROM cars |> ORDER BY name").await,
        table(
            "
            +----------+-----+
            | name     | mpg |
            +----------+-----+
            | chevelle | 18  |
            | skylark  | 15  |
            +----------+-----+"
        )
    );
}

#[cfg(feature = "compression")]
#[tokio::test]
async fn gzip_csv_files() {
    const SALES_GZ: &[u8] = include_bytes!("data/files/sales.csv.gz");
    let project = Project::new(&[
        ("sales.csv.gz", SALES_GZ),
        ("archive/2025.csv.gz", SALES_GZ),
        ("archive/2025.csv", SALES_CSV),
    ])
    .await;
    project
        .load(
            "CREATE EXTERNAL TABLE sales
            OPTIONS (format = 'CSV', uris = ['sales.csv.gz'], compression = 'GZIP');

            CREATE EXTERNAL TABLE archive
            OPTIONS (format = 'CSV', uris = ['archive/'], compression = 'GZIP');",
        )
        .await;
    assert_eq!(
        project.rows("FROM sales |> ORDER BY month, region").await,
        table(SALES_ROWS)
    );
    // The directory keeps the files with the extension of the format and the compression.
    assert_eq!(
        project
            .rows("FROM archive |> AGGREGATE count(*) AS n")
            .await,
        table(
            "
            +---+
            | n |
            +---+
            | 3 |
            +---+"
        )
    );
}

#[cfg(feature = "compression")]
#[tokio::test]
async fn gzip_json_arrays() {
    // The first character of a compressed file is the first of its text.
    const CARS_GZ: &[u8] = include_bytes!("data/files/cars.json.gz");
    let project = Project::new(&[("cars.json.gz", CARS_GZ)]).await;
    project
        .load(
            "CREATE EXTERNAL TABLE cars
            OPTIONS (format = 'JSON', uris = ['cars.json.gz'], compression = 'GZIP');",
        )
        .await;
    assert_eq!(
        project.rows("FROM cars |> AGGREGATE count(*) AS n").await,
        table(
            "
            +---+
            | n |
            +---+
            | 2 |
            +---+"
        )
    );
}

/// The files of a Hive layout of [`SALES_CSV`]'s columns, by region.
fn hive_files() -> Vec<(&'static str, &'static [u8])> {
    vec![
        (
            "sales/region=east/part-0.csv",
            b"month,amount\n1,10.5\n2,7.25\n",
        ),
        ("sales/region=west/part-0.csv", b"month,amount\n1,20.0\n"),
        ("sales/region=west/notes.txt", b"not a CSV file\n"),
    ]
}

#[tokio::test]
async fn hive_partition_keys() {
    let project = Project::new(&hive_files()).await;
    project
        .load(
            "CREATE EXTERNAL TABLE inferred
            WITH PARTITION COLUMNS
            OPTIONS (format = 'CSV', uris = ['sales/'], hive_partition_uri_prefix = 'sales/');

            CREATE EXTERNAL TABLE typed (month INT64, amount FLOAT64)
            WITH PARTITION COLUMNS (region STRING)
            OPTIONS (format = 'CSV', uris = ['sales/*.csv'], hive_partition_uri_prefix = 'sales',
              skip_leading_rows = 1);",
        )
        .await;
    assert_eq!(
        project.columns("inferred").await,
        columns(&[
            ("month", DataType::Int64),
            ("amount", DataType::Float64),
            ("region", DataType::Utf8),
        ])
    );
    for name in ["inferred", "typed"] {
        let rows = project
            .rows(&format!(
                "FROM {name} |> SELECT region, month, amount |> ORDER BY month, region"
            ))
            .await;
        assert_eq!(rows, table(SALES_ROWS), "{name}");
    }
}

#[tokio::test]
async fn equalities_of_partition_keys_read_folders_however_spelled() {
    // A month with a leading zero, and a name whose writer encoded its `:`, which DataFusion
    // would list under a prefix spelled `month=1` or `slot=08:30`.
    let project = Project::new(&[
        ("m/year=2025/month=01/slot=08%3A30/a.csv", b"n\n1\n"),
        ("m/year=2025/month=02/slot=09%3A00/b.csv", b"n\n2\n"),
    ])
    .await;
    project
        .load(
            "CREATE EXTERNAL TABLE inferred WITH PARTITION COLUMNS
            OPTIONS (format = 'CSV', uris = ['m/'], hive_partition_uri_prefix = 'm/');
            CREATE EXTERNAL TABLE listed WITH PARTITION COLUMNS (year INT64, month INT64, slot STRING)
            OPTIONS (format = 'CSV', uris = ['m/'], hive_partition_uri_prefix = 'm/');",
        )
        .await;
    for name in ["inferred", "listed"] {
        for (filter, n) in [
            ("year = 2025 AND month = 1", 1),
            ("2025 = year AND month = 2", 2),
            ("year = 2025 AND month = 1 AND slot = '08:30'", 1),
            ("slot = '09:00'", 2),
            ("year = 2025 AND month = 3", 0),
        ] {
            let rows = project
                .rows(&format!(
                    "FROM {name} |> WHERE {filter} |> AGGREGATE COALESCE(SUM(n), 0) AS n"
                ))
                .await;
            assert!(
                rows.contains(&format!("| {n} |")),
                "{name} [{filter}]: {rows}"
            );
        }
    }
}

#[tokio::test]
async fn hive_partition_keys_take_the_type_of_their_values() {
    // A key of integers, of dates, of dates and times, of a date and an integer, and of an
    // integer and a name, with a time as a writer encodes its `:` and `+`, and as written.
    let project = Project::new(&[
        (
            "events/year=2025/day=2025-01-31/started=2025-01-31 16:00:00+00/mixed=2025-01-31/kind=7/a.csv",
            b"n\n1\n",
        ),
        (
            "events/year=2026/day=2026-02-01/started=2026-02-01%2008%3A30%3A00%2B00/mixed=2026/kind=b/b.csv",
            b"n\n2\n",
        ),
    ])
    .await;
    project
        .load(
            "CREATE EXTERNAL TABLE events
            WITH PARTITION COLUMNS
            OPTIONS (format = 'CSV', uris = ['events/'], hive_partition_uri_prefix = 'events/');",
        )
        .await;
    assert_eq!(
        project.columns("events").await,
        columns(&[
            ("n", DataType::Int64),
            ("year", DataType::Int64),
            ("day", DataType::Date32),
            ("started", avenger_sql::types::timestamp()),
            ("mixed", DataType::Utf8),
            ("kind", DataType::Utf8),
        ])
    );
    let rows = project
        .rows(
            "FROM events
            |> WHERE year = 2026 AND day = DATE '2026-02-01'
              AND started = TIMESTAMP '2026-02-01 08:30:00+00'
            |> SELECT n, year + 1 AS next, mixed, kind",
        )
        .await;
    assert_eq!(
        rows,
        table(
            "
            +---+------+-------+------+
            | n | next | mixed | kind |
            +---+------+-------+------+
            | 2 | 2027 | 2026  | b    |
            +---+------+-------+------+"
        )
    );
}

#[tokio::test]
async fn globs_keep_brackets_literal() {
    let project = Project::new(&[
        ("logs/day[1]-a.csv", b"n\n1\n"),
        ("logs/day[1]-b.csv", b"n\n2\n"),
        // A glob class [1] would match these.
        ("logs/day1-c.csv", b"n\n4\n"),
        ("logs/a[1].csv", b"n\n8\n"),
        ("logs/a1.csv", b"n\n16\n"),
    ])
    .await;
    project
        .load(
            "CREATE EXTERNAL TABLE days OPTIONS (format = 'CSV', uris = ['logs/day[1]-*.csv']);
            CREATE EXTERNAL TABLE one OPTIONS (format = 'CSV', uris = ['logs/a[1].csv']);",
        )
        .await;
    let sum = |name: &str| format!("FROM {name} |> AGGREGATE sum(n) AS n");
    assert!(project.rows(&sum("days")).await.contains("| 3 |"));
    assert!(project.rows(&sum("one")).await.contains("| 8 |"));
}

#[tokio::test]
async fn directories_and_several_uris() {
    let project = Project::new(&[
        ("dir/a.csv", b"n\n1\n"),
        ("dir/b.csv", b"n\n2\n"),
        ("dir/notes.txt", b"n\n4\n"),
        ("dir/sub/c.csv", b"n\n8\n"),
        ("more/d.txt", b"n\n16\n"),
    ])
    .await;
    project
        .load(
            "CREATE EXTERNAL TABLE numbers
            OPTIONS (format = 'CSV', uris = ['dir/', 'more/d.txt']);",
        )
        .await;
    // The directory keeps its CSV files and not those of its subdirectories, and a URI of one
    // file reads it whatever its extension.
    assert!(project
        .rows("FROM numbers |> AGGREGATE sum(n) AS n")
        .await
        .contains("| 19 |"));
}

#[tokio::test]
async fn load_data_into_from_two_file_sets() {
    let project = Project::new(&[
        ("data/sales.csv", SALES_CSV),
        (
            "data/more.json",
            b"[{\"amount\": 3, \"region\": \"north\", \"month\": 3}]",
        ),
    ])
    .await;
    let registered = project
        .load(
            "LOAD DATA INTO sales FROM FILES (format = 'CSV', uris = ['data/sales.csv']);

            CREATE TABLE first_load AS (FROM sales |> AGGREGATE count(*) AS n);
            CREATE VIEW first_view AS (FROM sales |> AGGREGATE count(*) AS n);

            LOAD DATA INTO sales FROM FILES (format = 'JSON', uris = ['data/more.json']);",
        )
        .await;
    assert_eq!(names(&registered), ["sales", "first_load", "first_view"]);
    assert_eq!(project.columns("sales").await, columns(&SALES_COLUMNS));
    assert_eq!(
        project.rows("FROM sales |> ORDER BY month, region").await,
        table(
            "
            +--------+-------+--------+
            | region | month | amount |
            +--------+-------+--------+
            | east   | 1     | 10.5   |
            | west   | 1     | 20.0   |
            | east   | 2     | 7.25   |
            | north  | 3     | 3.0    |
            +--------+-------+--------+"
        )
    );
    // A read between two loads sees the rows of the first, whether it runs once or at each
    // query.
    for name in ["first_load", "first_view"] {
        assert!(
            project
                .rows(&format!("FROM {name}"))
                .await
                .contains("| 3 |"),
            "{name}"
        );
    }
}

#[tokio::test]
async fn views_read_the_rows_of_loads() {
    let project = Project::new(&[("data/sales.csv", SALES_CSV)]).await;
    project
        .load(
            "LOAD DATA OVERWRITE sales FROM FILES (format = 'CSV', uris = ['data/sales.csv']);
            CREATE VIEW east AS (FROM sales |> WHERE region = 'east' |> SELECT month, amount);
            CREATE EXTERNAL TABLE files OPTIONS (format = 'CSV', uris = ['data/sales.csv']);",
        )
        .await;
    project.delete("data/sales.csv").await;
    // The view reads the rows in memory, while the external table reads its files again.
    assert_eq!(
        project.rows("FROM east |> ORDER BY month").await,
        table(
            "
            +-------+--------+
            | month | amount |
            +-------+--------+
            | 1     | 10.5   |
            | 2     | 7.25   |
            +-------+--------+"
        )
    );
    assert!(project
        .rows("FROM files |> AGGREGATE count(*) AS n")
        .await
        .contains("| 0 |"));
}

#[tokio::test]
async fn tables_of_queries() {
    let project = Project::new(&[("data/sales.csv", SALES_CSV)]).await;
    project
        .load(
            "CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/sales.csv']);
            CREATE TABLE totals (region STRING, total FLOAT64) AS (
              FROM sales
              |> WHERE amount > (FROM sales |> AGGREGATE min(amount))
              |> AGGREGATE sum(amount) AS total GROUP BY region
            );",
        )
        .await;
    project
        .put("data/sales.csv", b"region,month,amount\nsouth,1,1\n")
        .await;
    // The table keeps the rows of its query when it ran.
    assert_eq!(
        project.rows("FROM totals |> ORDER BY region").await,
        table(
            "
            +--------+-------+
            | region | total |
            +--------+-------+
            | east   | 10.5  |
            | west   | 20.0  |
            +--------+-------+"
        )
    );
}

#[tokio::test]
async fn schemas_and_qualified_tables() {
    let project = Project::new(&[("data/sales.csv", SALES_CSV)]).await;
    let registered = project
        .load(
            "CREATE SCHEMA finance OPTIONS (description = 'The tables of money');
            LOAD DATA OVERWRITE Finance.sales
            FROM FILES (format = 'CSV', uris = ['data/sales.csv']);
            CREATE VIEW finance.totals AS (FROM finance.sales |> AGGREGATE sum(amount) AS total);",
        )
        .await;
    assert_eq!(
        names(&registered),
        ["finance", "Finance.sales", "finance.totals"]
    );
    assert!(matches!(
        registered.registered[0].registration,
        Registration::Schema(_)
    ));
    assert!(project
        .rows("FROM finance.totals")
        .await
        .contains("| 37.75 |"));
    // A script of the project reads the tables that an earlier script registers.
    project
        .load("CREATE TABLE copied AS (FROM finance.sales |> AGGREGATE count(*) AS n);")
        .await;
    assert!(project.rows("FROM copied").await.contains("| 3 |"));
}

#[tokio::test]
async fn temporary_tables_stay_in_their_script() {
    let project = Project::new(&[("data/sales.csv", SALES_CSV)]).await;
    let registered = project
        .load(
            "CREATE TEMP EXTERNAL TABLE raw_sales
            OPTIONS (format = 'CSV', uris = ['data/sales.csv']);
            CREATE TABLE east AS (FROM raw_sales |> WHERE region = 'east');",
        )
        .await;
    assert_eq!(names(&registered), ["east"]);
    assert!(project.ctx.table_provider("raw_sales").await.is_err());
    assert!(project
        .rows("FROM east |> AGGREGATE count(*) AS n")
        .await
        .contains("| 2 |"));
}

#[tokio::test]
async fn reloads_replace_tables() {
    let project = Project::new(&[("data/sales.csv", SALES_CSV)]).await;
    let script =
        "LOAD DATA OVERWRITE sales FROM FILES (format = 'CSV', uris = ['data/sales.csv']);";
    project.load(script).await;
    project
        .put("data/sales.csv", b"region,month,amount\nsouth,1,1\n")
        .await;
    project.load(script).await;
    assert!(project
        .rows("FROM sales |> AGGREGATE count(*) AS n")
        .await
        .contains("| 1 |"));
}

#[tokio::test]
async fn errors_of_files() {
    let project = Project::new(&[("data/sales.csv", SALES_CSV)]).await;
    let errors = project
        .errors(
            "CREATE EXTERNAL TABLE missing OPTIONS (format = 'CSV', uris = ['data/nothing.csv']);",
        )
        .await;
    assert_eq!(errors, ["No file matches 'data/nothing.csv'"]);
    // A column list infers no schema, and a URI that matches no file is still an error.
    let errors = project
        .errors(
            "LOAD DATA OVERWRITE t (n INT64) FROM FILES (format = 'CSV', uris = ['data/*.json']);",
        )
        .await;
    assert_eq!(errors, ["No file matches 'data/*.json'"]);
}

#[tokio::test]
async fn errors_of_lowering() {
    let project = Project::new(&[("data/sales.csv", b"region,month,amount\neast,1,x\n")]).await;
    let text = "CREATE EXTERNAL TABLE sales (region STRING, month INT64, amount FLOAT64)
        OPTIONS (format = 'CSV', uris = ['data/sales.csv'], skip_leading_rows = 1);
        CREATE TABLE totals AS (FROM sales |> AGGREGATE sum(amount) AS total);
        CREATE VIEW fine AS (FROM sales);";
    let (analysis, catalog) = project.analyze(text).await;
    assert!(analysis.diagnostics.is_empty());
    let errors: Vec<LoweringError> = lower(&project.ctx, &analysis, &catalog).await.unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].definition, 1);
    assert!(errors[0].message.contains("x"), "{}", errors[0].message);
    // A script with an error registers nothing.
    assert!(project.ctx.table_provider("sales").await.is_err());
    assert!(project.ctx.table_provider("fine").await.is_err());
}

#[tokio::test]
async fn uris_resolve_against_the_base() {
    let project = Project::new(&[("data/a b?.csv", SALES_CSV)]).await;
    project
        .load("CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/a b?.csv']);")
        .await;
    assert_eq!(project.columns("sales").await, columns(&SALES_COLUMNS));
    // `\` separates the names of a path, as `/` does, here and before a `*`.
    project
        .load(
            r"CREATE EXTERNAL TABLE windows OPTIONS (format = 'CSV', uris = [r'data\a b?.csv']);
            CREATE EXTERNAL TABLE pattern OPTIONS (format = 'CSV', uris = [r'data\a*.csv']);",
        )
        .await;
    for name in ["windows", "pattern"] {
        assert_eq!(
            project.columns(name).await,
            columns(&SALES_COLUMNS),
            "{name}"
        );
    }
}

#[tokio::test]
async fn absolute_paths_and_file_urls() {
    let dir = std::env::temp_dir().join(format!("avenger-sql-absolute-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sales.csv"), SALES_CSV).unwrap();
    let path = dir.join("sales.csv").display().to_string();
    let url = url_of(&dir.join("sales.csv"));
    let project = Project::new(&[]).await;
    project
        .load(&format!(
            "CREATE EXTERNAL TABLE by_path OPTIONS (format = 'CSV', uris = ['{path}']);
            CREATE EXTERNAL TABLE by_url OPTIONS (format = 'CSV', uris = ['{url}']);"
        ))
        .await;
    for name in ["by_path", "by_url"] {
        let rows = project
            .rows(&format!("FROM {name} |> ORDER BY month, region"))
            .await;
        assert_eq!(rows, table(SALES_ROWS), "{name}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A Windows path is absolute on any machine, and so not a path of a Unix machine, and `\`
/// separates the names of a relative path, as `/` does.
#[cfg(unix)]
#[tokio::test]
async fn uris_read_by_their_kind() {
    let project = Project::new(&[]).await;
    let text = r"CREATE EXTERNAL TABLE windows OPTIONS (format = 'CSV', uris = [r'C:/x.csv']);
        CREATE EXTERNAL TABLE named OPTIONS (format = 'CSV', uris = [r'data\..\..\x.csv']);";
    assert_eq!(
        project.errors(text).await,
        [
            "'C:/x.csv' is not an absolute path of this machine",
            r"'data\..\..\x.csv' goes above the root of the store of the project's files, memory://project/"
        ]
    );
}

/// The `file:` URL of the absolute path `path`.
fn url_of(path: &std::path::Path) -> String {
    format!("file://{}", path.display())
}

#[tokio::test]
async fn parent_directories() {
    let dir = std::env::temp_dir().join(format!("avenger-sql-parent-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("project")).unwrap();
    std::fs::create_dir_all(dir.join("shared")).unwrap();
    std::fs::write(dir.join("shared/sales.csv"), SALES_CSV).unwrap();
    let text = "CREATE EXTERNAL TABLE sales
        OPTIONS (format = 'CSV', uris = ['./../shared/sales.csv']);";
    let files = file_reads(&parse_data(text).syntax(), Default::default())
        .remove(0)
        .files;
    let ctx = SessionContext::new();
    let base = format!("{}/", url_of(&dir.join("project")));
    let table = file_table(&ctx.state(), &files, &base).await.unwrap();
    assert_eq!(table.schema().fields().len(), 3);
    std::fs::remove_dir_all(&dir).unwrap();
    // A path doesn't go above the root of the store.
    let project = Project::new(&[]).await;
    assert_eq!(
        project.errors(text).await,
        ["'./../shared/sales.csv' goes above the root of the store of the project's files, memory://project/"]
    );
}

/// The sum of the column `n` of each of the tables `names`.
async fn sums(project: &Project, names: &[&str]) -> Vec<String> {
    let mut sums = Vec::new();
    for name in names {
        let query = format!("FROM {name} |> AGGREGATE sum(n) AS n");
        sums.push(project.rows(&query).await);
    }
    sums
}

#[tokio::test]
async fn reloads_list_the_files_again() {
    let project = Project::new(&[("dir/a.csv", b"n\n1\n")]).await;
    let script = "CREATE EXTERNAL TABLE numbers OPTIONS (format = 'CSV', uris = ['dir/*.csv']);
        CREATE EXTERNAL TABLE listed OPTIONS (format = 'CSV', uris = ['dir/']);";
    project.load(script).await;
    project.put("dir/b.csv", b"n\n2\n").await;
    // The session keeps the listings of its queries, which a reload of the script lists again.
    project.load(script).await;
    for rows in sums(&project, &["numbers", "listed"]).await {
        assert!(rows.contains("| 3 |"), "{rows}");
    }
}

#[tokio::test]
async fn queries_list_the_files_of_a_hive_layout_without_a_cache() {
    use datafusion::execution::{
        cache::cache_manager::CacheManagerConfig, runtime_env::RuntimeEnvBuilder,
    };
    let runtime = RuntimeEnvBuilder::new()
        .with_cache_manager(CacheManagerConfig::default().with_list_files_cache_limit(0))
        .build_arc()
        .unwrap();
    let ctx = SessionContext::new_with_config_rt(Default::default(), runtime);
    let project = Project::with_session(
        ctx,
        &[("dir/a.csv", b"n\n1\n"), ("parts/k=a/a.csv", b"n\n1\n")],
    )
    .await;
    project
        .load(
            "CREATE EXTERNAL TABLE listed OPTIONS (format = 'CSV', uris = ['dir/']);
            CREATE EXTERNAL TABLE parts WITH PARTITION COLUMNS
            OPTIONS (format = 'CSV', uris = ['parts/'], hive_partition_uri_prefix = 'parts/');",
        )
        .await;
    project.put("dir/b.csv", b"n\n2\n").await;
    project.put("parts/k=b/b.csv", b"n\n2\n").await;
    // Without the session's cache of listings, each query of a Hive layout lists its files, and
    // a table without partition columns reads the files that it matched when it was built.
    let sums = sums(&project, &["listed", "parts"]).await;
    assert!(sums[0].contains("| 1 |"), "{}", sums[0]);
    assert!(sums[1].contains("| 3 |"), "{}", sums[1]);
}

#[tokio::test]
async fn first_csv_file_types_the_columns() {
    let project = Project::new(&[
        ("sales/a.csv", b"region,amount\neast,10.5\n"),
        ("sales/b.csv", b"region,amount\nwest,20\n"),
    ])
    .await;
    project
        .load("CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['sales/']);")
        .await;
    assert_eq!(
        project.columns("sales").await,
        columns(&[("region", DataType::Utf8), ("amount", DataType::Float64)])
    );
    let rows = project
        .rows("FROM sales |> AGGREGATE sum(amount) AS total")
        .await;
    assert!(rows.contains("| 30.5 "), "{rows}");
}

#[tokio::test]
async fn patterns_and_directories_match_names_in_one_directory() {
    let project = Project::new(&[
        // Names with `=`, `?`, `[`, a space and `%`, which the table reads by their URLs.
        ("data/ratio=1.csv", b"n\n1\n"),
        ("data/a b?.csv", b"n\n2\n"),
        ("data/[x].csv", b"n\n4\n"),
        ("data/p%20q.csv", b"n\n8\n"),
        // A subdirectory that looks like a partition of a Hive layout, and another extension.
        ("data/year=2025/x.csv", b"n\n16\n"),
        ("data/sub/y.csv", b"n\n32\n"),
        ("data/notes.txt", b"n\n64\n"),
    ])
    .await;
    project
        .load(
            r"CREATE EXTERNAL TABLE pattern OPTIONS (format = 'CSV', uris = ['data/*.csv']);
            CREATE EXTERNAL TABLE folder OPTIONS (format = 'CSV', uris = ['data/']);
            CREATE EXTERNAL TABLE windows OPTIONS (format = 'CSV', uris = ['data\\']);
            CREATE EXTERNAL TABLE one OPTIONS (format = 'CSV', uris = ['data/ratio=1.csv']);",
        )
        .await;
    for rows in sums(&project, &["pattern", "folder", "windows"]).await {
        assert!(rows.contains("| 15 |"), "{rows}");
    }
    assert!(sums(&project, &["one"]).await[0].contains("| 1 |"));
}

#[tokio::test]
async fn uris_of_directories_end_with_a_separator() {
    let project = Project::new(&[("data/a.csv", b"n\n1\n")]).await;
    assert_eq!(
        project
            .errors(
                "CREATE EXTERNAL TABLE a OPTIONS (format = 'CSV', uris = ['data']);
                CREATE EXTERNAL TABLE b OPTIONS (format = 'CSV', uris = ['.']);"
            )
            .await,
        [
            "'data' names a directory, and the URI of a directory ends with /, as 'data/'",
            "'.' names a directory, and the URI of a directory ends with /, as './'",
        ]
    );
    // A directory of the local file system, which has no file at its path.
    let dir = std::env::temp_dir().join(format!("avenger-sql-directory-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.csv"), b"n\n1\n").unwrap();
    let path = dir.display().to_string();
    let errors = project
        .errors(&format!(
            "CREATE EXTERNAL TABLE local OPTIONS (format = 'CSV', uris = ['{path}'])"
        ))
        .await;
    assert_eq!(
        errors,
        [format!(
            "'{path}' names a directory, and the URI of a directory ends with /, as '{path}/'"
        )]
    );
    project
        .load(&format!(
            "CREATE EXTERNAL TABLE local OPTIONS (format = 'CSV', uris = ['{path}/'])"
        ))
        .await;
    assert!(sums(&project, &["local"]).await[0].contains("| 1 |"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn hive_prefixes_end_with_either_separator() {
    let project = Project::new(&hive_files()).await;
    project
        .load(
            r"CREATE EXTERNAL TABLE sales WITH PARTITION COLUMNS
            OPTIONS (format = 'CSV', uris = ['sales\\*.csv'], hive_partition_uri_prefix = 'sales\\');",
        )
        .await;
    assert_eq!(
        project
            .rows("FROM sales |> SELECT region, month, amount |> ORDER BY month, region")
            .await,
        table(SALES_ROWS)
    );
}

#[tokio::test]
async fn partition_keys_are_not_columns_of_a_column_list() {
    let project = Project::new(&hive_files()).await;
    let errors = project
        .errors(
            "CREATE EXTERNAL TABLE sales (region STRING, month INT64, amount FLOAT64)
            WITH PARTITION COLUMNS
            OPTIONS (format = 'CSV', uris = ['sales/'], hive_partition_uri_prefix = 'sales/',
              skip_leading_rows = 1);",
        )
        .await;
    assert_eq!(
        errors,
        ["Column region is both a column of the files and a partition key"]
    );
}

#[tokio::test]
async fn the_first_json_file_with_rows_tells_arrays_from_lines() {
    let project = Project::new(&[("cars/a.json", b""), ("cars/b.json", CARS_JSON)]).await;
    project
        .load("CREATE EXTERNAL TABLE cars OPTIONS (format = 'JSON', uris = ['cars/']);")
        .await;
    let rows = project.rows("FROM cars |> AGGREGATE count(*) AS n").await;
    assert!(rows.contains("| 2 |"), "{rows}");
}

#[tokio::test]
async fn files_that_no_statement_writes_are_errors() {
    let text = "CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/sales.csv']);";
    let files = file_reads(&parse_data(text).syntax(), Default::default())
        .remove(0)
        .files;
    let project = Project::new(&[("data/sales.csv", SALES_CSV)]).await;
    let state = project.ctx.state();
    let without_options = Files { csv: None, ..files };
    assert_eq!(
        file_table(&state, &without_options, BASE)
            .await
            .err()
            .unwrap(),
        "The CSV files have no options of CSV files"
    );
}

/// The names of the tables of the default schema of `ctx`.
fn table_names(ctx: &SessionContext) -> Vec<String> {
    let mut names = ctx
        .catalog("datafusion")
        .unwrap()
        .schema("public")
        .unwrap()
        .table_names();
    names.sort();
    names
}

#[tokio::test]
async fn tables_replace_those_of_the_same_name_in_any_case() {
    let project = Project::new(&[("data/sales.csv", SALES_CSV)]).await;
    // A table of the host.
    project
        .ctx
        .register_batch(TableReference::bare("SALES"), sales_batch())
        .unwrap();
    let replaced = |loaded: &Loaded| -> Vec<String> {
        (loaded.registered[0].replaced.iter())
            .map(|(name, _)| name.clone())
            .collect()
    };
    let loaded = project
        .load("LOAD DATA OVERWRITE sales FROM FILES (format = 'CSV', uris = ['data/sales.csv']);")
        .await;
    assert_eq!(table_names(&project.ctx), ["sales"]);
    // The registration holds the table of the host that it replaced, for a loader to restore.
    assert_eq!(replaced(&loaded), ["SALES"]);
    let (_, host) = &loaded.registered[0].replaced[0];
    assert_eq!(host.schema(), sales_batch().schema());
    // A reload of the script with another case of the name.
    let loaded = project
        .load("LOAD DATA OVERWRITE Sales FROM FILES (format = 'CSV', uris = ['data/sales.csv']);")
        .await;
    assert_eq!(table_names(&project.ctx), ["Sales"]);
    assert_eq!(replaced(&loaded), ["sales"]);
    assert!(project
        .rows("FROM sales |> AGGREGATE count(*) AS n")
        .await
        .contains("| 3 |"));
}

/// A schema of the host that registers no tables, as the schema of another system can be, with
/// one table, `t`.
#[derive(Debug)]
struct Fixed(Arc<dyn TableProvider>);

#[async_trait::async_trait]
impl SchemaProvider for Fixed {
    fn table_names(&self) -> Vec<String> {
        vec!["t".to_string()]
    }

    async fn table(
        &self,
        name: &str,
    ) -> datafusion::common::Result<Option<Arc<dyn TableProvider>>> {
        Ok((name == "t").then(|| self.0.clone()))
    }

    fn table_exist(&self, name: &str) -> bool {
        name == "t"
    }
}

#[tokio::test]
async fn a_registration_that_fails_restores_the_session() {
    let project = Project::new(&[("data/sales.csv", SALES_CSV)]).await;
    let batch = sales_batch();
    let table = MemTable::try_new(batch.schema(), vec![vec![batch]]).unwrap();
    let catalog = project.ctx.catalog("datafusion").unwrap();
    catalog
        .register_schema("fixed", Arc::new(Fixed(Arc::new(table))))
        .unwrap();
    project
        .load("CREATE TABLE sales AS (FROM fixed.t |> WHERE region = 'east');")
        .await;
    let text = "CREATE SCHEMA finance;
        LOAD DATA OVERWRITE sales FROM FILES (format = 'CSV', uris = ['data/sales.csv']);
        CREATE TABLE fixed.copy AS (FROM fixed.t);";
    let (analysis, lowered) = project.analyze(text).await;
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let errors = lower(&project.ctx, &analysis, &lowered).await.unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].definition, 2);
    // The session keeps the schemas and the table that it had.
    assert!(catalog.schema("finance").is_none());
    assert_eq!(table_names(&project.ctx), ["sales"]);
    assert!(project
        .rows("FROM sales |> AGGREGATE count(*) AS n")
        .await
        .contains("| 2 |"));
}
