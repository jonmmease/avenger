/// PDF output options.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfRenderOptions {
    /// The page's background color, or `None` for a transparent page. White by default.
    pub background: Option<[f32; 4]>,
    /// Fail when an image resource isn't ready, instead of drawing its placeholder. Disabled by
    /// default.
    pub strict_images: bool,
}

impl Default for PdfRenderOptions {
    fn default() -> Self {
        Self {
            background: Some([1.0; 4]),
            strict_images: false,
        }
    }
}
