/// PDF output options.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfRenderOptions {
    /// The page's background color, or `None` for a transparent page. White by default.
    pub background: Option<[f32; 4]>,
    /// Draw an image resource that isn't ready as its placeholder, or as nothing without one,
    /// instead of failing the export with the URLs that aren't ready. Disabled by default.
    pub allow_unready_images: bool,
}

impl Default for PdfRenderOptions {
    fn default() -> Self {
        Self {
            background: Some([1.0; 4]),
            allow_unready_images: false,
        }
    }
}
