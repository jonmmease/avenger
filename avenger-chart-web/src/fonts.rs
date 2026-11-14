//! The fonts of the library: the fonts that `avenger-fonts` bundles, which the module embeds, and
//! the font files that a page loads after them, each one TrueType, OpenType, or collection font.

use std::sync::Arc;

use avenger_typst_label::{bundled_font_options, FontOptions, RegisteredFont};

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

/// The font options of the bundled fonts and then `loaded`, with the bundled families as the
/// defaults, and without the system's fonts, which a page cannot read.
pub fn font_options(loaded: Vec<RegisteredFont>) -> FontOptions {
    let mut options = bundled_font_options();
    options.load_system_fonts = false;
    options.registered_fonts.extend(loaded);
    options
}
