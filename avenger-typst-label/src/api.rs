use crate::delimiter::{ParsedSegment, parse_segments};
use crate::engine::engine::TypstEngineCore;
use crate::error::{MathTypesetError, TypstInitError};
use crate::limits::MathLimits;
use crate::style::{MathFontConfig, MathStrictness};
use crate::types::{MathSyntaxMode, TextLineArtifact, TextLineOptions};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TypstCacheConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TypstEngineConfig {
    pub font_config: MathFontConfig,
    pub cache: TypstCacheConfig,
    pub strictness: MathStrictness,
}

impl Default for TypstEngineConfig {
    fn default() -> Self {
        Self {
            font_config: MathFontConfig::default(),
            cache: TypstCacheConfig::default(),
            strictness: MathStrictness::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AvengerTypst {
    engine: TypstEngineCore,
}

impl AvengerTypst {
    pub fn new(config: TypstEngineConfig) -> Result<Self, TypstInitError> {
        Ok(Self {
            engine: TypstEngineCore::new(&config)?,
        })
    }

    pub fn typeset_text_line(
        &self,
        source: &str,
        options: &TextLineOptions,
    ) -> Result<TextLineArtifact, MathTypesetError> {
        validate_source_limits(source, options.limits)?;
        if matches!(options.syntax, MathSyntaxMode::TypstFragmentStrict) {
            let segments = parse_segments(source, &options.delimiters)?;
            validate_math_segments(&segments, options.limits)?;
        }

        self.engine.typeset_text_line(source, options)
    }
}

fn validate_source_limits(source: &str, limits: MathLimits) -> Result<(), MathTypesetError> {
    if source.len() > limits.max_source_bytes {
        return Err(MathTypesetError::SourceTooLarge {
            actual: source.len(),
            limit: limits.max_source_bytes,
        });
    }
    Ok(())
}

fn validate_math_segments(
    segments: &[ParsedSegment],
    limits: MathLimits,
) -> Result<(), MathTypesetError> {
    let math_span_count = segments
        .iter()
        .filter(|segment| matches!(segment, ParsedSegment::Math { .. }))
        .count();
    if math_span_count > limits.max_math_spans {
        return Err(MathTypesetError::TooManyMathSpans {
            actual: math_span_count,
            limit: limits.max_math_spans,
        });
    }

    for segment in segments {
        if let ParsedSegment::Math {
            source,
            source_range,
            ..
        } = segment
        {
            validate_math_fragment(source, limits, source_range.clone())?;
        }
    }

    Ok(())
}

fn validate_math_fragment(
    source: &str,
    limits: MathLimits,
    range: std::ops::Range<usize>,
) -> Result<(), MathTypesetError> {
    if source.trim().is_empty() {
        return Err(MathTypesetError::EmptyMathFragment {
            start: range.start,
            end: range.end,
        });
    }

    strict_hash_precheck(source, range.start)?;
    let depth = max_grouping_depth(source);
    if depth > limits.max_math_depth {
        return Err(MathTypesetError::MathDepthExceeded {
            actual: depth,
            limit: limits.max_math_depth,
        });
    }
    validate_math_parse(source, range.start)?;

    Ok(())
}

fn strict_hash_precheck(source: &str, offset: usize) -> Result<(), MathTypesetError> {
    let mut escaped = false;
    for (idx, ch) in source.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '#' {
            return Err(MathTypesetError::UnsupportedSyntax {
                position: offset + idx,
                message: "embedded Typst code is not allowed in math fragments",
            });
        }
    }
    Ok(())
}

fn validate_math_parse(source: &str, offset: usize) -> Result<(), MathTypesetError> {
    crate::engine::math::syntax::parse_math(source, offset).map(|_| ())
}

fn max_grouping_depth(source: &str) -> usize {
    let mut escaped = false;
    let mut depth = 0usize;
    let mut max_depth = 0usize;
    for ch in source.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match ch {
            '(' | '[' | '{' => {
                depth += 1;
                max_depth = max_depth.max(depth);
            }
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max_depth
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TextLineOutputRequest;

    #[test]
    fn default_backend_is_constructible() {
        let engine = AvengerTypst::new(TypstEngineConfig::default());
        assert!(engine.is_ok());
    }

    #[test]
    fn fragment_rejects_hash_code() {
        let engine = AvengerTypst::new(TypstEngineConfig::default()).unwrap();
        let err = engine
            .typeset_text_line(
                "$#let x = 1$",
                &TextLineOptions {
                    outputs: TextLineOutputRequest {
                        paths: false,
                        raster: None,
                        pdf_text_layer: false,
                        positioned_runs: false,
                    },
                    ..TextLineOptions::default()
                },
            )
            .unwrap_err();
        assert!(matches!(
            err,
            MathTypesetError::UnsupportedSyntax { position: 1, .. }
        ));
    }
}
