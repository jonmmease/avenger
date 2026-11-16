//! The analysis of the editor's text: a `Document` of avenger-sql-language over a catalog of the
//! session's tables, which every feature of the editor reads, from the colors of its tokens to its
//! completions.

use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Schema};
use avenger_sql::{analysis::Options, catalog::MemoryCatalog, library::Library};
use avenger_sql_language::Document;

use crate::{samples::Sample, sources::Source};

/// The catalog of the tables that the session holds: the user's files by their names, and the
/// sample tables in the schema `samples`.
pub fn catalog(sources: &[Source], samples: &[Sample]) -> MemoryCatalog {
    let schema = |columns: &[(String, DataType)]| {
        let fields: Vec<Field> = (columns.iter())
            .map(|(name, ty)| Field::new(name, ty.clone(), true))
            .collect();
        Arc::new(Schema::new(fields))
    };
    let mut catalog = MemoryCatalog::new();
    for source in sources {
        catalog = catalog.with_schema(&[source.name.as_str()], schema(&source.columns));
    }
    for sample in samples {
        catalog = catalog.with_schema(&["samples", sample.name.as_str()], schema(&sample.columns));
    }
    catalog
}

/// The document of the editor's text, which the editor makes again after each edit, and the
/// catalog and library that it reads.
pub struct Analyzed {
    catalog: Arc<MemoryCatalog>,
    library: Arc<Library>,
    document: Option<Document>,
}

impl Default for Analyzed {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzed {
    pub fn new() -> Self {
        Self {
            catalog: Arc::new(MemoryCatalog::new()),
            library: Arc::new(Library::new()),
            document: None,
        }
    }

    /// Reads the tables of `catalog` from now on.
    pub fn set_catalog(&mut self, catalog: MemoryCatalog) {
        self.catalog = Arc::new(catalog);
        self.document = None;
    }

    /// The document of `text`, analyzed again when the text changed.
    pub fn document(&mut self, text: &str) -> &Document {
        let stale = self
            .document
            .as_ref()
            .is_none_or(|document| document.text() != text);
        if stale {
            self.document = Some(Document::new(
                text,
                self.catalog.clone(),
                self.library.clone(),
                Options::default(),
            ));
        }
        self.document.as_ref().expect("a document")
    }
}
