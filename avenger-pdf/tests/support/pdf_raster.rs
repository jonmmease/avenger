//! PDFium raster checks use a pinned library installed by CI.
use pdfium_render::prelude::{PdfRenderConfig, Pdfium, PdfiumError};
use std::sync::Mutex;

static PDFIUM_LOCK: Mutex<()> = Mutex::new(());

pub fn pdf_to_png(bytes: &[u8], width: f32, height: f32) -> image::RgbaImage {
    let _guard = PDFIUM_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let path = std::env::var_os("AVENGER_PDFIUM_LIBRARY_PATH")
        .expect("set AVENGER_PDFIUM_LIBRARY_PATH to PDFium 7763; see avenger-pdf/README.md");
    let pdfium = match Pdfium::bind_to_library(path) {
        Ok(bindings) => Pdfium::new(bindings),
        Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Pdfium::default(),
        Err(error) => panic!("load pinned PDFium library: {error}"),
    };
    let document = pdfium.load_pdf_from_byte_slice(bytes, None).unwrap();
    assert_eq!(document.pages().len(), 1);
    let page = document.pages().get(0).unwrap();
    assert!((page.width().value - width).abs() < 0.01);
    assert!((page.height().value - height).abs() < 0.01);
    let png = page
        .render_with_config(
            &PdfRenderConfig::new()
                .set_fixed_size((width * 2.0).ceil() as i32, (height * 2.0).ceil() as i32),
        )
        .unwrap()
        .as_image()
        .unwrap()
        .to_rgba8();
    png
}
