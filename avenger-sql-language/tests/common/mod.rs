//! The catalog of the tests, and documents of scripts with a cursor.

#![allow(dead_code)]

use std::sync::Arc;

use avenger_sql::{analysis::Options, catalog::MemoryCatalog, library::Library};
use avenger_sql_language::{Document, Offset, TextRange};
use datafusion::arrow::datatypes::{DataType, Field, Fields, Schema};

/// The tables `penguins` and `islands`. `islands` has a column named by a reserved word,
/// `order`, a struct column, `location`, and a `BOOL` column, `coastal`.
pub fn catalog() -> MemoryCatalog {
    let penguins = Schema::new(vec![
        Field::new("species", DataType::Utf8, true),
        Field::new("island", DataType::Utf8, true),
        Field::new("bill_length_mm", DataType::Float64, true),
        Field::new("body_mass_g", DataType::Int64, true),
    ]);
    let location = Fields::from(vec![
        Field::new("lat", DataType::Float64, true),
        Field::new("lon", DataType::Float64, true),
    ]);
    let islands = Schema::new(vec![
        Field::new("name", DataType::Utf8, true),
        Field::new("order", DataType::Int64, true),
        Field::new("location", DataType::Struct(location), true),
        Field::new("coastal", DataType::Boolean, true),
    ]);
    MemoryCatalog::new()
        .with_schema(&["penguins"], Arc::new(penguins))
        .with_schema(&["islands"], Arc::new(islands))
}

/// The document of `text`.
pub fn document(text: &str) -> Document {
    Document::new(
        text,
        Arc::new(catalog()),
        Arc::new(Library::new()),
        Options::default(),
    )
}

/// The document of `text` without its cursor marker `$0`, and the offset of the marker.
pub fn at_cursor(text: &str) -> (Document, Offset) {
    let offset = text.find("$0").expect("a cursor marker `$0`");
    let text = text.replacen("$0", "", 1);
    (document(&text), Offset::from(offset as u32))
}

/// The text of `range` in `document`.
pub fn at(document: &Document, range: TextRange) -> &str {
    &document.text()[std::ops::Range::<usize>::from(range)]
}

/// The offset of the `n`th occurrence of `needle` in `document`, counting from 0.
pub fn find(document: &Document, needle: &str, n: usize) -> Offset {
    let (start, _) = document.text().match_indices(needle).nth(n).expect(needle);
    Offset::from(start as u32)
}
