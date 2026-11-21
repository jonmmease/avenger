//! Data scripts: the scripts of a project that register its tables with GoogleSQL's DDL.
//!
//! A data script, which [`avenger_sql_syntax::parse_data`] reads, holds `CREATE EXTERNAL TABLE`,
//! which declares a table over files, `LOAD DATA`, which loads files into a table once, `CREATE
//! VIEW`, `CREATE TABLE ... AS`, and `CREATE SCHEMA`, with `CREATE TEMP FUNCTION` and `IMPORT
//! MODULE` for its own statements. `TEMP` on a table keeps it inside its script.
//!
//! [`FILE_OPTIONS`] is the table of the options of files, which the checks read, and
//! [`UNSUPPORTED_OPTIONS`] the options of BigQuery that a data script rejects. The options of a
//! statement that reads files give its [`Files`]. Reading the schema of files is I/O, so a loader
//! collects the files of a script with [`file_reads`].

mod options;

use std::fmt;

use avenger_sql_syntax::{ast, ast::AstNode, SyntaxNode, TextRange};
use datafusion::arrow::datatypes::Fields;

pub use options::{
    file_option, option_format, option_places, unsupported_option, uri_kind, FileOption,
    OptionPlace, OptionType, OptionValue, UnsupportedOption, UriKind, FILE_OPTIONS,
    UNSUPPORTED_OPTIONS,
};
use options::{file_options, FileStatement};

use crate::types::TypeOptions;

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

/// A set of files to read, as the options of a statement describe them: what a loader reads.
/// Two statements with equal options read equal files.
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
/// read, in the order of the text, with the types of their column lists by `types`. A statement
/// whose options have errors reads none. A loader reads the schemas of these files, as a catalog
/// fetches the tables of [`crate::catalog::references`].
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
