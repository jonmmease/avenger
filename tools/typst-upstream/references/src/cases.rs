//! Fixture manifests and the page wrapper.

use std::{fmt::Write, fs, path::Path};

use serde::Deserialize;

use crate::Result;

/// A fixture directory's `cases.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    defaults: Defaults,
    #[serde(rename = "case")]
    pub cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Defaults {
    text_font: String,
    math_font: String,
    font_size: f64,
    font_weight: u16,
}

/// One label. Optional fields override the manifest defaults.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub source: String,
    /// Upstream test names (unique across `tests/suite`) the case reduces. Provenance only.
    #[serde(default)]
    #[allow(dead_code)]
    pub upstream_tests: Vec<String>,
    /// Free-form provenance or intent. Ignored by the generator.
    #[allow(dead_code)]
    pub note: Option<String>,
    text_font: Option<String>,
    math_font: Option<String>,
    font_size: Option<f64>,
    font_weight: Option<u16>,
    lang: Option<String>,
    region: Option<String>,
    dir: Option<String>,
}

/// A case wrapped in a page, and where the label source starts inside it.
pub struct Wrapped {
    pub text: String,
    pub offset: usize,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let manifest: Self =
            toml::from_str(&text).map_err(|err| format!("{}: {err}", path.display()))?;
        let mut ids = std::collections::HashSet::new();
        for case in &manifest.cases {
            if !ids.insert(case.id.as_str()) {
                return Err(format!("{}: duplicate case id {}", path.display(), case.id).into());
            }
        }
        Ok(manifest)
    }

    /// Wraps a label in a page that fits one box, so the box frame is the label's frame.
    ///
    /// The equation and raw rules mirror how `avenger-typst-label` resolves its math and
    /// monospace families.
    pub fn wrap(&self, case: &Case) -> Wrapped {
        let defaults = &self.defaults;
        let text_font = case.text_font.as_ref().unwrap_or(&defaults.text_font);
        let math_font = case.math_font.as_ref().unwrap_or(&defaults.math_font);
        let size = case.font_size.unwrap_or(defaults.font_size);
        let weight = case.font_weight.unwrap_or(defaults.font_weight);

        let mut text = String::from("#set page(width: auto, height: auto, margin: 0pt)\n");
        write!(
            text,
            "#set text(font: {text_font:?}, size: {size}pt, weight: {weight}"
        )
        .unwrap();
        if let Some(lang) = &case.lang {
            write!(text, ", lang: {lang:?}").unwrap();
        }
        if let Some(region) = &case.region {
            write!(text, ", region: {region:?}").unwrap();
        }
        if let Some(dir) = &case.dir {
            write!(text, ", dir: {dir}").unwrap();
        }
        text.push_str(")\n");
        writeln!(
            text,
            "#show math.equation: set text(font: {math_font:?}, weight: {weight})"
        )
        .unwrap();
        text.push_str("#show raw: set text(font: \"DejaVu Sans Mono\")\n");
        text.push_str("#box[");
        let offset = text.len();
        text.push_str(&case.source);
        text.push_str("]\n");
        Wrapped { text, offset }
    }
}
