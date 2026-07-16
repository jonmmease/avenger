use std::sync::Arc;

use avenger_lang_core::{LineIndex, SourceError, SourceFile, SourceId, SourceMap, SourceOrigin};

#[test]
fn utf8_crlf_tabs_and_eof_have_stable_positions() {
    let text: Arc<str> = "alpha\r\n\tβeta\nlast".into();
    let index = LineIndex::new(text.clone());

    assert_eq!(index.line_count(), 3);
    assert_eq!(index.line_text(0), Some("alpha"));
    assert_eq!(index.line_text(1), Some("\tβeta"));
    assert_eq!(index.line_text(2), Some("last"));

    let beta = text.find('β').unwrap();
    let beta_location = index.location(beta).unwrap();
    assert_eq!(beta_location.line, 1);
    assert_eq!(beta_location.column, 1);
    assert_eq!(beta_location.byte_column, 1);
    assert_eq!(beta_location.display_column, 4);

    let after_beta = beta + 'β'.len_utf8();
    let after_beta_location = index.location(after_beta).unwrap();
    assert_eq!(after_beta_location.column, 2);
    assert_eq!(after_beta_location.byte_column, 3);
    assert_eq!(after_beta_location.display_column, 5);

    let cr = text.find('\r').unwrap();
    assert_eq!(index.location(cr).unwrap().column, 5);
    assert_eq!(index.location(cr + 1).unwrap().column, 5);

    let eof = index.location(text.len()).unwrap();
    assert_eq!((eof.line, eof.column), (2, 4));
}

#[test]
fn positions_reject_non_boundaries_and_out_of_range_offsets() {
    let text: Arc<str> = "aβ".into();
    let index = LineIndex::new(text.clone());

    assert!(matches!(
        index.location(2),
        Err(SourceError::NotCharBoundary { offset: 2 })
    ));
    assert!(matches!(
        index.location(text.len() + 1),
        Err(SourceError::OffsetOutOfBounds { .. })
    ));
}

#[test]
fn duplicate_source_ids_do_not_replace_the_original_source() {
    let id = SourceId::new(4);
    let mut sources = SourceMap::default();
    sources
        .insert(SourceFile::new(
            id,
            SourceOrigin::Memory("first".to_string()),
            "first",
        ))
        .unwrap();
    assert!(matches!(
        sources.insert(SourceFile::new(
            id,
            SourceOrigin::Memory("second".to_string()),
            "second",
        )),
        Err(SourceError::DuplicateSourceId(found)) if found == id
    ));
    assert_eq!(sources.get(id).unwrap().text(), "first");
}
