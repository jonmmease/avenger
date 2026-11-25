//! The fonts of the library: the font files that a page adds, each one TrueType, OpenType, or
//! collection font. The module embeds no fonts, so a page adds the faces that `avenger-fonts`
//! bundles first, in their order, and its own fonts after them.

use std::sync::Arc;

use avenger_format_datetime_googlesql::GoogleSqlDateTimeFormatProvider;
use avenger_format_number_googlesql::GoogleSqlNumberFormatProvider;
use avenger_typst_label::{EngineOptions, FontOptions, LabelEngine, RegisteredFont};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum FontsError {
    #[error("the file is not a font")]
    NotAFont,
}

/// Each face of the font `file`: one for a font, and each face of a collection.
pub fn read_fonts(file: &[u8]) -> Result<Vec<RegisteredFont>, FontsError> {
    let data: Arc<[u8]> = Arc::from(file);
    let count = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
    (0..count)
        .map(|index| {
            ttf_parser::Face::parse(&data, index).map_err(|_| FontsError::NotAFont)?;
            Ok(RegisteredFont::new(data.clone()).with_face_index(index))
        })
        .collect()
}

/// The font options of the fonts that the page added, `added`, with the families of the bundled
/// fonts as the defaults, and without the system's fonts, which a page cannot read.
pub fn font_options(added: Vec<RegisteredFont>) -> FontOptions {
    FontOptions {
        load_system_fonts: false,
        ..FontOptions::bundled_families(added)
    }
}

/// The label engine of the fonts that the page added, `added`, with GoogleSQL's number and date
/// formats, whose syntaxes the charts of avenger-sql name.
pub fn label_engine(added: Vec<RegisteredFont>) -> LabelEngine {
    LabelEngine::new(EngineOptions {
        fonts: font_options(added),
    })
    .with_number_formatting(Arc::new(GoogleSqlNumberFormatProvider::new()))
    .with_datetime_formatting(Arc::new(GoogleSqlDateTimeFormatProvider::new()))
}
