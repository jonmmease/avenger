use std::collections::{BTreeMap, BTreeSet};

use avenger_text::{
    fonts::build_fontdb,
    types::{FontStyle, FontWeight, FontWeightNameSpec},
    FontResolutionOptions, MissingFontPolicy,
};
use base64::{prelude::BASE64_STANDARD, Engine};
use font_subset::FontReader;
use svgtypes::{parse_font_families, FontFamily};

use crate::error::AvengerSvgError;

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct SvgFontKey {
    family: String,
    weight: u16,
    style: SvgFontStyle,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct SvgFontRequest {
    families: Vec<String>,
    weight: u16,
    style: SvgFontStyle,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
enum SvgFontStyle {
    Normal,
    Italic,
}

#[derive(Debug, Default)]
pub(crate) struct SvgFontCollector {
    text_by_request: BTreeMap<SvgFontRequest, BTreeSet<char>>,
}

impl SvgFontCollector {
    pub(crate) fn collect_text(
        &mut self,
        font_family: &str,
        font_weight: &FontWeight,
        font_style: &FontStyle,
        text: &str,
        options: &FontResolutionOptions,
    ) -> Result<(), AvengerSvgError> {
        if text.is_empty() {
            return Ok(());
        }

        let families = parse_named_font_families(font_family, options)?;
        if families.is_empty() {
            return Ok(());
        }

        let request = SvgFontRequest {
            families,
            weight: font_weight_number(font_weight),
            style: (*font_style).into(),
        };

        self.text_by_request
            .entry(request)
            .or_default()
            .extend(text.chars());
        Ok(())
    }

    pub(crate) fn font_face_css(
        &self,
        options: &FontResolutionOptions,
    ) -> Result<String, AvengerSvgError> {
        if self.text_by_request.is_empty() {
            return Ok(String::new());
        }

        let fontdb = build_fontdb(options);
        let mut text_by_font = BTreeMap::new();

        for (request, chars) in &self.text_by_request {
            let Some(key) = resolve_font_request(&fontdb, request) else {
                handle_font_issue(
                    format!(
                        "missing SVG font face from [{}] weight {} style {}",
                        request.families.join(", "),
                        request.weight,
                        font_style_css(request.style)
                    ),
                    options.missing_font,
                )?;
                continue;
            };

            text_by_font
                .entry(key)
                .or_insert_with(BTreeSet::new)
                .extend(chars.iter().copied());
        }

        let mut css = String::new();

        for (key, chars) in &text_by_font {
            let Some(face_id) = query_font_face(&fontdb, key) else {
                handle_font_issue(
                    format!(
                        "missing SVG font face '{}' weight {} style {}",
                        key.family,
                        key.weight,
                        font_style_css(key.style)
                    ),
                    options.missing_font,
                )?;
                continue;
            };

            let Some(subset) = fontdb.with_face_data(face_id, |font_data, _face_index| {
                subset_font_to_woff2(font_data, chars)
            }) else {
                handle_font_issue(
                    format!(
                        "unable to load SVG font data for '{}' weight {} style {}",
                        key.family,
                        key.weight,
                        font_style_css(key.style)
                    ),
                    options.missing_font,
                )?;
                continue;
            };

            match subset {
                Ok(woff2) => push_font_face_css(&mut css, key, &woff2),
                Err(err) => {
                    handle_font_issue(
                        format!(
                            "unable to subset SVG font '{}' weight {} style {}: {err}",
                            key.family,
                            key.weight,
                            font_style_css(key.style)
                        ),
                        options.missing_font,
                    )?;
                }
            }
        }

        Ok(css)
    }
}

fn parse_named_font_families(
    font_family: &str,
    options: &FontResolutionOptions,
) -> Result<Vec<String>, AvengerSvgError> {
    let families = match parse_font_families(font_family) {
        Ok(families) => families,
        Err(err) => {
            handle_font_issue(
                format!("failed to parse font-family '{font_family}': {err}"),
                options.missing_font,
            )?;
            return Ok(Vec::new());
        }
    };

    let named = families
        .into_iter()
        .filter_map(|family| match family {
            FontFamily::Named(family) => Some(family),
            FontFamily::Serif
            | FontFamily::SansSerif
            | FontFamily::Cursive
            | FontFamily::Fantasy
            | FontFamily::Monospace => None,
        })
        .collect::<Vec<_>>();

    if named.is_empty() && !is_generic_only_font_family(font_family) {
        handle_font_issue(
            format!("font-family '{font_family}' does not contain an embeddable named family"),
            options.missing_font,
        )?;
    }

    Ok(named)
}

fn resolve_font_request(fontdb: &fontdb::Database, request: &SvgFontRequest) -> Option<SvgFontKey> {
    request.families.iter().find_map(|family| {
        let key = SvgFontKey {
            family: family.clone(),
            weight: request.weight,
            style: request.style,
        };
        query_font_face(fontdb, &key).map(|_| key)
    })
}

fn is_generic_only_font_family(font_family: &str) -> bool {
    parse_font_families(font_family).is_ok_and(|families| {
        !families.is_empty()
            && families.iter().all(|family| {
                matches!(
                    family,
                    FontFamily::Serif
                        | FontFamily::SansSerif
                        | FontFamily::Cursive
                        | FontFamily::Fantasy
                        | FontFamily::Monospace
                )
            })
    })
}

fn query_font_face(fontdb: &fontdb::Database, key: &SvgFontKey) -> Option<fontdb::ID> {
    let families = [fontdb::Family::Name(key.family.as_str())];
    let query = fontdb::Query {
        families: &families,
        weight: fontdb::Weight(key.weight),
        stretch: fontdb::Stretch::Normal,
        style: match key.style {
            SvgFontStyle::Normal => fontdb::Style::Normal,
            SvgFontStyle::Italic => fontdb::Style::Italic,
        },
    };

    fontdb.query(&query)
}

fn subset_font_to_woff2(font_data: &[u8], chars: &BTreeSet<char>) -> Result<Vec<u8>, String> {
    let reader = FontReader::new(font_data).map_err(|err| err.to_string())?;
    let font = reader.read().map_err(|err| err.to_string())?;
    let supported_chars = chars
        .iter()
        .copied()
        .filter(|ch| font.contains_char(*ch))
        .collect::<BTreeSet<_>>();
    if supported_chars.is_empty() {
        return Err("font does not contain any requested characters".to_string());
    }

    let subset = font
        .subset(&supported_chars)
        .map_err(|err| err.to_string())?;
    subset
        .validate()
        .map_err(|err| err.to_string())?
        .into_result()
        .map_err(|warnings| warnings.to_string())?;
    Ok(subset.to_woff2())
}

fn push_font_face_css(css: &mut String, key: &SvgFontKey, woff2: &[u8]) {
    css.push_str("@font-face {\n");
    css.push_str("  font-family: \"");
    css.push_str(&escape_css_string(&key.family));
    css.push_str("\";\n");
    css.push_str("  font-style: ");
    css.push_str(font_style_css(key.style));
    css.push_str(";\n");
    css.push_str("  font-weight: ");
    css.push_str(&key.weight.to_string());
    css.push_str(";\n");
    css.push_str("  font-display: block;\n");
    css.push_str("  src: url(\"data:font/woff2;base64,");
    css.push_str(&BASE64_STANDARD.encode(woff2));
    css.push_str("\") format(\"woff2\");\n");
    css.push_str("}\n");
}

fn handle_font_issue(message: String, policy: MissingFontPolicy) -> Result<(), AvengerSvgError> {
    match policy {
        MissingFontPolicy::Error => Err(AvengerSvgError::Font(message)),
        MissingFontPolicy::Warn | MissingFontPolicy::Fallback => Ok(()),
    }
}

fn font_weight_number(font_weight: &FontWeight) -> u16 {
    match font_weight {
        FontWeight::Name(FontWeightNameSpec::Normal) => 400,
        FontWeight::Name(FontWeightNameSpec::Bold) => 700,
        FontWeight::Number(weight) => weight.round().clamp(1.0, 1000.0) as u16,
    }
}

impl From<FontStyle> for SvgFontStyle {
    fn from(style: FontStyle) -> Self {
        match style {
            FontStyle::Normal => Self::Normal,
            FontStyle::Italic => Self::Italic,
        }
    }
}

fn font_style_css(style: SvgFontStyle) -> &'static str {
    match style {
        SvgFontStyle::Normal => "normal",
        SvgFontStyle::Italic => "italic",
    }
}

fn escape_css_string(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\n' => "\\a ".chars().collect::<Vec<_>>(),
            '\r' => "\\d ".chars().collect::<Vec<_>>(),
            '\u{000c}' => "\\c ".chars().collect::<Vec<_>>(),
            ch => vec![ch],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_font_family_before_generic_fallbacks() {
        let options = FontResolutionOptions::default();

        let families =
            parse_named_font_families("\"Atkinson Hyperlegible Next\", sans-serif", &options)
                .unwrap();

        assert_eq!(families, vec!["Atkinson Hyperlegible Next"]);
    }

    #[test]
    fn parses_multiple_named_font_family_fallbacks() {
        let options = FontResolutionOptions::default();

        let families = parse_named_font_families(
            "\"Missing Display Face\", \"Atkinson Hyperlegible Next\", sans-serif",
            &options,
        )
        .unwrap();

        assert_eq!(
            families,
            vec!["Missing Display Face", "Atkinson Hyperlegible Next"]
        );
    }

    #[test]
    fn skips_generic_only_font_families() {
        let options = FontResolutionOptions::default();

        let families = parse_named_font_families("sans-serif", &options).unwrap();

        assert!(families.is_empty());
    }

    #[test]
    fn embeds_subset_woff2_css_for_bundled_fonts() {
        let mut collector = SvgFontCollector::default();
        let options = FontResolutionOptions::default();
        collector
            .collect_text(
                "Atkinson Hyperlegible Next",
                &FontWeight::Name(FontWeightNameSpec::Bold),
                &FontStyle::Italic,
                "Axis",
                &options,
            )
            .unwrap();

        let css = collector.font_face_css(&options).unwrap();

        assert!(css.contains("@font-face"));
        assert!(css.contains("font-family: \"Atkinson Hyperlegible Next\";"));
        assert!(css.contains("font-style: italic;"));
        assert!(css.contains("font-weight: 700;"));
        assert!(css.contains("data:font/woff2;base64,"));
    }

    #[test]
    fn resolves_to_first_available_named_font_family() {
        let mut collector = SvgFontCollector::default();
        let options = FontResolutionOptions::default();
        collector
            .collect_text(
                "\"Missing Display Face\", \"Atkinson Hyperlegible Next\", sans-serif",
                &FontWeight::Name(FontWeightNameSpec::Normal),
                &FontStyle::Normal,
                "Axis",
                &options,
            )
            .unwrap();

        let css = collector.font_face_css(&options).unwrap();

        assert!(css.contains("font-family: \"Atkinson Hyperlegible Next\";"));
        assert!(!css.contains("font-family: \"Missing Display Face\";"));
    }

    #[test]
    fn embeds_four_bundled_weight_style_faces_in_deterministic_order() {
        let mut collector = SvgFontCollector::default();
        let options = FontResolutionOptions::default();
        for (weight, style) in [
            (
                FontWeight::Name(FontWeightNameSpec::Bold),
                FontStyle::Italic,
            ),
            (
                FontWeight::Name(FontWeightNameSpec::Normal),
                FontStyle::Italic,
            ),
            (
                FontWeight::Name(FontWeightNameSpec::Bold),
                FontStyle::Normal,
            ),
            (
                FontWeight::Name(FontWeightNameSpec::Normal),
                FontStyle::Normal,
            ),
        ] {
            collector
                .collect_text(
                    "Atkinson Hyperlegible Next",
                    &weight,
                    &style,
                    "Axis",
                    &options,
                )
                .unwrap();
        }

        let css = collector.font_face_css(&options).unwrap();
        let regular = css
            .find("font-style: normal;\n  font-weight: 400;")
            .unwrap();
        let italic = css
            .find("font-style: italic;\n  font-weight: 400;")
            .unwrap();
        let bold = css
            .find("font-style: normal;\n  font-weight: 700;")
            .unwrap();
        let bold_italic = css
            .find("font-style: italic;\n  font-weight: 700;")
            .unwrap();

        assert_eq!(css.matches("@font-face").count(), 4);
        assert!(regular < italic);
        assert!(italic < bold);
        assert!(bold < bold_italic);
    }

    #[test]
    fn missing_font_policy_controls_missing_svg_font_errors() {
        for (policy, should_error) in [
            (MissingFontPolicy::Error, true),
            (MissingFontPolicy::Warn, false),
            (MissingFontPolicy::Fallback, false),
        ] {
            let mut collector = SvgFontCollector::default();
            let options = FontResolutionOptions {
                missing_font: policy,
                ..Default::default()
            };
            collector
                .collect_text(
                    "Missing Display Face",
                    &FontWeight::Name(FontWeightNameSpec::Normal),
                    &FontStyle::Normal,
                    "Axis",
                    &options,
                )
                .unwrap();

            let result = collector.font_face_css(&options);

            assert_eq!(result.is_err(), should_error);
            if !should_error {
                assert_eq!(result.unwrap(), "");
            }
        }
    }
}
