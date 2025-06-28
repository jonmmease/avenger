use lazy_static::lazy_static;
use resvg::render;
use std::panic;
use std::sync::{Arc, Mutex};
use usvg::fontdb::Database;

use crate::error::AvengerImageError;

lazy_static! {
    pub static ref FONT_DB: Mutex<usvg::fontdb::Database> = Mutex::new(init_font_db());
}

fn init_font_db() -> usvg::fontdb::Database {
    let mut font_database = Database::new();
    font_database.load_system_fonts();
    font_database
}

pub fn svg_to_png(svg: &str, scale: f32) -> Result<Vec<u8>, AvengerImageError> {
    // default ppi to 72
    let font_database = FONT_DB.lock().expect("Failed to acquire fontdb lock");

    // catch_unwind so that we don't poison Mutexes
    // if usvg/resvg panics
    let response = panic::catch_unwind(|| {
        let xml_opt = usvg::roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        };
        let opts = usvg::Options {
            fontdb: Arc::new(font_database.clone()),
            ..Default::default()
        };
        let doc = usvg::roxmltree::Document::parse_with_options(svg, xml_opt)?;
        let rtree = usvg::Tree::from_xmltree(&doc, &opts)?;

        let mut pixmap = tiny_skia::Pixmap::new(
            (rtree.size().width() * scale) as u32,
            (rtree.size().height() * scale) as u32,
        )
        .unwrap();

        let transform = tiny_skia::Transform::from_scale(scale, scale);
        render(&rtree, transform, &mut pixmap.as_mut());
        Ok(pixmap.encode_png())
    });
    match response {
        Ok(Ok(Ok(png_result))) => Ok(png_result),
        Ok(Err(err)) => Err(err),
        err => panic!("{err:?}"),
    }
}
