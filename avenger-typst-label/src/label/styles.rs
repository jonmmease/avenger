//! A label's root styles: its options as the style properties of the pipeline.

use super::options::{LabelOptions, TextDir};
use super::world::LabelWorld;
use crate::typst_library::foundations::{Smart, Styles};
use crate::typst_library::layout::{Abs, Dir, Em};
use crate::typst_library::math::{EquationElem, LabelMathStyle};
use crate::typst_library::text::{
    FontFamily, FontList, RawElem, TextDir as TextDirection, TextElem, TextSize,
};
use crate::typst_library::visualize::Paint;

/// Upstream's emoji fallback families, in its order.
const EMOJI: [&str; 4] =
    ["twitter color emoji", "noto color emoji", "apple color emoji", "segoe ui emoji"];

/// The families an engine falls back to, from its font options.
#[derive(Debug, Clone)]
pub(crate) struct Defaults {
    /// The family `sans-serif` names, which text and math fall back to first.
    sans: String,
    /// The family of raw text, if the engine sets one.
    monospace: Option<String>,
    /// The math family of labels that name none, which math falls back to first.
    math: Option<String>,
}

impl Defaults {
    /// The engine's default families.
    pub fn new(
        world: &LabelWorld,
        monospace: Option<String>,
        math: Option<String>,
    ) -> Self {
        let sans = world.families("sans-serif").remove(0);
        Self { sans, monospace, math }
    }

    /// The family `sans-serif` names.
    pub fn sans(&self) -> &str {
        &self.sans
    }
}

/// The root styles of a label.
// avenger: in place of a document's `set` rules and its equations' `show` rules. Text falls
// back to the engine's sans-serif family, then upstream's emoji families; math to the
// engine's math family first.
pub(crate) fn root_styles(
    world: &LabelWorld,
    defaults: &Defaults,
    options: &LabelOptions,
) -> Styles {
    let text = &options.text;
    let list = |families: Vec<String>| {
        FontList(families.iter().map(|family| FontFamily::new(family)).collect())
    };
    let fallbacks = |first: &[&str]| {
        FontList(
            first
                .iter()
                .chain(&EMOJI)
                .map(|family| FontFamily::new(family))
                .collect(),
        )
    };

    let mut styles = Styles::new();
    styles.set(TextElem::font, list(world.families(&text.font_family)));
    styles.set(TextElem::fallbacks, fallbacks(&[defaults.sans.as_str()]));
    styles.set(TextElem::size, TextSize(Abs::pt(f64::from(text.font_size)).into()));
    styles.set(TextElem::fill, Paint::Solid(text.fill));
    styles.set(TextElem::weight, text.font_weight);
    styles.set(TextElem::style, text.font_style);
    styles.set(TextElem::lang, text.lang);
    styles.set(TextElem::region, text.region);
    styles.set(
        TextElem::dir,
        TextDirection(match text.dir {
            TextDir::Auto => Smart::Auto,
            TextDir::Ltr => Smart::Custom(Dir::LTR),
            TextDir::Rtl => Smart::Custom(Dir::RTL),
        }),
    );

    if let Some(monospace) = &defaults.monospace {
        styles.set(RawElem::label_font, Some(list(vec![monospace.clone()])));
    }

    let math = &options.math;
    let font = if math.font_family.trim().is_empty() {
        defaults.math.clone().map(|family| list(vec![family]))
    } else {
        Some(list(world.families(&math.font_family)))
    };
    styles.set(
        EquationElem::label_style,
        LabelMathStyle {
            font,
            weight: math.font_weight,
            size: math.font_size.map(|size| Em::new(f64::from(size.0))),
            fill: math.fill.map(Paint::Solid),
        },
    );
    let mut first = vec![];
    first.extend(defaults.math.as_deref());
    first.push(defaults.sans.as_str());
    styles.set(EquationElem::fallbacks, fallbacks(&first));

    styles
}
