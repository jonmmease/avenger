use std::{collections::HashMap, sync::Arc};

use tower_lsp_server::ls_types::{TextDocumentContentChangeEvent, TextDocumentItem, Uri};

use crate::position::{PositionEncoding, PositionError, PositionIndex};

#[derive(Clone, Debug)]
pub(crate) struct OpenDocument {
    pub(crate) uri: Uri,
    pub(crate) language_id: String,
    pub(crate) version: i32,
    pub(crate) text: Arc<str>,
    pub(crate) positions: PositionIndex,
}

#[derive(Debug, Default)]
pub(crate) struct DocumentStore {
    documents: HashMap<Uri, OpenDocument>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DocumentError {
    #[error("document is not open: {0}")]
    NotOpen(String),
    #[error("document is already open: {0}")]
    AlreadyOpen(String),
    #[error("stale document version {incoming}; current version is {current}")]
    StaleVersion { incoming: i32, current: i32 },
    #[error("invalid document change: {0}")]
    InvalidChange(#[from] PositionError),
    #[error("rangeLength {provided} does not match replaced length {actual}")]
    RangeLengthMismatch { provided: u32, actual: u32 },
    #[error("document size {actual} bytes exceeds configured limit {limit} bytes")]
    TooLarge { actual: usize, limit: usize },
}

impl DocumentStore {
    pub(crate) fn open(
        &mut self,
        item: TextDocumentItem,
        encoding: PositionEncoding,
        max_bytes: usize,
    ) -> Result<OpenDocument, DocumentError> {
        if self.documents.contains_key(&item.uri) {
            return Err(DocumentError::AlreadyOpen(item.uri.as_str().to_owned()));
        }
        if item.text.len() > max_bytes {
            return Err(DocumentError::TooLarge {
                actual: item.text.len(),
                limit: max_bytes,
            });
        }
        let text: Arc<str> = item.text.into();
        let document = OpenDocument {
            uri: item.uri.clone(),
            language_id: item.language_id,
            version: item.version,
            positions: PositionIndex::new(text.clone(), encoding),
            text,
        };
        self.documents.insert(item.uri, document.clone());
        Ok(document)
    }

    pub(crate) fn change(
        &mut self,
        uri: &Uri,
        version: i32,
        changes: &[TextDocumentContentChangeEvent],
        encoding: PositionEncoding,
        max_bytes: usize,
    ) -> Result<OpenDocument, DocumentError> {
        let current = self
            .documents
            .get(uri)
            .ok_or_else(|| DocumentError::NotOpen(uri.as_str().to_owned()))?;
        if version <= current.version {
            return Err(DocumentError::StaleVersion {
                incoming: version,
                current: current.version,
            });
        }

        // Apply into a temporary value so a malformed later edit cannot corrupt
        // the last valid snapshot.
        let mut text = current.text.to_string();
        for change in changes {
            if let Some(range) = change.range {
                let index = PositionIndex::new(Arc::from(text.as_str()), encoding);
                let bytes = index.byte_range(range)?;
                if let Some(provided) = change.range_length {
                    let replaced = &text[bytes.clone()];
                    let actual = match encoding {
                        PositionEncoding::Utf8 => replaced.len(),
                        PositionEncoding::Utf16 => replaced.encode_utf16().count(),
                    } as u32;
                    if provided != actual {
                        return Err(DocumentError::RangeLengthMismatch { provided, actual });
                    }
                }
                text.replace_range(bytes, &change.text);
            } else {
                text = change.text.clone();
            }
        }
        if text.len() > max_bytes {
            return Err(DocumentError::TooLarge {
                actual: text.len(),
                limit: max_bytes,
            });
        }
        let text: Arc<str> = text.into();
        let updated = OpenDocument {
            uri: current.uri.clone(),
            language_id: current.language_id.clone(),
            version,
            positions: PositionIndex::new(text.clone(), encoding),
            text,
        };
        self.documents.insert(uri.clone(), updated.clone());
        Ok(updated)
    }

    pub(crate) fn save(
        &mut self,
        uri: &Uri,
        text: Option<String>,
        encoding: PositionEncoding,
        max_bytes: usize,
    ) -> Result<OpenDocument, DocumentError> {
        let current = self
            .documents
            .get(uri)
            .ok_or_else(|| DocumentError::NotOpen(uri.as_str().to_owned()))?;
        let Some(text) = text else {
            return Ok(current.clone());
        };
        if text.len() > max_bytes {
            return Err(DocumentError::TooLarge {
                actual: text.len(),
                limit: max_bytes,
            });
        }
        let text: Arc<str> = text.into();
        let updated = OpenDocument {
            uri: current.uri.clone(),
            language_id: current.language_id.clone(),
            version: current.version,
            positions: PositionIndex::new(text.clone(), encoding),
            text,
        };
        self.documents.insert(uri.clone(), updated.clone());
        Ok(updated)
    }

    pub(crate) fn close(&mut self, uri: &Uri) -> Result<OpenDocument, DocumentError> {
        self.documents
            .remove(uri)
            .ok_or_else(|| DocumentError::NotOpen(uri.as_str().to_owned()))
    }

    pub(crate) fn get(&self, uri: &Uri) -> Option<&OpenDocument> {
        self.documents.get(uri)
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = &OpenDocument> {
        self.documents.values()
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use tower_lsp_server::ls_types::{Position, Range};

    use super::*;

    fn item(text: &str, version: i32) -> TextDocumentItem {
        TextDocumentItem::new(
            Uri::from_str("file:///tmp/chart.avenger").unwrap(),
            "avenger".into(),
            version,
            text.into(),
        )
    }

    #[test]
    fn applies_sequential_incremental_edits() {
        let mut store = DocumentStore::default();
        let doc = store
            .open(item("a😀c", 1), PositionEncoding::Utf16, usize::MAX)
            .unwrap();
        let changes = vec![
            TextDocumentContentChangeEvent {
                range: Some(Range::new(Position::new(0, 1), Position::new(0, 3))),
                range_length: Some(2),
                text: "b".into(),
            },
            TextDocumentContentChangeEvent {
                range: Some(Range::new(Position::new(0, 2), Position::new(0, 3))),
                range_length: Some(1),
                text: "d".into(),
            },
        ];
        let changed = store
            .change(&doc.uri, 2, &changes, PositionEncoding::Utf16, usize::MAX)
            .unwrap();
        assert_eq!(&*changed.text, "abd");
    }

    #[test]
    fn malformed_batch_is_transactional() {
        let mut store = DocumentStore::default();
        let doc = store
            .open(item("abc", 1), PositionEncoding::Utf16, usize::MAX)
            .unwrap();
        let changes = vec![
            TextDocumentContentChangeEvent {
                range: Some(Range::new(Position::new(0, 0), Position::new(0, 1))),
                range_length: None,
                text: "x".into(),
            },
            TextDocumentContentChangeEvent {
                range: Some(Range::new(Position::new(9, 0), Position::new(9, 1))),
                range_length: None,
                text: "y".into(),
            },
        ];
        assert!(
            store
                .change(&doc.uri, 2, &changes, PositionEncoding::Utf16, usize::MAX,)
                .is_err()
        );
        let retained = store.get(&doc.uri).unwrap();
        assert_eq!(retained.version, 1);
        assert_eq!(&*retained.text, "abc");
    }

    #[test]
    fn rejects_stale_versions_and_supports_full_replacement() {
        let mut store = DocumentStore::default();
        let doc = store
            .open(item("old", 5), PositionEncoding::Utf16, usize::MAX)
            .unwrap();
        let replacement = [TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: "new".into(),
        }];
        assert!(matches!(
            store.change(
                &doc.uri,
                5,
                &replacement,
                PositionEncoding::Utf16,
                usize::MAX,
            ),
            Err(DocumentError::StaleVersion { .. })
        ));
        let updated = store
            .change(
                &doc.uri,
                6,
                &replacement,
                PositionEncoding::Utf16,
                usize::MAX,
            )
            .unwrap();
        assert_eq!(&*updated.text, "new");
    }

    #[test]
    fn size_limits_are_transactional_for_open_change_and_save() {
        let mut store = DocumentStore::default();
        assert!(matches!(
            store.open(item("12345", 1), PositionEncoding::Utf8, 4),
            Err(DocumentError::TooLarge { .. })
        ));
        let doc = store
            .open(item("1234", 1), PositionEncoding::Utf8, 4)
            .unwrap();
        let replacement = [TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: "12345".to_owned(),
        }];
        assert!(matches!(
            store.change(&doc.uri, 2, &replacement, PositionEncoding::Utf8, 4),
            Err(DocumentError::TooLarge { .. })
        ));
        assert_eq!(store.get(&doc.uri).unwrap().version, 1);
        assert!(matches!(
            store.save(
                &doc.uri,
                Some("12345".to_owned()),
                PositionEncoding::Utf8,
                4,
            ),
            Err(DocumentError::TooLarge { .. })
        ));
        assert_eq!(&*store.get(&doc.uri).unwrap().text, "1234");
    }

    #[test]
    fn randomized_incremental_edits_match_string_oracle() {
        for encoding in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
            let mut store = DocumentStore::default();
            let doc = store
                .open(item("alpha 😀\r\nbeta 中\ngamma", 1), encoding, usize::MAX)
                .unwrap();
            let uri = doc.uri;
            let mut oracle = doc.text.to_string();
            let mut version = 1;
            let mut random = 0x4d59_5df4_d0f3_3173_u64;
            let replacements = ["", "x", "😀", "\n", "é中"];

            for _ in 0..250 {
                let index = PositionIndex::new(Arc::from(oracle.as_str()), encoding);
                let offsets = oracle
                    .char_indices()
                    .map(|(offset, _)| offset)
                    .chain(std::iter::once(oracle.len()))
                    .filter(|offset| index.position(*offset).is_ok())
                    .collect::<Vec<_>>();
                random = random
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                let left = offsets[(random as usize) % offsets.len()];
                random = random
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                let right = offsets[(random as usize) % offsets.len()];
                let (start, end) = if left <= right {
                    (left, right)
                } else {
                    (right, left)
                };
                random = random
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                let replacement = replacements[(random as usize) % replacements.len()];
                let replaced = &oracle[start..end];
                let range_length = match encoding {
                    PositionEncoding::Utf8 => replaced.len(),
                    PositionEncoding::Utf16 => replaced.encode_utf16().count(),
                } as u32;
                let change = [TextDocumentContentChangeEvent {
                    range: Some(Range::new(
                        index.position(start).unwrap(),
                        index.position(end).unwrap(),
                    )),
                    range_length: Some(range_length),
                    text: replacement.to_owned(),
                }];
                oracle.replace_range(start..end, replacement);
                version += 1;
                let updated = store
                    .change(&uri, version, &change, encoding, usize::MAX)
                    .unwrap();
                assert_eq!(&*updated.text, oracle);
            }
        }
    }
}
