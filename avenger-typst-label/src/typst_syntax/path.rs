//! File identifiers for spans.
//!
//! Upstream `crates/typst-syntax/src/path.rs` interns rooted virtual paths into
//! `FileId`s. Labels are parsed from in-memory strings, so Avenger keeps only
//! the 16-bit id that `Span` packs into its high bits, with upstream's
//! `from_raw`/`into_raw` API. Ids are never resolved to paths.

use std::fmt::{self, Debug, Formatter};
use std::num::NonZeroU16;

/// An opaque file identifier packed into spans.
#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub struct FileId(NonZeroU16);

impl FileId {
    /// The id every label parse uses. Spans only need their byte ranges, and the id is never
    /// resolved to a file.
    pub const LABEL: Self = Self(NonZeroU16::MIN);

    /// Construct from a raw number.
    pub const fn from_raw(v: NonZeroU16) -> Self {
        Self(v)
    }

    /// Extract the raw underlying number.
    pub const fn into_raw(self) -> NonZeroU16 {
        self.0
    }
}

impl Debug for FileId {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "FileId({})", self.0)
    }
}
