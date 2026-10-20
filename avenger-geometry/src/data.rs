//! The rows behind a scene's marks, which the hit-testing tree pairs with the marks' instances.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use arrow_array::{Array, RecordBatch};
use arrow_ord::ord::make_comparator;
use arrow_schema::SortOptions;
use avenger_scenegraph::marks::mark::SceneMark;

/// The rows behind a scene's marks, by mark name, which a scene builder returns beside its scene.
/// A mark's rows match its instances in order, or a line's vertices.
#[derive(Debug, Clone, Default)]
pub struct SceneData {
    rows: HashMap<String, Arc<RecordBatch>>,
}

impl SceneData {
    pub fn new() -> Self {
        Self::default()
    }

    /// Give the mark named `mark` its `rows`, in place of any rows it had.
    pub fn insert(&mut self, mark: impl Into<String>, rows: Arc<RecordBatch>) {
        self.rows.insert(mark.into(), rows);
    }

    /// The rows of the mark named `mark`, if it has rows.
    pub fn rows(&self, mark: &str) -> Option<&Arc<RecordBatch>> {
        self.rows.get(mark)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &Arc<RecordBatch>)> {
        self.rows.iter().map(|(name, rows)| (name.as_str(), rows))
    }
}

/// One instance's row: the rows of its mark and the instance's index in them.
#[derive(Debug, Clone)]
pub struct Datum {
    pub rows: Arc<RecordBatch>,
    pub row: usize,
}

impl Datum {
    /// The row of `rows`, such as a newer build's rows of the datum's mark, that holds the
    /// datum: its own row when `rows` are the datum's rows, or else the first row whose `key`
    /// column holds the datum's key. `None` for other rows without a key, and when the datum's
    /// key is null or no row holds it.
    pub fn row_in(&self, rows: &Arc<RecordBatch>, key: Option<&str>) -> Option<usize> {
        if Arc::ptr_eq(&self.rows, rows) {
            return Some(self.row);
        }
        let (Some(own), Some(theirs)) = (self.rows.column_by_name(key?), rows.column_by_name(key?))
        else {
            return None;
        };
        if own.is_null(self.row) {
            return None;
        }
        let compare =
            make_comparator(own.as_ref(), theirs.as_ref(), SortOptions::default()).ok()?;
        (0..rows.num_rows())
            .find(|&row| !theirs.is_null(row) && compare(self.row, row) == Ordering::Equal)
    }
}

/// Scene data that doesn't match its scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneDataError {
    /// No mark in the scene has the name.
    NoMark(String),
    /// More than one mark in the scene has the name, so the rows don't say which they belong to.
    SharedName(String),
    /// The rows don't number the mark's instances.
    RowCount {
        mark: String,
        instances: usize,
        rows: usize,
    },
}

impl fmt::Display for SceneDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoMark(mark) => write!(f, "no mark is named {mark:?}"),
            Self::SharedName(mark) => {
                write!(
                    f,
                    "several marks are named {mark:?}, so its rows have no one mark"
                )
            }
            Self::RowCount {
                mark,
                instances,
                rows,
            } => write!(
                f,
                "mark {mark:?} has {instances} instances, and its scene data has {rows} rows"
            ),
        }
    }
}

impl std::error::Error for SceneDataError {}

/// The instances that a mark's rows number: a line's and an area's vertices, and one image for
/// each mesh image instance.
pub(crate) fn instance_count(mark: &SceneMark) -> usize {
    match mark {
        SceneMark::Arc(mark) => mark.len as usize,
        SceneMark::Area(mark) => mark.len as usize,
        SceneMark::Path(mark) => mark.len as usize,
        SceneMark::Symbol(mark) => mark.len as usize,
        SceneMark::Line(mark) => mark.len as usize,
        SceneMark::Trail(mark) => mark.len as usize,
        SceneMark::Rect(mark) => mark.len as usize,
        SceneMark::Rule(mark) => mark.len as usize,
        SceneMark::Text(mark) => mark.len as usize,
        SceneMark::Image(mark) => mark.len as usize,
        SceneMark::MeshImage(mark) => mark.instances.len(),
        SceneMark::Group(_) => 0,
    }
}
