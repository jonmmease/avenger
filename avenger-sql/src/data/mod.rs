//! Data scripts: the scripts of a project that register its tables with GoogleSQL's DDL.
//!
//! A data script, which [`avenger_sql_syntax::parse_data`] reads, holds `CREATE EXTERNAL TABLE`,
//! which declares a table over files, `LOAD DATA`, which loads files into a table once, `CREATE
//! VIEW`, `CREATE TABLE ... AS`, and `CREATE SCHEMA`, with `CREATE TEMP FUNCTION` and `IMPORT
//! MODULE` for its own statements. `TEMP` on a table keeps it inside its script. A chart script
//! holds `CREATE TEMP EXTERNAL TABLE` and `LOAD DATA ... TEMP TABLE` too, for its own tables of
//! files, which [`crate::analyze`] analyzes with the same code.
//!
//! [`FILE_OPTIONS`] is the table of the options of files, which the checks read, and
//! [`UNSUPPORTED_OPTIONS`] the options of BigQuery that a data script rejects. The options of a
//! statement that reads files give its [`Files`]. Reading the schema of files is I/O, so a loader
//! collects the files of a script with [`file_reads`] before the analysis, and a [`Catalog`]
//! gives the table of each, as [`Catalog::files`]. [`crate::analyze_data`] then analyzes the
//! script into a [`DataAnalysis`]: each table, view and schema that it defines, the reads that
//! leave it, and its errors.
//!
//! [`file_table`] builds the table of a [`Files`] over the object stores of a session, with its
//! URIs relative to the URL of the project's root, [`file_urls`] gives the URL of each of its
//! URIs, whose [`FileUrl::changed_by`] tells whether a change of a file or a directory changes
//! the files that it reads, and [`lower`] registers the tables, views and schemas of an analyzed
//! script in a session.
//!
//! [`Catalog`]: crate::catalog::Catalog
//! [`Catalog::files`]: crate::catalog::Catalog::files

mod lowering;
mod options;
mod table;

use std::{fmt, sync::Arc};

use avenger_sql_syntax::{ast, ast::AstNode, SyntaxNode, TextRange};
use datafusion::{
    arrow::datatypes::{Fields, SchemaRef},
    catalog::{Session, TableProvider},
    common::{plan_err, Result},
    logical_expr::{Expr, LogicalPlan, TableType},
    physical_plan::ExecutionPlan,
};

pub use lowering::{lower, schema_name, LoweringError, Registered, Registration, ScriptTables};
pub(crate) use options::{column_list, file_options, table_options, FileStatement};
pub use options::{
    file_option, option_format, option_places, unsupported_option, uri_kind, FileOption,
    OptionPlace, OptionType, OptionValue, UnsupportedOption, UriKind, FILE_OPTIONS,
    UNSUPPORTED_OPTIONS,
};
pub use table::{file_table, file_urls, FileUrl};

use crate::{
    analysis::{Diagnostic, Model},
    catalog::Reference,
    types::TypeOptions,
};

/// The format of files, the value of the option `format`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Csv,
    /// JSON files, of the format `NEWLINE_DELIMITED_JSON`, whose alias is `JSON`.
    Json,
    Parquet,
    Avro,
    /// Arrow IPC files, which is not one of BigQuery's formats.
    Arrow,
}

impl Format {
    pub const ALL: [Format; 5] = [
        Format::Csv,
        Format::Json,
        Format::Parquet,
        Format::Avro,
        Format::Arrow,
    ];

    /// The format of the value `text` of the option `format`, without regard to case.
    pub fn parse(text: &str) -> Option<Format> {
        let format = match text.to_ascii_uppercase().as_str() {
            "CSV" => Format::Csv,
            "NEWLINE_DELIMITED_JSON" | "JSON" => Format::Json,
            "PARQUET" => Format::Parquet,
            "AVRO" => Format::Avro,
            "ARROW" => Format::Arrow,
            _ => return None,
        };
        Some(format)
    }

    /// The name of the format, as the option `format` writes it.
    pub fn name(self) -> &'static str {
        match self {
            Format::Csv => "CSV",
            Format::Json => "NEWLINE_DELIMITED_JSON",
            Format::Parquet => "PARQUET",
            Format::Avro => "AVRO",
            Format::Arrow => "ARROW",
        }
    }

    /// The extension of the files of the format, which a URI of a directory reads.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
            Format::Parquet => "parquet",
            Format::Avro => "avro",
            Format::Arrow => "arrow",
        }
    }

    /// The cargo feature of avenger-sql that reads the format, for a format that needs one.
    pub fn feature(self) -> Option<&'static str> {
        match self {
            Format::Parquet => Some("parquet"),
            Format::Avro => Some("avro"),
            Format::Csv | Format::Json | Format::Arrow => None,
        }
    }

    /// Whether this build reads the format.
    pub fn available(self) -> bool {
        match self {
            Format::Parquet => cfg!(feature = "parquet"),
            Format::Avro => cfg!(feature = "avro"),
            Format::Csv | Format::Json | Format::Arrow => true,
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The compression of files, the value of the option `compression`: BigQuery's `GZIP`, and
/// DataFusion's others, which the cargo feature `compression` reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Compression {
    Gzip,
    Bzip2,
    Xz,
    Zstd,
}

impl Compression {
    /// The compression of the value `text` of the option `compression`, without regard to case.
    pub fn parse(text: &str) -> Option<Compression> {
        let compression = match text.to_ascii_uppercase().as_str() {
            "GZIP" => Compression::Gzip,
            "BZIP2" => Compression::Bzip2,
            "XZ" => Compression::Xz,
            "ZSTD" => Compression::Zstd,
            _ => return None,
        };
        Some(compression)
    }

    pub fn name(self) -> &'static str {
        match self {
            Compression::Gzip => "GZIP",
            Compression::Bzip2 => "BZIP2",
            Compression::Xz => "XZ",
            Compression::Zstd => "ZSTD",
        }
    }

    /// The extension of a compressed file, after the extension of its format, as `gz` of
    /// `sales.csv.gz`, which a URI of a directory reads.
    pub fn extension(self) -> &'static str {
        match self {
            Compression::Gzip => "gz",
            Compression::Bzip2 => "bz2",
            Compression::Xz => "xz",
            Compression::Zstd => "zst",
        }
    }

    /// Whether this build reads the compression.
    pub fn available(self) -> bool {
        cfg!(feature = "compression")
    }
}

/// How a CSV file reads.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CsvOptions {
    /// Whether the first row is a header: it names the columns of a table without a column
    /// list, and `skip_leading_rows = 1` skips it in a table with one.
    pub header: bool,
    /// The byte between fields, of `field_delimiter`.
    pub delimiter: u8,
    /// The byte that quotes a field, of `quote`.
    pub quote: u8,
    /// Whether a quoted field can hold a line break, of `allow_quoted_newlines`.
    pub quoted_newlines: bool,
}

/// The partition columns of `WITH PARTITION COLUMNS`, the keys of a Hive layout's paths after
/// the option `hive_partition_uri_prefix`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Partitions {
    /// The clause without a list: the keys of the paths as columns, whose types a loader gives.
    Inferred,
    /// The columns of the list, in the order of the path.
    Columns(Fields),
}

/// A set of files to read, as the options of a statement describe them: what a catalog turns
/// into a table, and what a loader reads. Two statements with equal options read equal files,
/// so a catalog can keep the table of each by its `Files`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Files {
    pub format: Format,
    /// The URIs as the script writes them: paths or `file:` URLs, each with one `*` at most. A
    /// loader resolves a relative path against the project root, and decides whether it reads
    /// an absolute path, a path above the root, or a `file:` URL.
    pub uris: Vec<String>,
    /// The columns of the statement's column list, whose types the files' values cast to.
    /// Without one, the table takes the schema of its files.
    pub columns: Option<Fields>,
    /// How the files read, for CSV files.
    pub csv: Option<CsvOptions>,
    pub compression: Option<Compression>,
    pub hive_partition_uri_prefix: Option<String>,
    pub partitions: Option<Partitions>,
}

/// A statement of a data script that reads files: an external table or a `LOAD DATA`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRead {
    /// The path of the table, as the statement writes it.
    pub names: Vec<String>,
    pub files: Files,
    /// The range of the statement.
    pub range: TextRange,
    /// The range of the value of `uris`, where an error of reading the files goes.
    pub uris: TextRange,
}

/// The files that the external tables and the `LOAD DATA` statements of the data script `root`
/// read, in the order of the text, with the types of their column lists by `types`, the
/// [`type_options`](crate::catalog::Catalog::type_options) of the catalog of the analysis, so that
/// its [`files`](crate::catalog::Catalog::files) finds them. A statement
/// whose options have errors reads none. A loader reads the schemas of these files before the
/// analysis, as a catalog fetches the tables of [`crate::catalog::references`].
pub fn file_reads(root: &SyntaxNode, types: TypeOptions) -> Vec<FileRead> {
    let mut reads = Vec::new();
    for node in root.descendants() {
        let (path, statement) =
            if let Some(external) = ast::CreateExternalTableStmt::cast(node.clone()) {
                (external.path(), FileStatement::external(&external))
            } else if let Some(load) = ast::LoadDataStmt::cast(node.clone()) {
                (load.path(), FileStatement::load(&load))
            } else {
                continue;
            };
        let (Some(names), Some(statement)) = (path.and_then(|path| path.names()), statement) else {
            continue;
        };
        let options = file_options(&statement, types, &mut Vec::new());
        if let (Some(files), Some(uris)) = (options.files, options.uris) {
            reads.push(FileRead {
                names,
                files,
                range: node.text_range(),
                uris,
            });
        }
    }
    reads
}

/// The analysis of a data script, which [`crate::analyze_data`] gives.
#[derive(Clone, Debug)]
pub struct DataAnalysis {
    /// The statements that define a table, a view, or a schema, or load files into a table, in
    /// the order of the text.
    pub definitions: Vec<Definition>,
    /// The reads of tables of the catalog, in the order of the text, whether the catalog holds
    /// them or not: the reads of tables that the script doesn't define, and every read of the
    /// code of a module, which reads no table of the script. A read in the body of a function is
    /// at the call that plans it, and for a function of a module, at the import that checks it
    /// too.
    pub reads: Vec<Reference>,
    /// The semantic model of the queries of the script's views and tables.
    pub model: Model,
    pub diagnostics: Vec<Diagnostic>,
}

impl DataAnalysis {
    /// The definitions that other scripts see: those without `TEMP` that create their name and
    /// that no later definition of the script replaces. An append of `LOAD DATA INTO` adds rows
    /// to its table and defines none.
    pub fn exported(&self) -> impl Iterator<Item = &Definition> {
        let definitions = &self.definitions;
        (definitions.iter().enumerate())
            .filter(|(_, definition)| !definition.temp && definition.appends().is_none())
            .filter(move |(index, definition)| {
                !definitions[index + 1..].iter().any(|later| {
                    later.appends().is_none()
                        && later.is_schema() == definition.is_schema()
                        && crate::catalog::same_path(&later.names, &definition.names)
                })
            })
            .map(|(_, definition)| definition)
    }
}

/// A statement of a data script that defines a table, a view, or a schema, or loads files into
/// a table, or a chart script's statement of its own table of files.
#[derive(Clone, Debug)]
pub struct Definition {
    /// The path of its name, as the statement writes it.
    pub names: Vec<String>,
    pub kind: DefinitionKind,
    /// Whether it is `TEMP`, so only the later statements of its script read it.
    pub temp: bool,
    /// The range of the path of its name.
    pub name_range: TextRange,
    /// The range of the statement.
    pub range: TextRange,
    /// The `description` of its options.
    pub description: Option<String>,
    /// The schema of the table: its column list, the output of its query, or the schema of its
    /// files that the catalog gives. `None` for a schema, and for a table whose statement has an
    /// error that leaves its columns unknown.
    pub schema: Option<SchemaRef>,
}

impl Definition {
    /// The index of the definition whose table an append of `LOAD DATA INTO` adds rows to.
    pub fn appends(&self) -> Option<usize> {
        match &self.kind {
            DefinitionKind::LoadData { appends, .. } => *appends,
            _ => None,
        }
    }

    fn is_schema(&self) -> bool {
        matches!(self.kind, DefinitionKind::Schema)
    }
}

/// What a definition of a data script is.
#[derive(Clone, Debug)]
pub enum DefinitionKind {
    /// `CREATE EXTERNAL TABLE`, a table over files that each query reads again. `files` is
    /// `None` when its options have errors.
    ExternalTable { files: Option<Files> },
    /// `LOAD DATA`, which loads files into a table once.
    LoadData {
        files: Option<Files>,
        mode: LoadMode,
        /// For `LOAD DATA INTO` a table that an earlier statement of the script creates, the
        /// index of that statement's definition, whose table takes the rows of the files.
        appends: Option<usize>,
    },
    /// `CREATE VIEW`, a query that each query that reads it runs again. `plan` is `None` when
    /// its query has errors.
    View { plan: Option<LogicalPlan> },
    /// `CREATE TABLE ... AS`, the rows of a query, computed once.
    TableAs { plan: Option<LogicalPlan> },
    /// `CREATE SCHEMA`, a namespace of tables.
    Schema,
}

/// The mode of a `LOAD DATA`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LoadMode {
    /// `LOAD DATA OVERWRITE`, which creates the table.
    Overwrite,
    /// `LOAD DATA INTO`, which creates the table, or appends to a table that an earlier
    /// statement of the script creates.
    Into,
}

/// The table of a definition of a data script or a chart script, as the plans of the statements
/// after it read it: a placeholder of its schema that a loader replaces with the table that it
/// builds. A scan of it is an error.
#[derive(Clone, Debug)]
pub struct ScriptTable {
    names: Vec<String>,
    definition: usize,
    schema: SchemaRef,
    table_type: TableType,
}

impl ScriptTable {
    /// The table of the definition at the index `definition` of [`DataAnalysis::definitions`],
    /// or of [`crate::analysis::Analysis::file_tables`], named `names`, of the schema `schema`.
    pub(crate) fn new(
        names: Vec<String>,
        definition: usize,
        schema: SchemaRef,
        table_type: TableType,
    ) -> Self {
        Self {
            names,
            definition,
            schema,
            table_type,
        }
    }

    /// The path of the table, as its definition writes it.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The index of the definition in [`DataAnalysis::definitions`] whose rows a read sees: the
    /// table's definition, or the last append of `LOAD DATA INTO` before the read.
    pub fn definition(&self) -> usize {
        self.definition
    }
}

#[async_trait::async_trait]
impl TableProvider for ScriptTable {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    fn table_type(&self) -> TableType {
        self.table_type
    }

    async fn scan(
        &self,
        _state: &dyn Session,
        _projection: Option<&Vec<usize>>,
        _filters: &[Expr],
        _limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        plan_err!(
            "{} is a table of a script's statement, which a loader builds before a query reads it",
            self.names.join(".")
        )
    }
}
