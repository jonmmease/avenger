//! `avenger query`: the rows of a query in a project's session, as a chart script of the project
//! runs, with the tables of its data scripts and its modules. The query is SQL on the command
//! line, or with `-f`, a chart script or a cell, as `render` takes it, or another file of the
//! project. The result collects before anything prints, so a query that fails writes no rows,
//! and its errors print as `check` prints them.
//!
//! The rows print as a table, CSV, JSON (an array of objects), Parquet, or Arrow IPC, by
//! `--output`, or by the extension of `-o`. With `--json`, and without `--output` or `-o`, they
//! print in the versioned object of the other commands, with their columns. A chart cell prints
//! as `<chart>`, except in Parquet and Arrow, which keep its bytes. `--explain` prints
//! DataFusion's logical and physical plans in place of the rows.
//!
//! `--limit N` keeps the first `N` rows, and `--limit all` every row. Without it, rows that print
//! keep the first 10, and rows that `-o` writes to a file keep every row. A limit that cuts rows
//! off says so on standard error, or with `--json`, in the object's `more`.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use avenger_chart::results::is_chart_field;
use avenger_sql::types::googlesql_name;
use avenger_sql_language::FileRole;
use avenger_sql_project::{find_root, qmd::is_document, ProjectFiles, Role};
use datafusion::arrow::{
    array::{Array, ArrayRef, StringArray},
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use serde_json::json;

use crate::{
    project::open,
    render::{load, plan_target, Failure, Target},
    report::Output,
    Status,
};

/// The path of a query of the command line in the analysis of its project, and in its reports.
const QUERY_PATH: &str = "<query>";

/// What a query runs.
pub enum Source {
    /// SQL of the command line.
    Text(String),
    /// A chart script, a cell as `doc.qmd#NAME`, or another file of a project, whose text runs
    /// in the project.
    File(String),
}

/// The options of a query.
pub struct Options {
    /// The project of SQL of the command line: a directory in it, or the current directory.
    pub project: Option<PathBuf>,
    /// The limit of `--limit`, or `None` for the default: 10 rows that print, and every row that
    /// `-o` writes.
    pub limit: Option<Limit>,
    /// The format of `--output`.
    pub format: Option<String>,
    pub explain: bool,
    /// The file of `-o`.
    pub out: Option<PathBuf>,
}

/// The limit of `--limit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    Rows(usize),
    All,
}

impl Limit {
    /// The limit of `text`: a number of rows, or `all`.
    pub fn parse(text: &str) -> Result<Limit, String> {
        match text {
            "all" => Ok(Limit::All),
            text => (text.parse().map(Limit::Rows))
                .map_err(|_| "a number of rows, or `all`".to_string()),
        }
    }
}

/// The rows that print when `--limit` does not say.
const PRINTED_ROWS: usize = 10;

/// The formats of the rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Table,
    Csv,
    Json,
    Parquet,
    Arrow,
}

impl Format {
    fn named(name: &str) -> Option<Format> {
        match name.to_ascii_lowercase().as_str() {
            "table" | "txt" => Some(Format::Table),
            "csv" => Some(Format::Csv),
            "json" => Some(Format::Json),
            "parquet" => Some(Format::Parquet),
            "arrow" | "ipc" => Some(Format::Arrow),
            _ => None,
        }
    }
}

/// What the command prints: the rows in a format, or the versioned object of `--json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Print {
    Rows(Format),
    Object,
}

/// What `options` and `--json` print: `--output`, or the extension of `-o`, or with `--json`,
/// the versioned object, and otherwise a table.
fn print(options: &Options, output: &Output) -> Result<Print, String> {
    let explicit = match &options.format {
        Some(name) => Some(Format::named(name).ok_or_else(|| {
            format!("`--output {name}`: the formats are table, csv, json, parquet, and arrow")
        })?),
        None => None,
    };
    if options.explain && explicit.is_some() {
        return Err("`--explain` prints the plan, and takes no `--output`".into());
    }
    if output.json() && explicit.is_some() && options.out.is_none() {
        return Err(
            "`--json` and `--output` both choose what prints: use `--json` for the object of \
             the columns and the rows, or `--output json` for the rows alone"
                .into(),
        );
    }
    let by_extension = || {
        let out = options.out.as_ref()?;
        let extension = out.extension().and_then(|e| e.to_str()).unwrap_or_default();
        Some(Format::named(extension).ok_or_else(|| {
            format!(
                "{}: name the format with `--output`, or end the file in `.csv`, `.json`, \
                 `.parquet`, or `.arrow`",
                out.display()
            )
        }))
    };
    Ok(match (explicit, by_extension()) {
        (Some(format), _) => Print::Rows(format),
        (None, Some(format)) => Print::Rows(format?),
        (None, None) if output.json() => Print::Object,
        (None, None) => Print::Rows(Format::Table),
    })
}

/// Runs the query of `source` and prints its rows.
pub async fn run(source: Source, options: &Options, output: &Output) -> Result<Status, String> {
    let print = print(options, output)?;
    if let Print::Rows(format @ (Format::Parquet | Format::Arrow)) = print {
        if options.out.is_none() && std::io::stdout().is_terminal() {
            return Err(format!(
                "{format:?} is binary: write it to a file with `-o`, or pipe it to a program"
            ));
        }
    }
    let target = match source {
        Source::Text(text) => {
            let project = options.project.as_deref();
            query_target(project, text, PathBuf::from(QUERY_PATH)).await?
        }
        Source::File(file) => file_target(&file).await?,
    };
    let frame = match plan_target(&target).await {
        Ok(frame) => frame,
        Err(failure) => return failure_status(failure, output),
    };
    if options.explain {
        let plan = frame.explain(false, false).map_err(|e| e.to_string())?;
        return match plan.collect().await {
            Ok(batches) => explain(&batches, options, output),
            Err(error) => failure_status(target.failure(error), output),
        };
    }
    let limit = match (options.limit, &options.out) {
        (Some(Limit::Rows(rows)), _) => Some(rows),
        (Some(Limit::All), _) | (None, Some(_)) => None,
        (None, None) => Some(PRINTED_ROWS),
    };
    // One row past the limit tells whether the limit cut rows off.
    let frame = match limit {
        Some(limit) => frame.limit(0, Some(limit + 1)).map_err(|e| e.to_string())?,
        None => frame,
    };
    // The columns come from the query's schema, so a result without rows still has its header.
    let schema: SchemaRef = frame.schema().inner().clone();
    let batches = match frame.collect().await {
        Ok(batches) => batches,
        Err(error) => return failure_status(target.failure(error), output),
    };
    let (batches, more) = match limit {
        Some(limit) => first_rows(&schema, &batches, limit)?,
        None => (batches, false),
    };
    if let (true, false, Some(limit)) = (more, output.json(), limit) {
        let rows = match limit {
            1 => "row".to_string(),
            _ => format!("{limit} rows"),
        };
        eprintln!("the first {rows} of more: `--limit N` for more, or `--limit all`");
    }
    match print {
        Print::Object => {
            let columns: Vec<serde_json::Value> = (schema.fields().iter())
                .map(|field| {
                    let data_type = googlesql_name(field.data_type());
                    let chart = is_chart_field(field);
                    json!({"name": field.name(), "type": data_type, "chart": chart})
                })
                .collect();
            let (_, batches) = chart_text(&schema, &batches)?;
            let rows = json_rows(&batches)?;
            let mut fields = serde_json::Map::new();
            fields.insert("columns".into(), json!(columns));
            fields.insert("more".into(), json!(more));
            output.object_with("rows", &rows, fields);
        }
        Print::Rows(format) => {
            let bytes = write(format, &schema, &batches)?;
            emit(&bytes, options, output)?;
        }
    }
    Ok(Status::Ok)
}

/// The query of the text `text` in the project of `project`, or of the current directory, as a
/// chart script at [`QUERY_PATH`], whose reports are of the file `file`.
async fn query_target(
    project: Option<&Path>,
    text: String,
    file: PathBuf,
) -> Result<Target, String> {
    let mut opened = open(project).await?;
    let stylesheet = opened
        .files
        .load_stylesheet()
        .await
        .map_err(|e| e.to_string())?;
    opened
        .analysis
        .project_mut()
        .add(QUERY_PATH, FileRole::Chart, text.as_str());
    Ok(Target {
        loader: opened.loader,
        analysis: opened.analysis,
        stylesheet,
        path: QUERY_PATH.to_string(),
        text,
        script: QUERY_PATH.to_string(),
        file,
        document: None,
        cell: None,
        files: opened.files,
        root: Some(opened.root),
        loose: None,
        line: None,
        statement: None,
        data: false,
    })
}

/// The query of `-f`: a chart script, a cell, or a data script's statement, as `render` loads
/// it, or another file of a project, whose text runs in it. A loose script runs as a project of
/// one file. The project of the file loads once.
async fn file_target(file: &str) -> Result<Target, String> {
    // A file of a project without the role of a chart script runs in its project. A document's
    // errors, such as a cell that it lacks, are its own.
    // A target at a line names a statement, as `render` loads it.
    let (path, line) = crate::statement::split_line(file)?;
    let document = (path.rsplit_once('#')).is_some_and(|(path, _)| is_document(path));
    if line.is_none() && !document && !is_document(file) {
        let canonical = std::fs::canonicalize(file).map_err(|error| format!("{file}: {error}"))?;
        if let Some(root) = find_root(&canonical) {
            let files = (ProjectFiles::find(&root).await)
                .map_err(|error| format!("{}: {error}", root.display()))?;
            let relative = canonical.strip_prefix(&root).unwrap_or(&canonical);
            let relative = relative.to_string_lossy().replace('\\', "/");
            if !matches!(
                files.manifest().role(&relative),
                Ok(Some(Role::ChartScript | Role::DataScript))
            ) {
                let text = (std::fs::read_to_string(&canonical))
                    .map_err(|error| format!("{file}: {error}"))?;
                return query_target(Some(&root), text, canonical).await;
            }
        }
    }
    match load(file).await {
        Ok(target) => Ok(target),
        Err(Failure::Usage(error)) => Err(error),
        Err(Failure::Reports(_)) => unreachable!("a load reports no diagnostics"),
    }
}

/// Prints the reports of `failure`, as text on standard error, away from the rows of standard
/// output, or as JSON, and gives the status of a failing command, or the error of a command that
/// cannot run.
fn failure_status(failure: Failure, output: &Output) -> Result<Status, String> {
    match failure {
        Failure::Reports(reports) if output.json() => {
            output.diagnostics(reports);
            Ok(Status::Failed)
        }
        Failure::Reports(reports) => {
            output.write_diagnostics(&mut std::io::stderr().lock(), reports);
            Ok(Status::Failed)
        }
        Failure::Usage(error) => Err(error),
    }
}

/// Prints the plans of the rows `batches` of an `EXPLAIN`, each with its type.
fn explain(batches: &[RecordBatch], options: &Options, output: &Output) -> Result<Status, String> {
    let mut plans = Vec::new();
    for batch in batches {
        let text = |index: usize| -> Vec<String> {
            let column = batch.column(index);
            match column.as_any().downcast_ref::<StringArray>() {
                Some(strings) => (0..strings.len())
                    .map(|row| strings.value(row).to_string())
                    .collect(),
                None => Vec::new(),
            }
        };
        plans.extend(text(0).into_iter().zip(text(1)));
    }
    if output.json() && options.out.is_none() {
        let plans: Vec<serde_json::Value> = (plans.iter())
            .map(|(kind, plan)| json!({"type": kind, "plan": plan}))
            .collect();
        let mut fields = serde_json::Map::new();
        fields.insert("plans".into(), json!(plans));
        output.object(fields);
        return Ok(Status::Ok);
    }
    let text: Vec<String> = (plans.iter())
        .map(|(kind, plan)| format!("{kind}:\n{plan}\n"))
        .collect();
    emit(text.join("\n").as_bytes(), options, output)?;
    Ok(Status::Ok)
}

/// Writes `bytes` to the file of `-o`, and prints its path, or to standard output.
fn emit(bytes: &[u8], options: &Options, output: &Output) -> Result<(), String> {
    match &options.out {
        Some(out) => {
            std::fs::write(out, bytes).map_err(|error| format!("{}: {error}", out.display()))?;
            output.written(std::slice::from_ref(out));
        }
        None => {
            let mut stdout = std::io::stdout().lock();
            (stdout.write_all(bytes).and_then(|()| stdout.flush()))
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// The rows `batches`, of the schema `schema`, in `format`.
fn write(format: Format, schema: &SchemaRef, batches: &[RecordBatch]) -> Result<Vec<u8>, String> {
    let error = |error: datafusion::arrow::error::ArrowError| error.to_string();
    match format {
        Format::Table | Format::Csv | Format::Json => {
            let (schema, batches) = chart_text(schema, batches)?;
            match format {
                Format::Table => {
                    let table = datafusion::arrow::util::pretty::pretty_format_batches_with_schema(
                        schema, &batches,
                    )
                    .map_err(error)?;
                    Ok(format!("{table}\n").into_bytes())
                }
                Format::Csv => {
                    let mut writer = datafusion::arrow::csv::WriterBuilder::new()
                        .with_header(true)
                        .build(Vec::new());
                    // An empty batch writes the header of a result without rows.
                    writer
                        .write(&RecordBatch::new_empty(schema))
                        .map_err(error)?;
                    for batch in &batches {
                        writer.write(batch).map_err(error)?;
                    }
                    Ok(writer.into_inner())
                }
                _ => json_rows(&batches),
            }
        }
        Format::Parquet => {
            use datafusion::parquet::arrow::ArrowWriter;
            let mut bytes = Vec::new();
            let mut writer = ArrowWriter::try_new(&mut bytes, schema.clone(), None)
                .map_err(|e| e.to_string())?;
            for batch in batches {
                writer.write(batch).map_err(|e| e.to_string())?;
            }
            writer.close().map_err(|e| e.to_string())?;
            Ok(bytes)
        }
        Format::Arrow => {
            use datafusion::arrow::ipc::writer::FileWriter;
            let mut bytes = Vec::new();
            let mut writer = FileWriter::try_new(&mut bytes, schema).map_err(error)?;
            for batch in batches {
                writer.write(batch).map_err(error)?;
            }
            writer.finish().map_err(error)?;
            drop(writer);
            Ok(bytes)
        }
    }
}

/// The first `limit` rows of `batches`, of the schema `schema`, which hold at most one more, and
/// whether there was one more.
fn first_rows(
    schema: &SchemaRef,
    batches: &[RecordBatch],
    limit: usize,
) -> Result<(Vec<RecordBatch>, bool), String> {
    let rows = datafusion::arrow::compute::concat_batches(schema, batches)
        .map_err(|error| error.to_string())?;
    let more = rows.num_rows() > limit;
    Ok((vec![rows.slice(0, rows.num_rows().min(limit))], more))
}

/// The rows `batches` as a JSON array of objects, with their nulls.
fn json_rows(batches: &[RecordBatch]) -> Result<Vec<u8>, String> {
    use datafusion::arrow::json::{writer::JsonArray, WriterBuilder};
    let mut writer = WriterBuilder::new()
        .with_explicit_nulls(true)
        .build::<_, JsonArray>(Vec::new());
    for batch in batches {
        writer.write(batch).map_err(|e| e.to_string())?;
    }
    writer.finish().map_err(|e| e.to_string())?;
    Ok(writer.into_inner())
}

/// `schema` and `batches` with each column of charts as text: `<chart>`, or null.
fn chart_text(
    schema: &SchemaRef,
    batches: &[RecordBatch],
) -> Result<(SchemaRef, Vec<RecordBatch>), String> {
    let charts: Vec<usize> = (schema.fields().iter().enumerate())
        .filter(|(_, field)| is_chart_field(field))
        .map(|(index, _)| index)
        .collect();
    if charts.is_empty() {
        return Ok((schema.clone(), batches.to_vec()));
    }
    let fields: Vec<Field> = (schema.fields().iter().enumerate())
        .map(|(index, field)| match charts.contains(&index) {
            true => Field::new(field.name(), DataType::Utf8, true),
            false => field.as_ref().clone(),
        })
        .collect();
    let texts = Arc::new(Schema::new(fields));
    let batches = (batches.iter())
        .map(|batch| {
            let columns: Vec<ArrayRef> = (batch.columns().iter().enumerate())
                .map(|(index, column)| match charts.contains(&index) {
                    true => {
                        let text = (0..column.len())
                            .map(|row| (!column.is_null(row)).then_some("<chart>"));
                        Arc::new(text.collect::<StringArray>()) as ArrayRef
                    }
                    false => column.clone(),
                })
                .collect();
            RecordBatch::try_new(texts.clone(), columns).map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((texts, batches))
}
