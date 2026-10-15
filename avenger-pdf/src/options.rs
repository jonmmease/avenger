/// PDF output options.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfRenderOptions {
    /// The page's background color, or `None` for a transparent page. White by default.
    pub background: Option<[f32; 4]>,
}

impl Default for PdfRenderOptions {
    fn default() -> Self {
        Self {
            background: Some([1.0; 4]),
        }
    }
}
