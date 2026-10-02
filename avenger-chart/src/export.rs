use crate::{error, RenderedChart, Result};
impl RenderedChart {
    /// Export this frame as SVG with its measurement text engine.
    #[cfg(feature = "svg")]
    pub fn to_svg(&self) -> Result<String> {
        avenger_svg::SvgRenderer::new()
            .with_text_engine(self.text.clone())
            .render_scene_graph(&self.scene)
            .map_err(error)
    }
    /// Export this frame as a PDF document with the same text engine.
    #[cfg(feature = "pdf")]
    pub fn to_pdf(&self) -> Result<Vec<u8>> {
        avenger_pdf::PdfRenderer::new()
            .with_text_engine(self.text.clone())
            .render_scene_graph(&self.scene)
            .map_err(error)
    }
    /// Render this frame to PNG at a positive logical-to-physical pixel ratio.
    #[cfg(feature = "png")]
    pub async fn to_png(&self, scale: f32) -> Result<Vec<u8>> {
        use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
        if !scale.is_finite() || scale <= 0.0 {
            return Err(error("PNG scale must be finite and positive"));
        }
        let mut canvas = PngCanvas::new(
            avenger_common::canvas::CanvasDimensions {
                size: [self.scene.width, self.scene.height],
                scale,
            },
            CanvasConfig {
                text_engine: Some(self.text.clone()),
                ..Default::default()
            },
        )
        .await
        .map_err(error)?;
        canvas.set_scene(&self.scene).map_err(error)?;
        let image = canvas.render().await.map_err(error)?;
        let mut bytes = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, image::ImageFormat::Png)
            .map_err(error)?;
        Ok(bytes.into_inner())
    }
}
