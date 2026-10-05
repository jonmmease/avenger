use crate::{data::Context, locale::Signs};
use fixed_decimal::Sign;
use icu_experimental::{dimension::provider::percent::PercentEssentialsV1, provider::Baked};
use icu_pattern::{DoublePlaceholderKey, PatternItem};
use icu_provider::prelude::*;

/// ICU percent placement with skeleton-selected signs and unit symbols.
#[derive(Debug)]
pub(crate) struct Percent {
    patterns: DataPayload<PercentEssentialsV1>,
    /// The locale's Latin percent sign, which ICU4X embeds in its patterns (upstream issue #3838).
    source: &'static str,
    unit: &'static str,
}

impl Percent {
    pub fn new(context: &Context, unit: &'static str) -> Result<Self, DataError> {
        Ok(Self {
            patterns: context.load::<PercentEssentialsV1, _>(&Baked)?,
            source: context.symbols_for("latn").map_or("%", |s| s.percent),
            unit,
        })
    }

    pub fn render(&self, body: &str, sign: Sign, signs: &Signs) -> String {
        let data = self.patterns.get();
        // The unsigned pattern's only placeholder is the number.
        let items: Vec<_> = if sign == Sign::None {
            data.unsigned_pattern
                .iter()
                .map(|item| match item {
                    PatternItem::Literal(s) => PatternItem::Literal(s),
                    PatternItem::Placeholder(_) => {
                        PatternItem::Placeholder(DoublePlaceholderKey::Place0)
                    }
                })
                .collect()
        } else {
            data.signed_pattern.iter().collect()
        };
        items
            .into_iter()
            .map(|item| match item {
                // Replace the embedded symbol for numbering-system overrides and per-mille.
                PatternItem::Literal(s) => s.replace(self.source, self.unit),
                PatternItem::Placeholder(DoublePlaceholderKey::Place0) => body.into(),
                PatternItem::Placeholder(DoublePlaceholderKey::Place1) => signs.symbol(sign),
            })
            .collect()
    }
}
