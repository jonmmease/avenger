use crate::label::EngineOptions;
use crate::label::{LabelError, LabelInitError, PdfTextLayer};
use crate::typst_layout::frame::{LineLayoutArtifact, LineLayoutOptions, TypesetMetrics};
#[cfg(test)]
use crate::typst_layout::frame::{MathLayoutOptions, MathRunArtifact};
use crate::typst_svg::PathArtifact;

#[cfg(test)]
use crate::typst_eval::markup::parse_line_with_params;
#[cfg(test)]
use crate::typst_eval::math::parse_math;
use crate::typst_layout::inline::font::build_text_fontdb;
use crate::typst_layout::inline::try_layout_text_line;
#[cfg(test)]
use crate::typst_layout::math::try_typeset_simple_row_fragment_with_fontdb;
use crate::typst_library::text::content::{LabelContent, LineNode, PlainTextNode};

#[derive(Clone)]
pub(crate) struct TypstEngineCore {
    config: EngineOptions,
    text_fontdb: std::sync::Arc<fontdb::Database>,
}

impl std::fmt::Debug for TypstEngineCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypstEngineCore").finish_non_exhaustive()
    }
}

impl TypstEngineCore {
    pub(crate) fn new(config: &EngineOptions) -> Result<Self, LabelInitError> {
        Ok(Self {
            config: config.clone(),
            text_fontdb: std::sync::Arc::new(build_text_fontdb(config)),
        })
    }

    pub(crate) fn check_font(
        &self,
        style: &crate::label::TextStyle,
    ) -> Result<Option<crate::label::LabelWarning>, LabelError> {
        use crate::label::{LabelWarning, MissingFontPolicy};
        use crate::typst_layout::inline::font::TextFace;
        if self.config.fonts.missing_font == MissingFontPolicy::Fallback
            || TextFace::for_plain_style(style, &self.text_fontdb)?.is_some()
        {
            return Ok(None);
        }
        let family = style.font_family.clone();
        match self.config.fonts.missing_font {
            MissingFontPolicy::Error => Err(LabelError::MissingFont { family }),
            MissingFontPolicy::Warn => Ok(Some(LabelWarning::MissingFont { family })),
            MissingFontPolicy::Fallback => Ok(None),
        }
    }

    pub(crate) fn font_metrics(
        &self,
        style: &crate::label::TextStyle,
    ) -> Result<crate::label::FontMetrics, LabelError> {
        use crate::typst_layout::inline::font::TextFace;
        self.check_font(style)?;
        let face = TextFace::for_plain_style(style, &self.text_fontdb)?;
        let fallback = crate::label::TextStyle {
            font_family: "sans-serif".to_string(),
            ..style.clone()
        };
        face.or(TextFace::for_plain_style(&fallback, &self.text_fontdb)?)
            .and_then(|face| face.font_metrics(style.font_size))
            .ok_or_else(|| LabelError::MissingFont {
                family: style.font_family.clone(),
            })
    }

    #[cfg(test)]
    pub(crate) fn typeset_fragment(
        &self,
        source: &str,
        options: &MathLayoutOptions,
    ) -> Result<MathRunArtifact, LabelError> {
        let math = parse_math(source, 0)?;
        if let Some(artifact) = try_typeset_simple_row_fragment_with_fontdb(
            &math,
            options,
            &self.config,
            self.text_fontdb.as_ref(),
        )? {
            return Ok(artifact);
        }
        unsupported_fragment()
    }

    #[cfg(test)]
    pub(crate) fn typeset_markup_line(
        &self,
        source: &str,
        options: &LineLayoutOptions,
    ) -> Result<LineLayoutArtifact, LabelError> {
        if source.is_empty() {
            return Ok(empty_line_layout_artifact(source));
        }

        let line = parse_line_with_params(source, &options.params)?;
        self.typeset_parsed_line(source, &line, options)
    }

    pub(crate) fn typeset_plain_line(
        &self,
        source: &str,
        options: &LineLayoutOptions,
    ) -> Result<LineLayoutArtifact, LabelError> {
        if source.is_empty() {
            return Ok(empty_line_layout_artifact(source));
        }

        let line = plain_text_line(source);
        self.typeset_parsed_line(source, &line, options)
    }

    pub(crate) fn typeset_parsed_line(
        &self,
        source: &str,
        line: &LabelContent,
        options: &LineLayoutOptions,
    ) -> Result<LineLayoutArtifact, LabelError> {
        if let Some(artifact) = try_layout_text_line(
            source,
            line,
            options,
            &self.config,
            self.text_fontdb.as_ref(),
        )? {
            return Ok(artifact);
        }
        if line_contains_static_markup(line) {
            return Err(LabelError::UnsupportedOutput(
                "static text markup is parsed but not rendered yet",
            ));
        }

        unsupported_line_layout()
    }
}

#[cfg(test)]
fn unsupported_fragment() -> Result<MathRunArtifact, LabelError> {
    Err(LabelError::UnsupportedOutput(
        "this Typst math subset is not supported yet",
    ))
}

fn unsupported_line_layout() -> Result<LineLayoutArtifact, LabelError> {
    Err(LabelError::UnsupportedOutput(
        "this Typst text-line subset is not supported yet",
    ))
}

fn plain_text_line(source: &str) -> LabelContent {
    LabelContent {
        source: source.to_string(),
        nodes: if source.is_empty() {
            Vec::new()
        } else {
            vec![LineNode::Plain(PlainTextNode {
                text: source.to_string(),
                byte_range: 0..source.len(),
            })]
        },
    }
}

fn line_contains_static_markup(line: &LabelContent) -> bool {
    nodes_contain_static_markup(&line.nodes)
}

fn nodes_contain_static_markup(nodes: &[LineNode]) -> bool {
    nodes.iter().any(|node| match node {
        LineNode::Plain(_)
        | LineNode::Math(_)
        | LineNode::SmartQuote(_)
        | LineNode::Emoji(_)
        | LineNode::Symbol(_)
        | LineNode::Param(_) => false,
        LineNode::TextSpan(_) => true,
    })
}

fn empty_line_layout_artifact(source: &str) -> LineLayoutArtifact {
    let metrics = TypesetMetrics {
        width: 0.0,
        height: 0.0,
        baseline: 0.0,
        ascent: 0.0,
        descent: 0.0,
    };
    let paths = PathArtifact {
        logical_width: 0.0,
        logical_height: 0.0,
        items: Vec::new(),
        images: Vec::new(),
        draw_order: Vec::new(),
    };
    let pdf_text = PdfTextLayer {
        logical_width: 0.0,
        logical_height: 0.0,
        semantic_text: source.to_string(),
        glyph_runs: Vec::new(),
    };

    LineLayoutArtifact {
        source: source.to_string(),
        metrics,
        paths,
        pdf_text,
        positioned_runs: Vec::new(),
        font_resources: Vec::new(),
        warnings: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typst_library::{FontStyle, FontWeight};

    fn has_path_output(paths: &PathArtifact) -> bool {
        !paths.items.is_empty() || !paths.images.is_empty()
    }

    fn has_pdf_text(pdf_text: &PdfTextLayer) -> bool {
        !pdf_text.glyph_runs.is_empty()
    }

    #[test]
    fn empty_text_line_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();

        let options = LineLayoutOptions::default();
        let artifact = engine.typeset_markup_line("", &options).unwrap();

        assert_eq!(artifact.metrics.width, 0.0);
        assert!(artifact.paths.items.is_empty());
        assert!(artifact.pdf_text.glyph_runs.is_empty());
        assert!(artifact.positioned_runs.is_empty());
    }

    #[test]
    fn plain_text_line_paths_use_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine.typeset_markup_line("Hello", &options).unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert_eq!(artifact.paths.items.len(), 5);
    }

    #[test]
    fn plain_text_line_with_non_rtl_missing_glyph_paths_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine.typeset_markup_line("Revenue 🚀", &options).unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert!(has_path_output(&artifact.paths));
    }

    #[test]
    fn non_atkinson_plain_text_can_use_fontdb_fallback() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = LineLayoutOptions::default();
        options.text_style.font_family = "serif".to_string();

        let Ok(artifact) = engine.typeset_markup_line("Fallback font", &options) else {
            return;
        };

        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(artifact.positioned_runs[0].text, "Fallback font");
        assert!(has_path_output(&artifact.paths));
        assert!(has_pdf_text(&artifact.pdf_text));
        assert_eq!(artifact.font_resources.len(), 1);
    }

    #[test]
    fn non_atkinson_mixed_script_text_can_segment_fallback_fonts() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = LineLayoutOptions::default();
        options.text_style.font_family = "serif".to_string();

        let Ok(artifact) = engine.typeset_markup_line("Hello 温度", &options) else {
            return;
        };

        if artifact.pdf_text.glyph_runs.len() < 2 {
            return;
        }

        assert!(artifact.positioned_runs.len() >= 2);
        assert_eq!(
            artifact
                .positioned_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "Hello 温度"
        );
        assert!(
            artifact
                .positioned_runs
                .iter()
                .all(|run| run.pdf_text.is_some())
        );
        assert!(has_path_output(&artifact.paths));
        assert!(artifact.font_resources.len() >= 2);
    }

    #[test]
    fn default_mixed_script_text_can_segment_fallback_fonts_when_available() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        // Separate script runs can still use the same missing-glyph font.
        // Require an actual CJK face before asserting cross-font fallback.
        let has_cjk_font = engine.text_fontdb.faces().any(|info| {
            engine
                .text_fontdb
                .with_face_data(info.id, |data, index| {
                    ttf_parser::Face::parse(data, index)
                        .is_ok_and(|face| "温度".chars().all(|ch| face.glyph_index(ch).is_some()))
                })
                .unwrap_or(false)
        });
        if !has_cjk_font {
            return;
        }
        let options = LineLayoutOptions::default();

        let artifact = engine.typeset_markup_line("Hello 温度", &options).unwrap();
        let pdf_text = &artifact.pdf_text;
        if pdf_text.glyph_runs.len() < 2 {
            return;
        }

        assert!(artifact.positioned_runs.len() >= 2);
        assert_eq!(
            artifact
                .positioned_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "Hello 温度"
        );
        assert!(
            artifact
                .positioned_runs
                .iter()
                .all(|run| run.pdf_text.is_some())
        );
        assert!(has_path_output(&artifact.paths));
        assert!(artifact.font_resources.len() >= 2);
    }

    #[test]
    fn plain_text_line_with_rtl_text_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine.typeset_markup_line("שלום", &options).unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert!(has_path_output(&artifact.paths));
        assert!(has_pdf_text(&artifact.pdf_text));
        assert_eq!(
            artifact
                .positioned_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>(),
            vec!["שלום"]
        );
    }

    #[test]
    fn plain_text_line_treats_invalid_math_as_literal_text() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine
            .typeset_plain_line("before $x^$ after", &options)
            .unwrap();

        assert!(artifact.metrics.width > 0.0);
    }

    #[test]
    fn plain_text_line_with_zwj_emoji_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine.typeset_markup_line("Family 👨‍👩‍👧‍👦", &options).unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert!(has_path_output(&artifact.paths));
        assert!(has_pdf_text(&artifact.pdf_text));
        assert_eq!(
            artifact
                .positioned_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Family ", "👨‍👩‍👧‍👦"]
        );
    }

    #[test]
    fn named_emoji_alias_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine
            .typeset_markup_line("Revenue #emoji.rocket", &options)
            .unwrap();

        assert_eq!(artifact.source, "Revenue #emoji.rocket");
        assert_eq!(
            artifact
                .positioned_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Revenue ", "🚀"]
        );
        assert!(has_path_output(&artifact.paths));
        assert!(has_pdf_text(&artifact.pdf_text));
    }

    #[test]
    fn named_symbol_alias_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine
            .typeset_markup_line("Flow #sym.arrow.r target", &options)
            .unwrap();

        assert_eq!(artifact.source, "Flow #sym.arrow.r target");
        assert_eq!(
            artifact
                .positioned_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Flow → target"]
        );
        assert!(has_path_output(&artifact.paths));
        assert!(
            artifact
                .pdf_text
                .glyph_runs
                .iter()
                .any(|run| run.text == "Flow → target")
        );
    }

    #[cfg(all(feature = "raster", target_os = "macos"))]
    #[test]
    fn named_emoji_alias_rasterizes_color_pixels_on_macos() {
        let label = crate::label::LabelEngine::new(EngineOptions::default())
            .unwrap()
            .compile("Revenue #emoji.rocket", &Default::default())
            .unwrap();
        let raster = crate::label::rasterize(&label, &Default::default())
            .expect("raster output should be produced for emoji text");

        assert!(
            colored_pixel_count(&raster.image.data) > 20,
            "emoji raster should contain colored bitmap pixels rather than monochrome tofu"
        );
    }

    #[cfg(all(feature = "raster", target_os = "macos"))]
    fn colored_pixel_count(data: &[u8]) -> usize {
        data.chunks_exact(4)
            .filter(|pixel| {
                let [r, g, b, a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
                a > 0 && r.abs_diff(g).max(r.abs_diff(b)).max(g.abs_diff(b)) > 16
            })
            .count()
    }

    #[test]
    fn unknown_static_command_errors_before_rendering() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let err = engine
            .typeset_markup_line("#let x = 1", &options)
            .unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 0,
                message: "unsupported static text command"
            }
        );
    }

    #[test]
    fn static_command_options_render() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine
            .typeset_markup_line("#underline(stroke: red)[important]", &options)
            .unwrap();

        assert!(
            artifact
                .paths
                .items
                .iter()
                .any(|item| item.stroke.is_some())
        );
    }

    #[test]
    fn supported_static_decoration_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine
            .typeset_markup_line("#underline[important]", &options)
            .unwrap();

        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(artifact.positioned_runs[0].text, "important");
        assert!(artifact.positioned_runs[0].paths.is_some());
        assert!(has_pdf_text(&artifact.pdf_text));
    }

    #[test]
    fn static_case_transform_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine
            .typeset_markup_line("Mode #lower[MiXeD #sym.arrow.r] #upper(\"loud\")", &options)
            .unwrap();

        assert_eq!(
            artifact
                .positioned_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Mode ", "mixed →", " ", "LOUD"]
        );
        assert!(has_path_output(&artifact.paths));
        assert!(
            artifact
                .pdf_text
                .glyph_runs
                .iter()
                .any(|run| run.text == "mixed →")
        );
    }

    #[test]
    fn styled_plain_pdf_ranges_index_semantic_text() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        for (source, expected) in [
            ("*Bold label*", "Bold label"),
            ("_Italic label_", "Italic label"),
            ("#upper[caption]", "CAPTION"),
        ] {
            let artifact = engine
                .typeset_markup_line(source, &LineLayoutOptions::default())
                .unwrap();
            assert_eq!(artifact.pdf_text.semantic_text, expected);
            for run in &artifact.pdf_text.glyph_runs {
                assert_eq!(run.text, expected);
                for glyph in &run.glyphs {
                    assert_eq!(&run.text[glyph.text_range.clone()], glyph.unicode);
                }
            }
        }
    }

    #[test]
    fn static_smallcaps_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let smallcaps = engine
            .typeset_markup_line(
                "#smallcaps[Smallcaps] #smallcaps(all: true)[UNICEF]",
                &options,
            )
            .unwrap();

        assert_eq!(
            smallcaps
                .positioned_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Smallcaps", " ", "UNICEF"]
        );
        assert!(has_path_output(&smallcaps.paths));
        assert_eq!(
            smallcaps.pdf_text.semantic_text.as_str(),
            "Smallcaps UNICEF"
        );
        assert!(
            smallcaps.metrics.width > 0.0,
            "smallcaps labels should produce normal text metrics"
        );
        assert!(
            smallcaps
                .pdf_text
                .glyph_runs
                .iter()
                .any(|run| run.text == "Smallcaps"),
            "smallcaps text should remain PDF text rather than path-only output"
        );
    }

    #[test]
    fn static_emph_and_strong_use_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine
            .typeset_markup_line(
                "_Emph_ *Strong* #emph[call] #strong(delta: 150)[mild]",
                &options,
            )
            .unwrap();

        assert_eq!(
            artifact
                .positioned_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Emph", " ", "Strong", " ", "call", " ", "mild"]
        );
        assert_eq!(
            artifact.positioned_runs[0]
                .text_style
                .as_ref()
                .map(|style| style.font_style),
            Some(FontStyle::Italic)
        );
        assert_eq!(
            artifact.positioned_runs[2]
                .text_style
                .as_ref()
                .map(|style| &style.font_weight),
            Some(&FontWeight::Bold)
        );
        assert_eq!(
            artifact.positioned_runs[4]
                .text_style
                .as_ref()
                .map(|style| style.font_style),
            Some(FontStyle::Italic)
        );
        assert_eq!(
            artifact.positioned_runs[6]
                .text_style
                .as_ref()
                .map(|style| &style.font_weight),
            Some(&FontWeight::Number(550))
        );
        assert!(has_path_output(&artifact.paths));
        assert!(
            artifact
                .pdf_text
                .glyph_runs
                .iter()
                .any(|run| run.text == "Strong")
        );

        let solo = engine.typeset_markup_line("#emph[solo]", &options).unwrap();
        assert_eq!(solo.positioned_runs.len(), 1);
        assert_eq!(
            solo.positioned_runs[0]
                .text_style
                .as_ref()
                .map(|style| style.font_style),
            Some(FontStyle::Italic)
        );
    }

    #[test]
    fn static_raw_uses_monospace_show_set() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = LineLayoutOptions::default();
        options.text_style.font_size = 20.0;

        let artifact = engine
            .typeset_markup_line("Use `x # y` and #raw(\"z * w\")", &options)
            .unwrap();

        assert_eq!(
            artifact
                .positioned_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Use ", "x # y", " and ", "z * w"]
        );
        for index in [1, 3] {
            let style = artifact.positioned_runs[index]
                .text_style
                .as_ref()
                .expect("raw text should preserve positioned text style");
            assert_eq!(style.font_family, "monospace");
            assert_eq!(style.font_size, 16.0);
        }
        assert!(has_path_output(&artifact.paths));
        assert!(
            artifact
                .pdf_text
                .glyph_runs
                .iter()
                .any(|run| run.text == "x # y")
                && artifact
                    .pdf_text
                    .glyph_runs
                    .iter()
                    .any(|run| run.text == "z * w")
        );
    }

    #[test]
    fn synthesized_subscript_and_superscript_use_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine
            .typeset_markup_line(
                "H#sub(typographic: false)[2]O #super(typographic: false)[\\*]",
                &options,
            )
            .unwrap();

        assert_eq!(artifact.positioned_runs.len(), 4);
        assert_eq!(artifact.positioned_runs[0].text, "H");
        assert_eq!(artifact.positioned_runs[1].text, "2");
        assert_eq!(artifact.positioned_runs[2].text, "O ");
        assert_eq!(artifact.positioned_runs[3].text, "*");
        assert!(artifact.positioned_runs[1].y > artifact.positioned_runs[0].y);
        assert!(artifact.positioned_runs[3].y < artifact.positioned_runs[0].y);
        assert!(
            artifact.positioned_runs[1]
                .text_style
                .as_ref()
                .is_some_and(|style| style.font_size < options.text_style.font_size)
        );
        assert!(
            artifact.positioned_runs[3]
                .text_style
                .as_ref()
                .is_some_and(|style| style.font_size < options.text_style.font_size)
        );
        assert!(has_path_output(&artifact.paths));
        assert!(has_pdf_text(&artifact.pdf_text));
    }

    #[test]
    fn static_subscript_and_superscript_honor_literal_options() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = LineLayoutOptions::default();
        options.text_style.font_size = 20.0;

        let source = "x#super(typographic: false, baseline: -0.25em, size: 0.7em)[N] \
                      y#sub(typographic: false, baseline: 0.2em, size: 0.6em)[2]";
        let artifact = engine.typeset_markup_line(source, &options).unwrap();

        assert_eq!(
            artifact
                .positioned_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>(),
            vec!["x", "N", " y", "2"]
        );
        let plain_y = artifact.positioned_runs[0].y;
        let super_run = &artifact.positioned_runs[1];
        let sub_run = &artifact.positioned_runs[3];
        assert!((super_run.y - (plain_y - 5.0)).abs() < 1e-4);
        assert!((sub_run.y - (plain_y + 4.0)).abs() < 1e-4);
        assert!(
            super_run
                .text_style
                .as_ref()
                .is_some_and(|style| (style.font_size - 14.0).abs() < 1e-4)
        );
        assert!(
            sub_run
                .text_style
                .as_ref()
                .is_some_and(|style| (style.font_size - 12.0).abs() < 1e-4)
        );
        assert!(has_path_output(&artifact.paths));
        assert!(has_pdf_text(&artifact.pdf_text));
    }

    #[test]
    fn matrix_math_fragment_errors_before_rendering() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();

        let err = engine
            .typeset_fragment("mat(1, 2; 3, 4)", &MathLayoutOptions::default())
            .unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedFeature {
                position: 0,
                feature: "mat".to_string(),
                message: "matrix/table math is not supported in Avenger Typst subset"
            }
        );
    }

    #[test]
    fn simple_row_math_fragment_metrics_only_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine
            .typeset_fragment("alpha + beta -> gamma", &options)
            .unwrap();

        assert!(artifact.metrics.width > 0.0);
    }

    #[test]
    fn simple_row_math_fragment_paths_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine
            .typeset_fragment("alpha + beta -> gamma", &options)
            .unwrap();

        assert!(has_path_output(&artifact.paths));
    }

    #[test]
    fn simple_row_math_fragment_pdf_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine
            .typeset_fragment("alpha + beta -> gamma", &options)
            .unwrap();

        assert!(has_pdf_text(&artifact.pdf_text));
        assert_eq!(artifact.font_resources.len(), 1);
    }

    #[test]
    fn simple_script_math_fragment_pdf_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine.typeset_fragment("R^2 = 0.94", &options).unwrap();

        let pdf = &artifact.pdf_text;
        assert!(pdf.glyph_runs.iter().any(|run| run.font_size < 12.0));
        assert_eq!(artifact.font_resources.len(), 1);
    }

    #[test]
    fn simple_fraction_math_fragment_paths_use_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine.typeset_fragment("a / (b + c)", &options).unwrap();

        assert_eq!(artifact.paths.items.len(), 5);
        assert_eq!(artifact.pdf_text.glyph_runs.len(), 4);
    }

    #[test]
    fn simple_frac_call_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine.typeset_fragment("frac(x + y, z)", &options).unwrap();

        assert_eq!(artifact.paths.items.len(), 5);
        assert_eq!(artifact.pdf_text.glyph_runs.len(), 4);
    }

    #[test]
    fn simple_binom_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine.typeset_fragment("binom(n, k)", &options).unwrap();

        assert_eq!(artifact.paths.items.len(), 4);
        assert_eq!(artifact.pdf_text.glyph_runs.len(), 4);
    }

    #[test]
    fn simple_cancel_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine.typeset_fragment("cancel(x)", &options).unwrap();

        assert_eq!(artifact.paths.items.len(), 2);
        assert_eq!(artifact.pdf_text.glyph_runs.len(), 1);
    }

    #[test]
    fn simple_sqrt_fraction_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine
            .typeset_fragment("sqrt(x) / (1 + x^2)", &options)
            .unwrap();

        assert_eq!(artifact.paths.items.len(), 8);
        assert_eq!(artifact.pdf_text.glyph_runs.len(), 6);
    }

    #[test]
    fn simple_indexed_root_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine.typeset_fragment("root(3, x)", &options).unwrap();

        assert_eq!(artifact.paths.items.len(), 4);
        assert_eq!(artifact.pdf_text.glyph_runs.len(), 3);
    }

    #[test]
    fn simple_group_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine.typeset_fragment("x(t)", &options).unwrap();

        assert_eq!(artifact.paths.items.len(), 4);
        assert_eq!(artifact.pdf_text.glyph_runs.len(), 4);
    }

    #[test]
    fn identifier_subscript_group_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine.typeset_fragment("J_n(x)", &options).unwrap();

        assert_eq!(artifact.paths.items.len(), 5);
        assert_eq!(artifact.pdf_text.glyph_runs.len(), 5);
    }

    #[test]
    fn simple_delimiter_call_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine.typeset_fragment("abs(x)", &options).unwrap();

        assert_eq!(artifact.paths.items.len(), 3);
        assert_eq!(artifact.pdf_text.glyph_runs.len(), 3);
    }

    #[test]
    fn simple_lr_call_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine.typeset_fragment("lr(|x + y|)", &options).unwrap();

        assert_eq!(artifact.paths.items.len(), 5);
        assert_eq!(artifact.pdf_text.glyph_runs.len(), 5);
    }

    #[test]
    fn simple_operator_call_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine.typeset_fragment("sin(x)", &options).unwrap();

        assert_eq!(artifact.paths.items.len(), 6);
        assert_eq!(artifact.pdf_text.glyph_runs.len(), 4);
    }

    #[test]
    fn operator_identifier_script_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine
            .typeset_fragment("lim_(x -> oo) f(x)", &options)
            .unwrap();

        assert!(has_path_output(&artifact.paths));
        assert!(artifact.pdf_text.glyph_runs.len() > 6);
    }

    #[test]
    fn common_named_symbols_use_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = engine
            .typeset_fragment("forall x in RR + A subset.eq B + arrow.r.double", &options)
            .unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert!(has_path_output(&artifact.paths));
        let glyph_text = artifact
            .pdf_text
            .glyph_runs
            .iter()
            .flat_map(|run| run.glyphs.iter())
            .map(|glyph| glyph.unicode.as_str())
            .collect::<String>();
        assert!(glyph_text.contains('∀'));
        assert!(glyph_text.contains('∈'));
        assert!(glyph_text.contains('⊆'));
        assert!(glyph_text.contains('⇒'));
    }

    #[test]
    fn bold_math_fragment_uses_bundled_bold_math_font() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathLayoutOptions::default();
        options.style.font_weight = FontWeight::Bold;

        let artifact = engine.typeset_fragment("R^2 + beta", &options).unwrap();

        assert_eq!(
            artifact.font_resources[0].postscript_name.as_deref(),
            Some("LeteSansMath-Bold")
        );
    }

    #[cfg(feature = "raster")]
    #[test]
    fn simple_row_math_fragment_raster_uses_typst_engine() {
        let label = crate::label::LabelEngine::new(EngineOptions::default())
            .unwrap()
            .compile("$alpha + beta -> gamma$", &Default::default())
            .unwrap();
        let raster = crate::label::rasterize(&label, &Default::default())
            .expect("raster output should be produced for math text");
        assert!(raster.image.width > 0 && raster.image.height > 0);
    }

    #[test]
    fn matrix_text_line_span_reports_source_offset() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let err = engine
            .typeset_markup_line("before $mat(1, 2; 3, 4)$ after", &options)
            .unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedFeature {
                position: 8,
                feature: "mat".to_string(),
                message: "matrix/table math is not supported in Avenger Typst subset"
            }
        );
    }

    #[test]
    fn plain_text_line_metrics_only_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine.typeset_markup_line("Hello", &options).unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(artifact.positioned_runs[0].text, "Hello");
    }

    #[test]
    fn plain_text_line_pdf_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine.typeset_markup_line("Hello", &options).unwrap();

        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(artifact.font_resources.len(), 1);
        assert_eq!(artifact.pdf_text.glyph_runs.len(), 1);
    }

    #[test]
    fn mixed_text_math_metrics_use_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine
            .typeset_markup_line("Price \\$7, score $R^2$ = 0.94", &options)
            .unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert_eq!(artifact.positioned_runs.len(), 3);
        assert_eq!(artifact.positioned_runs[0].text, "Price $7, score ");
        assert_eq!(artifact.positioned_runs[1].text, "R^2");
        assert_eq!(artifact.positioned_runs[2].text, " = 0.94");
    }

    #[test]
    fn mixed_text_math_paths_use_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine
            .typeset_markup_line("Price \\$7, score $R^2$ = 0.94", &options)
            .unwrap();

        assert!(has_path_output(&artifact.paths));
        assert_eq!(artifact.positioned_runs.len(), 3);
        assert!(artifact.positioned_runs[1].paths.is_some());
    }

    #[test]
    fn mixed_text_math_pdf_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = LineLayoutOptions::default();

        let artifact = engine
            .typeset_markup_line("Price \\$7, score $R^2$ = 0.94", &options)
            .unwrap();

        assert_eq!(artifact.font_resources.len(), 2);
        assert!(has_pdf_text(&artifact.pdf_text));
        assert_eq!(artifact.positioned_runs.len(), 3);
        assert!(artifact.positioned_runs[0].pdf_text.is_some());
        assert!(artifact.positioned_runs[1].pdf_text.is_some());
        assert!(artifact.positioned_runs[2].pdf_text.is_some());
    }

    #[cfg(feature = "raster")]
    #[test]
    fn mixed_text_math_raster_uses_typst_engine() {
        let label = crate::label::LabelEngine::new(EngineOptions::default())
            .unwrap()
            .compile("Price \\$7, score $R^2$ = 0.94", &Default::default())
            .unwrap();
        let raster = crate::label::rasterize(&label, &Default::default())
            .expect("raster output should be produced for mixed text");
        assert!(raster.image.width > 0 && raster.image.height > 0);
    }
}
