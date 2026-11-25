//! The options of files: the table of the options that `CREATE EXTERNAL TABLE ... OPTIONS (...)`
//! and `LOAD DATA ... FROM FILES (...)` take, the options of BigQuery that a data script rejects,
//! and the reading of a statement's options into [`Files`].

use crate::library::search::{closest_name, did_you_mean};
use std::fmt;

use avenger_sql_syntax::{
    ast::{self, AstNode, LiteralKind, PrefixOp},
    SyntaxKind, SyntaxNode, TextRange,
};
use datafusion::arrow::datatypes::{DataType, Field, Fields};

use super::{Compression, CsvOptions, Files, Format, Partitions};
use crate::{analysis::Diagnostic, types, types::TypeOptions};

/// The GoogleSQL type of the value of an option.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OptionType {
    String,
    Int64,
    Bool,
    /// `ARRAY<STRING>`.
    StringArray,
}

impl fmt::Display for OptionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OptionType::String => "STRING",
            OptionType::Int64 => "INT64",
            OptionType::Bool => "BOOL",
            OptionType::StringArray => "ARRAY<STRING>",
        })
    }
}

/// Where an option goes: with the options of the files, or with the options of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OptionPlace {
    /// The options of the files that a table reads: an external table's `OPTIONS (...)` and
    /// `FROM FILES (...)` of a `LOAD DATA`.
    Files,
    /// The options of a table: an external table's `OPTIONS (...)`, and the `OPTIONS (...)` of a
    /// `LOAD DATA`, a view, a table and a schema.
    Table,
    /// The options of a selection: the `OPTIONS (...)` of a `DECLARE ... SELECTION`.
    Selection,
}

/// An option of files or of a table, in a data script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileOption {
    /// The name, as BigQuery names it. Names match without regard to case.
    pub name: &'static str,
    pub ty: OptionType,
    /// Whether a statement that reads files takes it.
    pub required: bool,
    pub place: OptionPlace,
    /// The formats of the files that it applies to.
    pub formats: &'static [Format],
    /// What it means, in one sentence.
    pub meaning: &'static str,
    /// The values that it takes, when they are a list: in upper case for an option of files or of
    /// a table, as BigQuery spells them, and in lower case for an option of a selection. Values
    /// match without regard to case.
    pub values: &'static [&'static str],
}

/// The options of files and of tables that a data script takes, whose names and types are
/// BigQuery's, and the options of a selection.
pub const FILE_OPTIONS: &[FileOption] = &[
    FileOption {
        name: "format",
        ty: OptionType::String,
        required: true,
        place: OptionPlace::Files,
        formats: &Format::ALL,
        meaning: "The format of the files: CSV, NEWLINE_DELIMITED_JSON (or JSON), PARQUET, AVRO, or ARROW for Arrow IPC files.",
        values: &[
            "CSV",
            "NEWLINE_DELIMITED_JSON",
            "JSON",
            "PARQUET",
            "AVRO",
            "ARROW",
        ],
    },
    FileOption {
        name: "uris",
        ty: OptionType::StringArray,
        required: true,
        place: OptionPlace::Files,
        formats: &Format::ALL,
        meaning: "The files, each a path or a `file:` URL with one `*` at most, which a loader reads from the project root when it is relative. A `*` matches the names of the files of one directory, and a URI that ends with `/` or `\\` reads the files of its directory with the format's extension. A `file:` URL follows the syntax of URLs, so `?` and `#` start its query and its fragment, while in a path they are characters of a name.",
        values: &[],
    },
    FileOption {
        name: "skip_leading_rows",
        ty: OptionType::Int64,
        required: false,
        place: OptionPlace::Files,
        formats: &[Format::Csv],
        meaning: "The rows before the data: 1 for a header that a column list names in its place.",
        values: &[],
    },
    FileOption {
        name: "field_delimiter",
        ty: OptionType::String,
        required: false,
        place: OptionPlace::Files,
        formats: &[Format::Csv],
        meaning: "The character between the fields of a row, `,` by default.",
        values: &[],
    },
    FileOption {
        name: "quote",
        ty: OptionType::String,
        required: false,
        place: OptionPlace::Files,
        formats: &[Format::Csv],
        meaning: "The character that quotes a field, `\"` by default.",
        values: &[],
    },
    FileOption {
        name: "null_marker",
        ty: OptionType::String,
        required: false,
        place: OptionPlace::Files,
        formats: &[Format::Csv],
        meaning: "The text of a null value.",
        values: &[],
    },
    FileOption {
        name: "allow_quoted_newlines",
        ty: OptionType::Bool,
        required: false,
        place: OptionPlace::Files,
        formats: &[Format::Csv],
        meaning: "Whether a quoted field can hold a line break.",
        values: &[],
    },
    FileOption {
        name: "compression",
        ty: OptionType::String,
        required: false,
        place: OptionPlace::Files,
        formats: &[Format::Csv, Format::Json],
        meaning: "The compression of the files: GZIP, BZIP2, XZ, or ZSTD.",
        values: &["GZIP", "BZIP2", "XZ", "ZSTD"],
    },
    FileOption {
        name: "hive_partition_uri_prefix",
        ty: OptionType::String,
        required: false,
        place: OptionPlace::Files,
        formats: &Format::ALL,
        meaning: "The prefix of the URIs before the partition keys of a Hive layout, such as `year=2025/`, which `WITH PARTITION COLUMNS` reads.",
        values: &[],
    },
    FileOption {
        name: "description",
        ty: OptionType::String,
        required: false,
        place: OptionPlace::Table,
        formats: &Format::ALL,
        meaning: "The description of the table, the view, or the schema.",
        values: &[],
    },
    FileOption {
        name: "resolution",
        ty: OptionType::String,
        required: false,
        place: OptionPlace::Selection,
        formats: &[],
        meaning: "How the selection combines the clauses of the marks that write it: 'global' holds one clause, which a new gesture in any chart replaces, 'union' selects a row that some mark's clause selects, and 'intersect' a row that every mark's clause selects. When omitted, it is 'global'.",
        values: &["global", "union", "intersect"],
    },
];

/// A literal value of an option, in the table of the options that a data script rejects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionValue {
    /// A string, in upper case, which matches without regard to case.
    String(&'static str),
    Int64(i64),
    Bool(bool),
}

impl OptionValue {
    /// Whether `value` is this value: a string of equal text without regard to case, or an equal
    /// integer or boolean.
    fn matches(self, value: &Value) -> bool {
        match (self, value) {
            (OptionValue::String(text), Value::String(value)) => text.eq_ignore_ascii_case(value),
            (OptionValue::Int64(int), Value::Int(value)) => int == *value,
            (OptionValue::Bool(bool), Value::Bool(value)) => bool == *value,
            _ => false,
        }
    }
}

/// The value as GoogleSQL writes it, as `'UTF8'`, `0`, or `FALSE`.
impl fmt::Display for OptionValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OptionValue::String(text) => write!(f, "'{text}'"),
            OptionValue::Int64(value) => write!(f, "{value}"),
            OptionValue::Bool(true) => f.write_str("TRUE"),
            OptionValue::Bool(false) => f.write_str("FALSE"),
        }
    }
}

/// An option of BigQuery that a data script rejects as unsupported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnsupportedOption {
    pub name: &'static str,
    pub place: OptionPlace,
    /// Why a data script doesn't take it.
    pub reason: &'static str,
    /// The values that a data script accepts anyway, because they are what it does without the
    /// option.
    pub accepts: &'static [OptionValue],
}

const fn unsupported(name: &'static str, reason: &'static str) -> UnsupportedOption {
    accepting(name, reason, &[])
}

/// An unsupported option of files that takes the values `accepts`, which are what a data script
/// does without it.
const fn accepting(
    name: &'static str,
    reason: &'static str,
    accepts: &'static [OptionValue],
) -> UnsupportedOption {
    UnsupportedOption {
        name,
        place: OptionPlace::Files,
        reason,
        accepts,
    }
}

const fn table_unsupported(name: &'static str, reason: &'static str) -> UnsupportedOption {
    UnsupportedOption {
        name,
        place: OptionPlace::Table,
        reason,
        accepts: &[],
    }
}

/// The text of the formats of dates and times, which DataFusion's readers don't take.
const TIME_FORMATS: &str =
    "its format elements differ from the formats that DataFusion's CSV and JSON readers take";

/// The options of BigQuery's external tables, loads, and tables that a data script rejects.
pub const UNSUPPORTED_OPTIONS: &[UnsupportedOption] = &[
    accepting(
        "allow_jagged_rows",
        "DataFusion's CSV reader reads rows of equal length",
        &[OptionValue::Bool(false)],
    ),
    unsupported(
        "bigtable_options",
        "a data script reads files, not Bigtable",
    ),
    unsupported(
        "column_name_character_map",
        "a column takes the name that its file gives",
    ),
    unsupported(
        "decimal_target_types",
        "a decimal column keeps the type that its file gives",
    ),
    unsupported(
        "enable_list_inference",
        "a Parquet list keeps the type that its file gives",
    ),
    unsupported(
        "enable_logical_types",
        "an Avro logical type keeps the type that DataFusion's reader gives",
    ),
    accepting(
        "encoding",
        "DataFusion reads UTF-8 files only",
        &[OptionValue::String("UTF8"), OptionValue::String("UTF-8")],
    ),
    unsupported(
        "enum_as_string",
        "a Parquet enum keeps the type that DataFusion's reader gives",
    ),
    accepting(
        "file_set_spec_type",
        "the URIs name the files, not manifests",
        &[OptionValue::String("FILE_SYSTEM_MATCH")],
    ),
    accepting(
        "ignore_unknown_values",
        "DataFusion's readers read every column of a file",
        &[OptionValue::Bool(false)],
    ),
    unsupported("json_extension", "a data script reads no GeoJSON files"),
    accepting(
        "max_bad_records",
        "DataFusion's readers stop at a bad record",
        &[OptionValue::Int64(0)],
    ),
    unsupported(
        "max_staleness",
        "a data script caches no metadata of its files",
    ),
    unsupported(
        "metadata_cache_mode",
        "a data script caches no metadata of its files",
    ),
    unsupported("null_markers", "a table takes one null_marker"),
    unsupported("object_metadata", "a data script makes no object tables"),
    unsupported(
        "preserve_ascii_control_characters",
        "DataFusion's CSV reader keeps the characters of its fields",
    ),
    unsupported(
        "projection_fields",
        "a data script reads no Datastore exports",
    ),
    unsupported(
        "reference_file_schema_uri",
        "a table takes the schema of its first file or of its column list",
    ),
    accepting(
        "require_hive_partition_filter",
        "a query can read every partition of a table",
        &[OptionValue::Bool(false)],
    ),
    unsupported("sheet_range", "a data script reads no Google Sheets"),
    unsupported(
        "source_column_match",
        "a column list names the columns of a file by their position",
    ),
    unsupported(
        "time_zone",
        "a timestamp without a zone keeps the type that its file gives",
    ),
    unsupported("date_format", TIME_FORMATS),
    unsupported("datetime_format", TIME_FORMATS),
    unsupported("time_format", TIME_FORMATS),
    unsupported("timestamp_format", TIME_FORMATS),
    table_unsupported(
        "expiration_timestamp",
        "a project loads its tables again from their files",
    ),
    table_unsupported("friendly_name", "a table's name is the one that it has"),
    table_unsupported("labels", "a data script keeps no labels of tables"),
    table_unsupported("tags", "a data script keeps no tags of tables"),
    table_unsupported("kms_key_name", "a data script encrypts no tables"),
    table_unsupported(
        "partition_expiration_days",
        "a data script's tables have no partitions that expire",
    ),
    UnsupportedOption {
        name: "require_partition_filter",
        place: OptionPlace::Table,
        reason: "a query can read every partition of a table",
        accepts: &[OptionValue::Bool(false)],
    },
    table_unsupported(
        "default_rounding_mode",
        "a table's numbers keep the rounding of their computation",
    ),
];

/// The option of [`FILE_OPTIONS`] named `name`, without regard to case.
pub fn file_option(name: &str) -> Option<&'static FileOption> {
    (FILE_OPTIONS.iter()).find(|option| option.name.eq_ignore_ascii_case(name))
}

/// The option of [`UNSUPPORTED_OPTIONS`] named `name`, without regard to case.
pub fn unsupported_option(name: &str) -> Option<&'static UnsupportedOption> {
    (UNSUPPORTED_OPTIONS.iter()).find(|option| option.name.eq_ignore_ascii_case(name))
}

/// The places of the options that the list `list` takes, when it is the `OPTIONS (...)` or the
/// `FROM FILES (...)` of a statement of a data script: an external table's `OPTIONS` takes the
/// options of its files and of its table, a `FROM FILES` those of the files, and the `OPTIONS` of
/// a `LOAD DATA`, a view, a table, and a schema those of a table.
pub fn option_places(list: &SyntaxNode) -> Option<&'static [OptionPlace]> {
    use SyntaxKind::*;
    let places: &'static [OptionPlace] = match (list.kind(), list.parent()?.kind()) {
        (FROM_FILES, LOAD_DATA_STMT) => &[OptionPlace::Files],
        (OPTIONS, CREATE_EXTERNAL_TABLE_STMT) => &[OptionPlace::Files, OptionPlace::Table],
        (OPTIONS, LOAD_DATA_STMT | CREATE_VIEW_STMT | CREATE_TABLE_STMT | CREATE_SCHEMA_STMT) => {
            &[OptionPlace::Table]
        }
        (OPTIONS, DECLARE_STMT) => &[OptionPlace::Selection],
        _ => return None,
    };
    Some(places)
}

/// The format that the option `format` of the list of options `list` names, and the range of its
/// value, when the value is a string literal of a format of a data script, whether this build
/// reads the format or not. A list that sets `format` twice names the format of the first.
pub fn option_format(list: &SyntaxNode) -> Option<(Format, TextRange)> {
    let entry = (list.children().filter_map(ast::OptionEntry::cast)).find(|entry| {
        (entry.name().and_then(|name| name.text()))
            .is_some_and(|name| name.eq_ignore_ascii_case("format"))
    })?;
    let expr = entry.expr()?;
    match &expr {
        ast::Expr::Literal(literal) => {
            let format = Format::parse(&literal.string_value()?)?;
            Some((format, expr.range()))
        }
        _ => None,
    }
}

/// What a URI of `uris` names, as its text alone tells it, on any machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UriKind {
    /// A path relative to the project's root, under the root.
    Relative,
    /// A path relative to the project's root whose `..` names go above the root.
    AboveRoot,
    /// An absolute path of Unix, as `/data/sales.csv`, or of Windows, as `C:/data/sales.csv` or
    /// `\data\sales.csv`.
    Absolute,
    /// A `file:` URL.
    FileUrl,
    /// A URL of another scheme, such as `https:`, which names no file of the machine.
    Url,
}

/// What the URI `uri` names. A path separates its names with `/` or `\`, as on Windows, and `..`
/// names the parent directory. A scheme is a letter and then letters, digits, `+`, `-`, and `.`,
/// before a `:`, and a single letter before `:` is the drive of a Windows path.
pub fn uri_kind(uri: &str) -> UriKind {
    let bytes = uri.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if drive || uri.starts_with(['/', '\\']) {
        return UriKind::Absolute;
    }
    if let Some((scheme, _)) = uri.split_once(':') {
        let is_scheme = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && (scheme.chars()).all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        if is_scheme {
            return match scheme.eq_ignore_ascii_case("file") {
                true => UriKind::FileUrl,
                false => UriKind::Url,
            };
        }
    }
    let mut depth: usize = 0;
    for name in uri.split(['/', '\\']) {
        match name {
            "" | "." => {}
            ".." => match depth.checked_sub(1) {
                Some(parent) => depth = parent,
                None => return UriKind::AboveRoot,
            },
            _ => depth += 1,
        }
    }
    UriKind::Relative
}

/// The value of an option: a literal, or an array of string literals.
#[derive(Clone, Debug)]
enum Value {
    String(String),
    Int(i64),
    Bool(bool),
    Strings(Vec<(String, TextRange)>),
    Null,
}

/// An option of a statement, with its value of its type.
#[derive(Clone, Debug)]
struct Entry {
    option: &'static FileOption,
    name: TextRange,
    value: Value,
    value_range: TextRange,
}

/// The value of `expr`, or the GoogleSQL type of a literal that is not a value of an option.
fn value(expr: &ast::Expr) -> Result<Value, Option<String>> {
    match expr {
        ast::Expr::Literal(literal) => match literal.kind() {
            Some(LiteralKind::String) => literal.string_value().map(Value::String).ok_or(None),
            Some(LiteralKind::Int) => (literal.int_value())
                .and_then(|value| i64::try_from(value).ok())
                .map(Value::Int)
                .ok_or(None),
            Some(LiteralKind::True) => Ok(Value::Bool(true)),
            Some(LiteralKind::False) => Ok(Value::Bool(false)),
            Some(LiteralKind::Null) => Ok(Value::Null),
            Some(LiteralKind::Float) => Err(Some("FLOAT64".to_string())),
            Some(LiteralKind::Bytes) => Err(Some("BYTES".to_string())),
            None => Err(None),
        },
        ast::Expr::PrefixExpr(prefix) if prefix.operator() == Some(PrefixOp::Neg) => {
            match prefix.expr().as_ref().map(value) {
                Some(Ok(Value::Int(value))) => Ok(Value::Int(-value)),
                Some(Err(found)) => Err(found),
                _ => Err(None),
            }
        }
        ast::Expr::ArrayExpr(array) => {
            let mut strings = Vec::new();
            for element in array.exprs() {
                match value(&element) {
                    Ok(Value::String(text)) => strings.push((text, element.range())),
                    Ok(Value::Int(_)) => return Err(Some("ARRAY<INT64>".to_string())),
                    Ok(Value::Bool(_)) => return Err(Some("ARRAY<BOOL>".to_string())),
                    _ => return Err(None),
                }
            }
            Ok(Value::Strings(strings))
        }
        _ => Err(None),
    }
}

/// Whether `value` is a value of the type `ty`.
fn of_type(value: &Value, ty: OptionType) -> bool {
    matches!(
        (value, ty),
        (Value::Null, _)
            | (Value::String(_), OptionType::String)
            | (Value::Int(_), OptionType::Int64)
            | (Value::Bool(_), OptionType::Bool)
            | (Value::Strings(_), OptionType::StringArray)
    )
}

/// The GoogleSQL type of `value`.
fn type_name(value: &Value) -> &'static str {
    match value {
        Value::String(_) => "STRING",
        Value::Int(_) => "INT64",
        Value::Bool(_) => "BOOL",
        Value::Strings(_) => "ARRAY<STRING>",
        Value::Null => "NULL",
    }
}

/// The name of the option of `places` that is closest to `name`, when one is close.
fn closest(name: &str, places: &[OptionPlace]) -> Option<&'static str> {
    let options = FILE_OPTIONS
        .iter()
        .filter(|option| places.contains(&option.place));
    closest_name(name, options.map(|option| option.name))
}

/// The options of one list, by their names, with the errors of the list.
struct Reader<'a> {
    diagnostics: &'a mut Vec<Diagnostic>,
    /// The options whose values have the options' types.
    entries: Vec<Entry>,
    /// The options that the lists set, with values of their types or not.
    set: Vec<&'static str>,
}

impl<'a> Reader<'a> {
    /// A reader of no lists yet, whose errors go to `diagnostics`.
    fn new(diagnostics: &'a mut Vec<Diagnostic>) -> Self {
        Reader {
            diagnostics,
            entries: Vec::new(),
            set: Vec::new(),
        }
    }

    fn error(&mut self, range: TextRange, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            range,
            message: message.into(),
            code: None,
        });
    }

    /// Reads the entries of the list of options `list`, which takes the options of its places,
    /// as [`option_places`] gives them, as the options of `what`.
    fn read(&mut self, list: &SyntaxNode, what: &str) {
        let Some(places) = option_places(list) else {
            return;
        };
        for entry in list.children().filter_map(ast::OptionEntry::cast) {
            let (Some(name), Some(expr)) = (entry.name(), entry.expr()) else {
                continue;
            };
            let Some(text) = name.text() else {
                continue;
            };
            let name_range = name.range();
            let Some(option) = file_option(&text) else {
                match unsupported_option(&text) {
                    Some(unsupported) => {
                        let accepted = value(&expr).is_ok_and(|value| {
                            (unsupported.accepts.iter()).any(|accepted| accepted.matches(&value))
                        });
                        if !accepted {
                            self.error(
                                name_range,
                                format!(
                                    "avenger-sql doesn't support BigQuery's option {}: {}",
                                    unsupported.name, unsupported.reason
                                ),
                            );
                        }
                    }
                    None => {
                        let message =
                            did_you_mean(format!("Unknown option {text}"), closest(&text, places));
                        self.error(name_range, message);
                    }
                }
                continue;
            };
            if !places.contains(&option.place) {
                let message = match option.place {
                    OptionPlace::Table => format!(
                        "{} is an option of the table, which goes in the OPTIONS (...) before FROM FILES",
                        option.name
                    ),
                    OptionPlace::Files
                        if list.parent().map(|parent| parent.kind())
                            == Some(SyntaxKind::LOAD_DATA_STMT) =>
                    {
                        format!(
                            "{} is an option of files, which goes in the FROM FILES (...) after OPTIONS",
                            option.name
                        )
                    }
                    OptionPlace::Files => format!(
                        "{} is an option of files, which {what} doesn't read",
                        option.name
                    ),
                    OptionPlace::Selection => {
                        format!("{} is an option of a DECLARE ... SELECTION", option.name)
                    }
                };
                self.error(name_range, message);
                continue;
            }
            if self.set.contains(&option.name) {
                self.error(name_range, format!("Option {} is set twice", option.name));
                continue;
            }
            self.set.push(option.name);
            let value_range = expr.range();
            let value = match value(&expr) {
                Ok(value) if of_type(&value, option.ty) => value,
                Ok(value) => {
                    let found = type_name(&value);
                    self.error(
                        value_range,
                        format!("Option {} takes {}; found {found}", option.name, option.ty),
                    );
                    continue;
                }
                Err(found) => {
                    let found = found.map_or(String::new(), |found| format!("; found {found}"));
                    self.error(
                        value_range,
                        format!(
                            "Option {} takes a literal of {}{found}",
                            option.name, option.ty
                        ),
                    );
                    continue;
                }
            };
            if let (Value::String(text), false) = (&value, option.values.is_empty()) {
                if !(option.values.iter()).any(|allowed| allowed.eq_ignore_ascii_case(text)) {
                    self.error(
                        value_range,
                        format!(
                            "Unknown {} '{text}': it is one of {}",
                            option.name,
                            option.values.join(", ")
                        ),
                    );
                    continue;
                }
            }
            if let Value::Null = value {
                if option.required {
                    self.error(value_range, format!("Option {} takes a value", option.name));
                }
                continue;
            }
            self.entries.push(Entry {
                option,
                name: name_range,
                value,
                value_range,
            });
        }
    }

    fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.option.name == name)
    }

    fn string(&self, name: &str) -> Option<(String, TextRange)> {
        match self.entry(name)? {
            Entry {
                value: Value::String(text),
                value_range,
                ..
            } => Some((text.clone(), *value_range)),
            _ => None,
        }
    }

    /// The description of the table, which a list of table options gives, and the range of the
    /// option's name.
    fn description(&self) -> Option<(String, TextRange)> {
        let entry = self.entry("description")?;
        let (text, _) = self.string("description")?;
        Some((text, entry.name))
    }
}

/// The description in the `OPTIONS (...)` of a view, a table, or a schema, `what`, which takes
/// the options of a table only, with the errors of the options.
pub(crate) fn table_options(
    options: Option<ast::Options>,
    what: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<String> {
    let mut reader = Reader::new(diagnostics);
    if let Some(options) = options {
        reader.read(options.syntax(), what);
    }
    reader.description().map(|(text, _)| text)
}

/// A column of a column list, and the range of its name.
pub(crate) struct ColumnDef {
    pub field: Field,
    pub range: TextRange,
}

impl ColumnDef {
    /// The column as its name, its type, and the range of its name.
    pub fn typed(self) -> (String, DataType, TextRange) {
        let ColumnDef { field, range } = self;
        (field.name().clone(), field.data_type().clone(), range)
    }
}

/// The columns of `list`, with the errors of their types and of a name that repeats.
pub(crate) fn column_list(
    list: &ast::ColumnList,
    types: TypeOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<ColumnDef> {
    let mut columns: Vec<ColumnDef> = Vec::new();
    for column in list.column_defs() {
        let (Some(name), Some(ty)) = (column.name(), column.ty()) else {
            continue;
        };
        let Some(text) = name.text() else {
            continue;
        };
        let ty = match types::arrow_type(&ty, types) {
            Ok(ty) => ty,
            Err(error) => {
                diagnostics.push(Diagnostic {
                    range: error.range,
                    message: error.message,
                    code: None,
                });
                continue;
            }
        };
        if (columns.iter()).any(|column| column.field.name().eq_ignore_ascii_case(&text)) {
            diagnostics.push(Diagnostic {
                range: name.range(),
                message: format!("Column {text} is named twice"),
                code: None,
            });
            continue;
        }
        let nullable = column.not_kw().is_none();
        columns.push(ColumnDef {
            field: Field::new(text, ty, nullable),
            range: name.range(),
        });
    }
    columns
}

/// The options of the files that a statement reads, and of its table.
pub(crate) struct FileStatement {
    /// The list of the files' options: an external table's `OPTIONS (...)`, which holds the
    /// options of its table too, or the `FROM FILES (...)` of a `LOAD DATA`.
    pub files: SyntaxNode,
    /// The `OPTIONS (...)` of the table of a `LOAD DATA`.
    pub table: Option<SyntaxNode>,
    /// The range of the list of the files' options, where an option that is missing goes.
    pub at: TextRange,
    pub columns: Option<ast::ColumnList>,
    pub partitions: Option<ast::PartitionColumns>,
}

impl FileStatement {
    pub(crate) fn external(statement: &ast::CreateExternalTableStmt) -> Option<Self> {
        let options = statement.options()?;
        Some(FileStatement {
            files: options.syntax().clone(),
            table: None,
            at: options.range(),
            columns: statement.column_list(),
            partitions: statement.partition_columns(),
        })
    }

    pub(crate) fn load(statement: &ast::LoadDataStmt) -> Option<Self> {
        let from = statement.from_files()?;
        Some(FileStatement {
            files: from.syntax().clone(),
            table: statement.options().map(|options| options.syntax().clone()),
            at: from.range(),
            columns: statement.column_list(),
            partitions: statement.partition_columns(),
        })
    }
}

/// What the options of a statement that reads files give.
#[derive(Default)]
pub(crate) struct FileOptions {
    /// The files, when the options have no errors.
    pub files: Option<Files>,
    /// The range of the value of `uris`, where an error of reading the files goes.
    pub uris: Option<TextRange>,
    /// The `description` of the table, and the range of the option's name.
    pub description: Option<(String, TextRange)>,
}

/// The files that `statement` reads, with the errors of its options, its column list and its
/// partition columns.
pub(crate) fn file_options(
    statement: &FileStatement,
    types: TypeOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> FileOptions {
    let errors = diagnostics.len();
    let mut reader = Reader::new(diagnostics);
    reader.read(&statement.files, "the table");
    if let Some(table) = &statement.table {
        reader.read(table, "a table");
    }
    let description = reader.description();
    let format = match option_format(&statement.files) {
        Some((format, range)) if !format.available() => {
            reader.error(
                range,
                format!(
                    "This build of avenger-sql reads no {} files: it lacks the cargo feature {}",
                    format.name(),
                    format.feature().unwrap_or_default()
                ),
            );
            None
        }
        Some((format, _)) => Some(format),
        None => {
            if !reader.set.contains(&"format") {
                reader.error(
                    statement.at,
                    "The files take a format, as in format = 'CSV'",
                );
            }
            None
        }
    };
    let uris_range = reader.entry("uris").map(|entry| entry.value_range);
    let uris = match reader.entry("uris").map(|entry| entry.value.clone()) {
        Some(Value::Strings(uris)) => {
            if uris.is_empty() {
                reader.error(
                    uris_range.unwrap_or(statement.at),
                    "uris takes a URI or more",
                );
            }
            for (uri, range) in &uris {
                check_uri(&mut reader, uri, *range);
            }
            uris
        }
        _ => {
            if !reader.set.contains(&"uris") {
                reader.error(
                    statement.at,
                    "The files take uris, as in uris = ['data/files/sales.csv']",
                );
            }
            Vec::new()
        }
    };
    if let Some(format) = format {
        for entry in &reader.entries {
            if !entry.option.formats.contains(&format) {
                let formats: Vec<&str> = (entry.option.formats.iter())
                    .map(|format| format.name())
                    .collect();
                let message = format!(
                    "Option {} applies to {} files, not {}",
                    entry.option.name,
                    formats.join(" and "),
                    format.name()
                );
                reader.diagnostics.push(Diagnostic {
                    range: entry.name,
                    message,
                    code: None,
                });
            }
        }
    }
    let columns =
        (statement.columns.as_ref()).map(|list| column_list(list, types, reader.diagnostics));
    let skip = match reader
        .entry("skip_leading_rows")
        .map(|e| (e.value.clone(), e.value_range))
    {
        Some((Value::Int(skip), range)) => {
            if skip < 0 {
                reader.error(range, "skip_leading_rows is 0 or more");
            } else if skip > 1 {
                reader.error(
                    range,
                    "avenger-sql doesn't support skip_leading_rows above 1: DataFusion's CSV reader skips a header and no other rows",
                );
            } else if skip == 1 && columns.is_none() {
                reader.error(
                    range,
                    "Without a column list, the first row names the columns, and avenger-sql skips no row before it: skip_leading_rows = 1 skips a header that a column list names in its place",
                );
            }
            skip
        }
        _ => 0,
    };
    let character = |reader: &mut Reader, name: &str, default: u8| -> u8 {
        let Some((text, range)) = reader.string(name) else {
            return default;
        };
        match text.as_bytes() {
            [byte] if byte.is_ascii() => *byte,
            _ => {
                reader.error(range, format!("{name} is one ASCII character"));
                default
            }
        }
    };
    let delimiter = character(&mut reader, "field_delimiter", b',');
    let quote = character(&mut reader, "quote", b'"');
    let compressible = format.is_none_or(|format| matches!(format, Format::Csv | Format::Json));
    let compression = match reader.string("compression").filter(|_| compressible) {
        Some((text, range)) => match Compression::parse(&text) {
            Some(compression) if !compression.available() => {
                reader.error(
                    range,
                    format!(
                        "This build of avenger-sql reads no {} files: it lacks the cargo feature compression",
                        compression.name()
                    ),
                );
                None
            }
            compression => compression,
        },
        None => None,
    };
    let prefix = reader.string("hive_partition_uri_prefix");
    if let Some((prefix, range)) = &prefix {
        // A prefix that names no directory has its own error, and not one at each URI.
        let names_directory = !prefix.trim_matches(['/', '\\']).is_empty();
        if !names_directory {
            reader.error(
                *range,
                "hive_partition_uri_prefix is the path of the directory of a Hive layout, as 'sales/', and not empty or a separator",
            );
        }
        for (uri, uri_range) in &uris {
            if names_directory && !uri.starts_with(prefix.as_str()) {
                reader.error(
                    *uri_range,
                    format!("The URI doesn't start with the hive_partition_uri_prefix '{prefix}'"),
                );
            }
        }
        if statement.partitions.is_none() {
            reader.error(
                *range,
                "hive_partition_uri_prefix goes with WITH PARTITION COLUMNS, which reads the keys after it",
            );
        }
    }
    let partitions = statement.partitions.as_ref().map(|clause| {
        if prefix.is_none() {
            reader.error(
                clause.range(),
                "WITH PARTITION COLUMNS reads the keys of the paths after the option hive_partition_uri_prefix, which the files don't set",
            );
        }
        match clause.column_list() {
            None => Partitions::Inferred,
            Some(list) => {
                let keys = column_list(&list, types, reader.diagnostics);
                for key in &keys {
                    let named = (columns.iter().flatten())
                        .any(|column| column.field.name().eq_ignore_ascii_case(key.field.name()));
                    if named {
                        reader.error(
                            key.range,
                            format!(
                                "Column {} is both a column of the files and a partition column",
                                key.field.name()
                            ),
                        );
                    }
                }
                Partitions::Columns(fields(&keys))
            }
        }
    });
    // DataFusion 55's CSV reader applies `null_regex` while it infers a schema and not while it
    // reads rows, where an empty field is null.
    if let Some((_, range)) = (reader.string("null_marker")).filter(|(text, _)| !text.is_empty()) {
        reader.error(
            range,
            "avenger-sql doesn't support null_marker other than '': DataFusion's CSV reader reads an empty field as null, and no other text",
        );
    }
    let quoted_newlines = matches!(
        reader
            .entry("allow_quoted_newlines")
            .map(|entry| &entry.value),
        Some(Value::Bool(true))
    );
    let files = match (format, diagnostics.len() == errors) {
        (Some(format), true) => Some(Files {
            format,
            uris: uris.into_iter().map(|(uri, _)| uri).collect(),
            csv: (format == Format::Csv).then(|| CsvOptions {
                header: columns.is_none() || skip == 1,
                delimiter,
                quote,
                quoted_newlines,
            }),
            columns: columns.as_deref().map(fields),
            compression,
            hive_partition_uri_prefix: prefix.map(|(prefix, _)| prefix),
            partitions,
        }),
        _ => None,
    };
    FileOptions {
        files,
        uris: uris_range,
        description,
    }
}

fn fields(columns: &[ColumnDef]) -> Fields {
    (columns.iter())
        .map(|column| column.field.clone())
        .collect::<Vec<Field>>()
        .into()
}

/// Checks a URI of `uris`: it holds one `*` at most, which no `/` or `\` follows, since it
/// matches the names of the files of one directory, and it is not a URL of a scheme other than
/// `file:`. A loader resolves a relative path against the project root, and decides whether it
/// reads an absolute path, a path above the root, or a `file:` URL.
fn check_uri(reader: &mut Reader, uri: &str, range: TextRange) {
    if uri.matches('*').count() > 1 {
        reader.error(range, "A URI holds one * at most");
    } else if (uri.find('*')).is_some_and(|star| uri[star..].contains(['/', '\\'])) {
        reader.error(
            range,
            "A * matches the names of the files of one directory, so no / or \\ follows it",
        );
    }
    if uri_kind(uri) == UriKind::Url {
        reader.error(
            range,
            "A data script reads local files, so a URI is a path or a file: URL, and not a URL of another scheme",
        );
    }
}
