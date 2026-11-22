//! The data files that the editor registers as tables, and the names that it gives them.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use arrow::datatypes::DataType;

/// The format of a data file, from its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Csv,
    Parquet,
}

impl FileKind {
    /// The format of `path`: CSV for `.csv` and Parquet for `.parquet`, in any case. Another
    /// extension has no format.
    pub fn of(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "csv" => Some(Self::Csv),
            "parquet" => Some(Self::Parquet),
            _ => None,
        }
    }

    /// The name of the format, as the data sidebar shows it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Parquet => "parquet",
        }
    }
}

/// A registered file: the name of its table, its path and format, and the names and types of its
/// columns.
#[derive(Clone, Debug)]
pub struct Source {
    pub name: String,
    pub path: PathBuf,
    pub kind: FileKind,
    pub columns: Vec<(String, DataType)>,
}

/// The table name of the file at `path`: the file stem in lowercase, with each character that is
/// not an ASCII letter, an ASCII digit, or `_` replaced by `_`, and with `_` before a leading
/// digit, so that the name is an identifier that needs no quotes. When `taken` holds the name,
/// the name ends with `_2`, `_3`, and so on, the first that `taken` does not hold.
pub fn table_name(path: &Path, taken: &BTreeSet<String>) -> String {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("");
    let mut base: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if base.is_empty() {
        base = "table".to_string();
    }
    if base.starts_with(|c: char| c.is_ascii_digit()) {
        base.insert(0, '_');
    }
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|name| !taken.contains(name))
        .expect("a free name")
}
