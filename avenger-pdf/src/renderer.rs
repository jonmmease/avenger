use std::path::Path;

use avenger_scenegraph::{
    marks::mark::SceneMark,
    render_order::{SceneDisplayList, SceneDisplayMark},
    scene_graph::SceneGraph,
};
use avenger_svg::{SvgBackground, SvgFontEmbedding, SvgImageMode, SvgRenderOptions, SvgRenderer};
use avenger_text::{
    types::{FontStyle, FontWeight, FontWeightNameSpec},
    MissingFontPolicy,
};

use crate::{
    error::AvengerPdfError,
    options::{PdfBackground, PdfRenderOptions},
};

#[derive(Debug, Clone, Default)]
pub struct PdfRenderer {
    options: PdfRenderOptions,
}

impl PdfRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_options(mut self, options: PdfRenderOptions) -> Self {
        self.options = options;
        self
    }

    pub fn render_scene_graph(&self, scene_graph: &SceneGraph) -> Result<Vec<u8>, AvengerPdfError> {
        let usvg_options = self.usvg_options();
        if matches!(
            self.options.font_resolution.missing_font,
            MissingFontPolicy::Error
        ) {
            validate_scene_graph_text_fonts(scene_graph, usvg_options.fontdb.as_ref())?;
        }

        let svg = self.render_svg_for_pdf(scene_graph)?;
        let tree = svg2pdf::usvg::Tree::from_str(&svg, &usvg_options)?;
        if matches!(
            self.options.font_resolution.missing_font,
            MissingFontPolicy::Error
        ) {
            validate_text_fonts_for_embedding(&tree)?;
        }
        let pdf = svg2pdf::to_pdf(
            &tree,
            svg2pdf::ConversionOptions {
                compress: self.options.compress,
                raster_scale: self.options.raster_scale,
                embed_text: true,
                pdfa: false,
            },
            svg2pdf::PageOptions::default(),
        )
        .map_err(|err| AvengerPdfError::Conversion(err.to_string()))?;
        Ok(pdf)
    }

    pub fn write_scene_graph_pdf<P: AsRef<Path>>(
        &self,
        scene_graph: &SceneGraph,
        output: P,
    ) -> Result<(), AvengerPdfError> {
        let output = output.as_ref();
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(output, self.render_scene_graph(scene_graph)?)?;
        Ok(())
    }

    fn render_svg_for_pdf(&self, scene_graph: &SceneGraph) -> Result<String, AvengerPdfError> {
        Ok(SvgRenderer::new()
            .with_options(self.svg_options())
            .render_scene_graph(scene_graph)?)
    }

    fn svg_options(&self) -> SvgRenderOptions {
        SvgRenderOptions {
            background: match self.options.background {
                PdfBackground::White => SvgBackground::White,
                PdfBackground::Transparent => SvgBackground::Transparent,
                PdfBackground::Color(color) => SvgBackground::Color(color),
            },
            precision: self.options.precision,
            image_mode: SvgImageMode::EmbedPngDataUris,
            font_resolution: self.options.font_resolution.clone(),
            font_embedding: SvgFontEmbedding::None,
            include_metadata: false,
        }
    }

    fn usvg_options(&self) -> svg2pdf::usvg::Options<'static> {
        let mut options = svg2pdf::usvg::Options::default();
        options.fontdb = std::sync::Arc::new(avenger_text::fonts::build_fontdb(
            &self.options.font_resolution,
        ));
        options
    }
}

fn validate_scene_graph_text_fonts(
    scene_graph: &SceneGraph,
    fontdb: &svg2pdf::usvg::fontdb::Database,
) -> Result<(), AvengerPdfError> {
    let display_list = SceneDisplayList::from_scene_graph(scene_graph);
    for item in display_list.ordered_items() {
        let SceneDisplayMark::Borrowed(SceneMark::Text(mark)) = &item.mark else {
            continue;
        };

        for (((text, font), font_weight), font_style) in mark
            .text_iter()
            .zip(mark.font_iter())
            .zip(mark.font_weight_iter())
            .zip(mark.font_style_iter())
        {
            if text.chars().all(char::is_whitespace) {
                continue;
            }

            let family = font.trim();
            if family.is_empty() || is_generic_font_family(family) {
                continue;
            }

            if !fontdb_has_scene_family(fontdb, family, font_weight, font_style) {
                return Err(AvengerPdfError::Font(format!(
                    "missing requested text font family {family}"
                )));
            }
        }
    }

    Ok(())
}

fn is_generic_font_family(family: &str) -> bool {
    matches!(
        family,
        "serif" | "sans-serif" | "cursive" | "fantasy" | "monospace"
    )
}

fn fontdb_has_scene_family(
    fontdb: &svg2pdf::usvg::fontdb::Database,
    family: &str,
    font_weight: &FontWeight,
    font_style: &FontStyle,
) -> bool {
    let families = [svg2pdf::usvg::fontdb::Family::Name(family)];
    let query = svg2pdf::usvg::fontdb::Query {
        families: &families,
        weight: svg2pdf::usvg::fontdb::Weight(font_weight_number(font_weight)),
        stretch: svg2pdf::usvg::fontdb::Stretch::Normal,
        style: scene_font_style(font_style),
    };
    fontdb.query(&query).is_some()
}

fn validate_text_fonts_for_embedding(tree: &svg2pdf::usvg::Tree) -> Result<(), AvengerPdfError> {
    let mut error = None;
    validate_group_text_fonts(tree.root(), tree.fontdb(), &mut error);
    if let Some(error) = error {
        return Err(error);
    }
    Ok(())
}

fn validate_group_text_fonts(
    group: &svg2pdf::usvg::Group,
    fontdb: &svg2pdf::usvg::fontdb::Database,
    error: &mut Option<AvengerPdfError>,
) {
    if error.is_some() {
        return;
    }

    for node in group.children() {
        match node {
            svg2pdf::usvg::Node::Group(group) => validate_group_text_fonts(group, fontdb, error),
            svg2pdf::usvg::Node::Text(text) => {
                if let Some(message) = text_font_embedding_error(text, fontdb) {
                    *error = Some(AvengerPdfError::Font(message));
                    return;
                }
            }
            _ => {}
        }

        node.subroots(|subroot| validate_group_text_fonts(subroot, fontdb, error));
        if error.is_some() {
            return;
        }
    }
}

fn text_font_embedding_error(
    text: &svg2pdf::usvg::Text,
    fontdb: &svg2pdf::usvg::fontdb::Database,
) -> Option<String> {
    if !text_has_visible_content(text) {
        return None;
    }

    if let Some(message) = missing_requested_font_error(text, fontdb) {
        return Some(message);
    }

    let mut glyph_count = 0usize;
    for span in text.layouted().iter().filter(|span| span.visible) {
        for glyph in &span.positioned_glyphs {
            glyph_count += 1;
            if glyph.id.0 == 0 {
                return Some(format!(
                    "missing glyph for '{}' in {}",
                    glyph.text,
                    describe_text_fonts(text)
                ));
            }
        }
    }

    if glyph_count == 0 {
        Some(format!(
            "no embeddable glyphs were resolved for {}",
            describe_text_fonts(text)
        ))
    } else {
        None
    }
}

fn missing_requested_font_error(
    text: &svg2pdf::usvg::Text,
    fontdb: &svg2pdf::usvg::fontdb::Database,
) -> Option<String> {
    for chunk in text.chunks() {
        for span in chunk.spans() {
            let span_text = chunk
                .text()
                .get(span.start()..span.end())
                .unwrap_or(chunk.text());
            if !span.is_visible() || !span_text.chars().any(|c| !c.is_whitespace()) {
                continue;
            }

            let named_families = span
                .font()
                .families()
                .iter()
                .filter_map(|family| match family {
                    svg2pdf::usvg::FontFamily::Named(name) => Some(name.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();

            if named_families.is_empty()
                || named_families
                    .iter()
                    .any(|family| fontdb_has_family(fontdb, family, span.font()))
            {
                continue;
            }

            return Some(format!(
                "missing requested text font families {}",
                named_families.join(", ")
            ));
        }
    }

    None
}

fn fontdb_has_family(
    fontdb: &svg2pdf::usvg::fontdb::Database,
    family: &str,
    font: &svg2pdf::usvg::Font,
) -> bool {
    let families = [svg2pdf::usvg::fontdb::Family::Name(family)];
    let query = svg2pdf::usvg::fontdb::Query {
        families: &families,
        weight: svg2pdf::usvg::fontdb::Weight(font.weight()),
        stretch: font_stretch(font.stretch()),
        style: font_style(font.style()),
    };
    fontdb.query(&query).is_some()
}

fn font_style(style: svg2pdf::usvg::FontStyle) -> svg2pdf::usvg::fontdb::Style {
    match style {
        svg2pdf::usvg::FontStyle::Normal => svg2pdf::usvg::fontdb::Style::Normal,
        svg2pdf::usvg::FontStyle::Italic => svg2pdf::usvg::fontdb::Style::Italic,
        svg2pdf::usvg::FontStyle::Oblique => svg2pdf::usvg::fontdb::Style::Oblique,
    }
}

fn font_stretch(stretch: svg2pdf::usvg::FontStretch) -> svg2pdf::usvg::fontdb::Stretch {
    match stretch {
        svg2pdf::usvg::FontStretch::UltraCondensed => {
            svg2pdf::usvg::fontdb::Stretch::UltraCondensed
        }
        svg2pdf::usvg::FontStretch::ExtraCondensed => {
            svg2pdf::usvg::fontdb::Stretch::ExtraCondensed
        }
        svg2pdf::usvg::FontStretch::Condensed => svg2pdf::usvg::fontdb::Stretch::Condensed,
        svg2pdf::usvg::FontStretch::SemiCondensed => svg2pdf::usvg::fontdb::Stretch::SemiCondensed,
        svg2pdf::usvg::FontStretch::Normal => svg2pdf::usvg::fontdb::Stretch::Normal,
        svg2pdf::usvg::FontStretch::SemiExpanded => svg2pdf::usvg::fontdb::Stretch::SemiExpanded,
        svg2pdf::usvg::FontStretch::Expanded => svg2pdf::usvg::fontdb::Stretch::Expanded,
        svg2pdf::usvg::FontStretch::ExtraExpanded => svg2pdf::usvg::fontdb::Stretch::ExtraExpanded,
        svg2pdf::usvg::FontStretch::UltraExpanded => svg2pdf::usvg::fontdb::Stretch::UltraExpanded,
    }
}

fn font_weight_number(font_weight: &FontWeight) -> u16 {
    match font_weight {
        FontWeight::Name(FontWeightNameSpec::Normal) => 400,
        FontWeight::Name(FontWeightNameSpec::Bold) => 700,
        FontWeight::Number(weight) => *weight as u16,
    }
}

fn scene_font_style(font_style: &FontStyle) -> svg2pdf::usvg::fontdb::Style {
    match font_style {
        FontStyle::Normal => svg2pdf::usvg::fontdb::Style::Normal,
        FontStyle::Italic => svg2pdf::usvg::fontdb::Style::Italic,
    }
}

fn text_has_visible_content(text: &svg2pdf::usvg::Text) -> bool {
    text.chunks().iter().any(|chunk| {
        chunk.spans().iter().any(|span| {
            span.is_visible()
                && chunk
                    .text()
                    .get(span.start()..span.end())
                    .unwrap_or(chunk.text())
                    .chars()
                    .any(|c| !c.is_whitespace())
        })
    })
}

fn describe_text_fonts(text: &svg2pdf::usvg::Text) -> String {
    let mut families = Vec::new();
    for chunk in text.chunks() {
        for span in chunk.spans() {
            for family in span.font().families() {
                let family = family.to_string();
                if !families.contains(&family) {
                    families.push(family);
                }
            }
        }
    }

    if families.is_empty() {
        "requested text fonts".to_string()
    } else {
        format!("requested text font families {}", families.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use avenger_color::{ColorOrGradient, Gradient, GradientStop, LinearGradient, RadialGradient};
    use avenger_common::{
        types::{ImageAlign, ImageBaseline},
        value::ScalarOrArray,
    };
    use avenger_image::RgbaImage;
    use avenger_scenegraph::{
        marks::{
            group::{Clip, SceneGroup},
            image::SceneImageMark,
            rect::SceneRectMark,
            text::SceneTextMark,
        },
        scene_graph::SceneGraph,
    };
    use avenger_text::FontResolutionOptions;

    use super::*;

    #[test]
    fn renders_scenegraph_to_pdf_bytes() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 10.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                x: ScalarOrArray::new_scalar(1.0),
                y: ScalarOrArray::new_scalar(2.0),
                width: Some(ScalarOrArray::new_scalar(3.0)),
                height: Some(ScalarOrArray::new_scalar(4.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0])),
                ..Default::default()
            }
            .into()],
        };

        let pdf = PdfRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn simple_vector_scene_does_not_emit_pdf_image_xobject() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 10.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                x: ScalarOrArray::new_scalar(1.0),
                y: ScalarOrArray::new_scalar(2.0),
                width: Some(ScalarOrArray::new_scalar(3.0)),
                height: Some(ScalarOrArray::new_scalar(4.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0])),
                ..Default::default()
            }
            .into()],
        };

        let pdf = PdfRenderer::new()
            .with_options(PdfRenderOptions {
                compress: false,
                ..Default::default()
            })
            .render_scene_graph(&scene_graph)
            .unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
        assert!(!pdf_contains(&pdf, b"/Subtype /Image"));
    }

    #[test]
    fn writes_scenegraph_pdf_and_creates_parent_dirs() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 10.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                x: ScalarOrArray::new_scalar(1.0),
                y: ScalarOrArray::new_scalar(2.0),
                width: Some(ScalarOrArray::new_scalar(3.0)),
                height: Some(ScalarOrArray::new_scalar(4.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0])),
                ..Default::default()
            }
            .into()],
        };
        let test_dir = std::env::temp_dir().join(format!(
            "avenger-pdf-write-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let output = test_dir.join("nested").join("chart.pdf");

        PdfRenderer::new()
            .write_scene_graph_pdf(&scene_graph, &output)
            .unwrap();

        let pdf = std::fs::read(&output).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        std::fs::remove_dir_all(test_dir).unwrap();
    }

    #[test]
    fn internal_svg_preserves_native_text_for_pdf_conversion() {
        let scene_graph = SceneGraph {
            width: 80.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar("Selectable".to_string()),
                x: ScalarOrArray::new_scalar(5.0),
                y: ScalarOrArray::new_scalar(12.0),
                font: ScalarOrArray::new_scalar("Atkinson Hyperlegible Next".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                ..Default::default()
            }
            .into()],
        };

        let renderer = PdfRenderer::new();
        let svg = renderer.render_svg_for_pdf(&scene_graph).unwrap();
        let tree = svg2pdf::usvg::Tree::from_str(&svg, &renderer.usvg_options()).unwrap();
        let pdf = renderer.render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains("<text "));
        assert!(tree.has_text_nodes());
        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn text_heavy_scenegraph_embeds_selectable_text_fonts() {
        let scene_graph = SceneGraph {
            width: 160.0,
            height: 60.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                len: 4,
                text: ScalarOrArray::new_array(vec![
                    "Alpha".to_string(),
                    "Beta".to_string(),
                    "Gamma".to_string(),
                    "Delta".to_string(),
                ]),
                x: ScalarOrArray::new_array(vec![8.0, 48.0, 88.0, 128.0]),
                y: ScalarOrArray::new_array(vec![16.0, 28.0, 40.0, 52.0]),
                font: ScalarOrArray::new_scalar("Atkinson Hyperlegible Next".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                ..Default::default()
            }
            .into()],
        };

        let renderer = PdfRenderer::new().with_options(PdfRenderOptions {
            compress: false,
            ..Default::default()
        });
        let svg = renderer.render_svg_for_pdf(&scene_graph).unwrap();
        let tree = svg2pdf::usvg::Tree::from_str(&svg, &renderer.usvg_options()).unwrap();
        let pdf = renderer.render_scene_graph(&scene_graph).unwrap();

        assert_eq!(svg.matches("<text ").count(), 4);
        assert!(tree.has_text_nodes());
        assert!(pdf_contains(&pdf, b"/FontDescriptor"));
        assert!(pdf_contains(&pdf, b"/FontFile2") || pdf_contains(&pdf, b"/FontFile3"));
        assert!(pdf_contains(&pdf, b"/ToUnicode"));
    }

    #[test]
    fn embeds_subset_font_for_bundled_text_font() {
        let scene_graph = SceneGraph {
            width: 80.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar("Embed me".to_string()),
                x: ScalarOrArray::new_scalar(5.0),
                y: ScalarOrArray::new_scalar(12.0),
                font: ScalarOrArray::new_scalar("Atkinson Hyperlegible Next".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                ..Default::default()
            }
            .into()],
        };

        let pdf = PdfRenderer::new()
            .with_options(PdfRenderOptions {
                compress: false,
                ..Default::default()
            })
            .render_scene_graph(&scene_graph)
            .unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf_contains(&pdf, b"/FontDescriptor"));
        assert!(pdf_contains(&pdf, b"/FontFile2") || pdf_contains(&pdf, b"/FontFile3"));
        assert!(pdf_contains(&pdf, b"/ToUnicode"));
    }

    #[test]
    fn errors_when_text_font_cannot_be_embedded() {
        let scene_graph = SceneGraph {
            width: 80.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar("Missing font".to_string()),
                x: ScalarOrArray::new_scalar(5.0),
                y: ScalarOrArray::new_scalar(12.0),
                font: ScalarOrArray::new_scalar("Definitely Missing Avenger Font".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                ..Default::default()
            }
            .into()],
        };

        let err = PdfRenderer::new()
            .render_scene_graph(&scene_graph)
            .unwrap_err();

        assert!(matches!(
            err,
            AvengerPdfError::Font(message)
                if message.contains("missing requested text font family")
                    && message.contains("Definitely Missing Avenger Font")
        ));
    }

    #[test]
    fn svg_parse_errors_map_to_pdf_error() {
        let err: AvengerPdfError =
            svg2pdf::usvg::Tree::from_str("<svg", &svg2pdf::usvg::Options::default())
                .unwrap_err()
                .into();

        assert!(matches!(err, AvengerPdfError::SvgParse(_)));
    }

    #[test]
    fn embeds_font_loaded_from_extra_font_dir() {
        let scene_graph = SceneGraph {
            width: 80.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar("Caveat".to_string()),
                x: ScalarOrArray::new_scalar(5.0),
                y: ScalarOrArray::new_scalar(12.0),
                font: ScalarOrArray::new_scalar("Caveat".to_string()),
                font_size: ScalarOrArray::new_scalar(14.0),
                ..Default::default()
            }
            .into()],
        };
        let caveat_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../avenger-vega-test-data/fonts/Caveat/static");

        let pdf = PdfRenderer::new()
            .with_options(PdfRenderOptions {
                compress: false,
                font_resolution: FontResolutionOptions {
                    extra_font_dirs: vec![caveat_dir],
                    ..Default::default()
                },
                ..Default::default()
            })
            .render_scene_graph(&scene_graph)
            .unwrap();

        assert!(pdf_contains(&pdf, b"/FontFile2") || pdf_contains(&pdf, b"/FontFile3"));
        assert!(pdf_contains(&pdf, b"/ToUnicode"));
    }

    #[test]
    fn image_mark_converts_to_pdf_image_xobject() {
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneImageMark {
                len: 1,
                aspect: false,
                smooth: false,
                image: ScalarOrArray::new_scalar(RgbaImage {
                    width: 1,
                    height: 1,
                    data: vec![255, 0, 0, 255],
                }),
                x: ScalarOrArray::new_scalar(4.0),
                y: ScalarOrArray::new_scalar(5.0),
                width: ScalarOrArray::new_scalar(12.0),
                height: ScalarOrArray::new_scalar(8.0),
                align: ScalarOrArray::new_scalar(ImageAlign::Left),
                baseline: ScalarOrArray::new_scalar(ImageBaseline::Top),
                ..Default::default()
            }
            .into()],
        };

        let renderer = PdfRenderer::new().with_options(PdfRenderOptions {
            compress: false,
            ..Default::default()
        });
        let svg = renderer.render_svg_for_pdf(&scene_graph).unwrap();
        let pdf = renderer.render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains("<image "));
        assert!(svg.contains("data:image/png;base64,"));
        assert!(pdf_contains(&pdf, b"/Subtype /Image"));
    }

    #[test]
    fn gradient_and_clip_scenegraph_converts_to_pdf() {
        let scene_graph = SceneGraph {
            width: 50.0,
            height: 30.0,
            origin: [0.0, 0.0],
            marks: vec![SceneGroup {
                clip: Clip::Rect {
                    x: 2.0,
                    y: 3.0,
                    width: 30.0,
                    height: 12.0,
                },
                marks: vec![SceneRectMark {
                    len: 1,
                    clip: true,
                    gradients: vec![Gradient::LinearGradient(LinearGradient {
                        x0: 0.0,
                        y0: 0.0,
                        x1: 1.0,
                        y1: 0.0,
                        stops: vec![
                            GradientStop {
                                offset: 0.0,
                                color: [1.0, 0.0, 0.0, 1.0],
                            },
                            GradientStop {
                                offset: 1.0,
                                color: [0.0, 0.0, 1.0, 1.0],
                            },
                        ],
                    })],
                    x: ScalarOrArray::new_scalar(0.0),
                    y: ScalarOrArray::new_scalar(0.0),
                    width: Some(ScalarOrArray::new_scalar(40.0)),
                    height: Some(ScalarOrArray::new_scalar(20.0)),
                    fill: ScalarOrArray::new_scalar(ColorOrGradient::GradientIndex(0)),
                    ..Default::default()
                }
                .into()],
                ..Default::default()
            }
            .into()],
        };

        let renderer = PdfRenderer::new().with_options(PdfRenderOptions {
            compress: false,
            ..Default::default()
        });
        let svg = renderer.render_svg_for_pdf(&scene_graph).unwrap();
        let tree = svg2pdf::usvg::Tree::from_str(&svg, &renderer.usvg_options()).unwrap();
        let pdf = renderer.render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains("<clipPath "));
        assert!(svg.contains("<linearGradient "));
        assert!(tree.size().width() > 0.0);
        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn radial_gradient_pattern_svg_converts_to_pdf() {
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                gradients: vec![Gradient::RadialGradient(RadialGradient {
                    x0: 0.5,
                    y0: 0.5,
                    x1: 0.5,
                    y1: 0.5,
                    r0: 0.0,
                    r1: 0.5,
                    stops: vec![
                        GradientStop {
                            offset: 0.0,
                            color: [1.0, 0.0, 0.0, 1.0],
                        },
                        GradientStop {
                            offset: 1.0,
                            color: [0.0, 0.0, 1.0, 1.0],
                        },
                    ],
                })],
                x: ScalarOrArray::new_scalar(2.0),
                y: ScalarOrArray::new_scalar(3.0),
                width: Some(ScalarOrArray::new_scalar(30.0)),
                height: Some(ScalarOrArray::new_scalar(10.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::GradientIndex(0)),
                ..Default::default()
            }
            .into()],
        };

        let renderer = PdfRenderer::new();
        let svg = renderer.render_svg_for_pdf(&scene_graph).unwrap();
        let tree = svg2pdf::usvg::Tree::from_str(&svg, &renderer.usvg_options()).unwrap();
        let pdf = renderer.render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains("<pattern "));
        assert!(svg.contains("<radialGradient "));
        assert!(svg.contains(r#"fill="url(#svg-gradient-0)""#));
        assert!(tree.size().width() > 0.0);
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(!pdf_contains(&pdf, b"/Subtype /Image"));
    }

    fn pdf_contains(pdf: &[u8], needle: &[u8]) -> bool {
        pdf.windows(needle.len()).any(|window| window == needle)
    }
}
