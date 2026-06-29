pub fn build_fontdb(options: &crate::FontResolutionOptions) -> fontdb::Database {
    let mut fontdb = fontdb::Database::new();
    for font in &options.registered_fonts {
        fontdb.load_font_data(font.data.to_vec());
    }

    if let Some(family) = &options.default_sans_serif_family {
        fontdb.set_sans_serif_family(family);
    }
    if let Some(family) = &options.default_monospace_family {
        fontdb.set_monospace_family(family);
    }

    if options.load_system_fonts {
        fontdb.load_system_fonts();
    }

    for font_dir in &options.extra_font_dirs {
        fontdb.load_fonts_dir(font_dir);
    }

    fontdb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_fontdb_uses_registered_font_defaults() {
        let font_data =
            include_bytes!("../../avenger-vega-test-data/fonts/Caveat/static/Caveat-Regular.ttf");
        let options = crate::FontResolutionOptions {
            registered_fonts: vec![avenger_typst_label::RegisteredFont::new(
                avenger_typst_label::MathFontBytesId(1),
                font_data.as_slice(),
            )],
            default_sans_serif_family: Some("Caveat".to_string()),
            ..Default::default()
        };
        let fontdb = build_fontdb(&options);

        let families = [fontdb::Family::SansSerif];
        let query = fontdb::Query {
            families: &families,
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        let sans_id = fontdb.query(&query).expect("sans-serif should resolve");
        let sans_face = fontdb.face(sans_id).expect("sans-serif face should exist");
        assert!(sans_face
            .families
            .iter()
            .any(|(family, _)| family == "Caveat"));
    }
}
