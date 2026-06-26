use crate::delimiter::{parse_segments, ParsedSegment};
use crate::engine::mock::MockMathEngine;
use crate::error::{MathTypesetError, TypstInitError};
use crate::limits::MathLimits;
use crate::pdf::{MathFontResource, MathFontResourceId};
use crate::style::{MathFontConfig, MathStrictness};
use crate::types::{
    MathFragmentOptions, MathRun, MathRunArtifact, MathStringArtifact, MathStringOptions,
    MathStringRun, PlainTextRun, TextLineArtifact, TextLineOptions,
};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum TypstEngineBackend {
    DeterministicMock,
    VendorTypst,
    OwnedTypst,
}

impl Default for TypstEngineBackend {
    fn default() -> Self {
        Self::DeterministicMock
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TypstCacheConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TypstEngineConfig {
    pub backend: TypstEngineBackend,
    pub font_config: MathFontConfig,
    pub cache: TypstCacheConfig,
    pub strictness: MathStrictness,
}

impl Default for TypstEngineConfig {
    fn default() -> Self {
        Self {
            backend: TypstEngineBackend::default(),
            font_config: MathFontConfig::default(),
            cache: TypstCacheConfig::default(),
            strictness: MathStrictness::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AvengerTypst {
    engine: EngineInner,
}

#[derive(Debug, Clone)]
enum EngineInner {
    Mock(MockMathEngine),
    #[cfg(feature = "vendor-typst")]
    Typst(crate::engine::typst::TypstMathEngine),
    #[cfg(feature = "owned")]
    Owned(crate::owned::engine::OwnedTypstEngine),
}

impl AvengerTypst {
    pub fn new(config: TypstEngineConfig) -> Result<Self, TypstInitError> {
        match config.backend {
            TypstEngineBackend::DeterministicMock => Ok(Self {
                engine: EngineInner::Mock(MockMathEngine),
            }),
            TypstEngineBackend::VendorTypst => new_vendor_typst_engine(config),
            TypstEngineBackend::OwnedTypst => new_owned_typst_engine(config),
        }
    }

    pub fn typeset_math_fragment(
        &self,
        source: &str,
        options: &MathFragmentOptions,
    ) -> Result<MathRunArtifact, MathTypesetError> {
        validate_source_limits(source, options.limits)?;
        validate_math_fragment(source, options.limits, 0..source.len())?;
        match &self.engine {
            EngineInner::Mock(engine) => engine.typeset_fragment(source, options),
            #[cfg(feature = "vendor-typst")]
            EngineInner::Typst(engine) => engine.typeset_fragment(source, options),
            #[cfg(feature = "owned")]
            EngineInner::Owned(engine) => engine.typeset_fragment(source, options),
        }
    }

    pub fn typeset_math_string(
        &self,
        source: &str,
        options: &MathStringOptions,
    ) -> Result<MathStringArtifact, MathTypesetError> {
        validate_source_limits(source, options.limits)?;
        let segments = parse_segments(source, &options.delimiters)?;
        let math_span_count = segments
            .iter()
            .filter(|segment| matches!(segment, ParsedSegment::Math { .. }))
            .count();
        if math_span_count > options.limits.max_math_spans {
            return Err(MathTypesetError::TooManyMathSpans {
                actual: math_span_count,
                limit: options.limits.max_math_spans,
            });
        }

        let mut runs = Vec::new();
        let mut font_resources = Vec::new();
        let mut warnings = Vec::new();

        for segment in segments {
            match segment {
                ParsedSegment::Plain { text, range } => {
                    runs.push(MathStringRun::Plain(PlainTextRun {
                        text,
                        byte_range: range,
                        style: options.text_style.clone(),
                    }));
                }
                ParsedSegment::Math {
                    source: math_source,
                    source_range,
                    delimiter,
                } => {
                    validate_math_fragment(&math_source, options.limits, source_range.clone())?;
                    let fragment_options = MathFragmentOptions {
                        style: options.math_style.clone(),
                        outputs: options.outputs.clone(),
                        syntax: options.syntax,
                        limits: options.limits,
                    };
                    let mut artifact = self
                        .typeset_math_fragment(&math_source, &fragment_options)
                        .map_err(|err| map_fragment_error(err, source_range.clone()))?;
                    let font_id_map =
                        merge_font_resources(&mut font_resources, &artifact.font_resources);
                    remap_artifact_font_ids(&mut artifact, &font_id_map);
                    warnings.extend(artifact.warnings.iter().cloned());
                    runs.push(MathStringRun::Math(MathRun {
                        source: math_source,
                        byte_range: source_range,
                        delimiter,
                        artifact,
                    }));
                }
            }
        }

        Ok(MathStringArtifact {
            source: source.to_string(),
            runs,
            font_resources,
            warnings,
        })
    }

    pub fn typeset_text_line(
        &self,
        source: &str,
        options: &TextLineOptions,
    ) -> Result<TextLineArtifact, MathTypesetError> {
        validate_source_limits(source, options.limits)?;
        let segments = parse_segments(source, &options.delimiters)?;
        validate_math_segments(&segments, options.limits)?;

        match &self.engine {
            EngineInner::Mock(engine) => engine.typeset_text_line(source, options),
            #[cfg(feature = "vendor-typst")]
            EngineInner::Typst(engine) => engine.typeset_text_line(source, options),
            #[cfg(feature = "owned")]
            EngineInner::Owned(engine) => engine.typeset_text_line(source, options),
        }
    }
}

#[cfg(feature = "vendor-typst")]
fn new_vendor_typst_engine(config: TypstEngineConfig) -> Result<AvengerTypst, TypstInitError> {
    Ok(AvengerTypst {
        engine: EngineInner::Typst(crate::engine::typst::TypstMathEngine::new(&config)?),
    })
}

#[cfg(not(feature = "vendor-typst"))]
fn new_vendor_typst_engine(_config: TypstEngineConfig) -> Result<AvengerTypst, TypstInitError> {
    Err(TypstInitError::BackendUnavailable(
        "vendor-typst feature is not enabled",
    ))
}

#[cfg(feature = "owned")]
fn new_owned_typst_engine(config: TypstEngineConfig) -> Result<AvengerTypst, TypstInitError> {
    Ok(AvengerTypst {
        engine: EngineInner::Owned(crate::owned::engine::OwnedTypstEngine::new(&config)?),
    })
}

#[cfg(not(feature = "owned"))]
fn new_owned_typst_engine(_config: TypstEngineConfig) -> Result<AvengerTypst, TypstInitError> {
    Err(TypstInitError::BackendUnavailable(
        "owned Typst backend requires the owned feature",
    ))
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
    validate_vendor_typst_parse(source, range.start)?;

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

#[cfg(feature = "vendor-typst")]
fn validate_vendor_typst_parse(source: &str, offset: usize) -> Result<(), MathTypesetError> {
    use typst_syntax::{LinkedNode, SyntaxKind};

    let root = typst_syntax::parse_math(source);
    let linked_root = LinkedNode::new(&root);
    if let Some(position) = first_node_position(&linked_root, |kind| kind == SyntaxKind::Error) {
        return Err(MathTypesetError::UnsupportedSyntax {
            position: offset + position,
            message: "invalid Typst math syntax",
        });
    }

    if let Some((position, message)) = first_unsupported_typst_node(&linked_root) {
        return Err(MathTypesetError::UnsupportedSyntax {
            position: offset + position,
            message,
        });
    }

    Ok(())
}

#[cfg(not(feature = "vendor-typst"))]
fn validate_vendor_typst_parse(_source: &str, _offset: usize) -> Result<(), MathTypesetError> {
    Ok(())
}

#[cfg(feature = "vendor-typst")]
fn first_node_position(
    node: &typst_syntax::LinkedNode<'_>,
    predicate: impl Copy + Fn(typst_syntax::SyntaxKind) -> bool,
) -> Option<usize> {
    if predicate(node.get().kind()) {
        return Some(node.range().start);
    }

    node.children()
        .find_map(|child| first_node_position(&child, predicate))
}

#[cfg(feature = "vendor-typst")]
fn first_unsupported_typst_node(
    node: &typst_syntax::LinkedNode<'_>,
) -> Option<(usize, &'static str)> {
    if let Some(message) = unsupported_typst_math_kind(node.get().kind()) {
        return Some((node.range().start, message));
    }

    node.children()
        .find_map(|child| first_unsupported_typst_node(&child))
}

#[cfg(feature = "vendor-typst")]
fn unsupported_typst_math_kind(kind: typst_syntax::SyntaxKind) -> Option<&'static str> {
    use typst_syntax::SyntaxKind;

    match kind {
        SyntaxKind::Hash
        | SyntaxKind::Code
        | SyntaxKind::CodeBlock
        | SyntaxKind::Parenthesized
        | SyntaxKind::Array
        | SyntaxKind::Dict
        | SyntaxKind::Keyed
        | SyntaxKind::Unary
        | SyntaxKind::Binary
        | SyntaxKind::FieldAccess
        | SyntaxKind::FuncCall
        | SyntaxKind::Args
        | SyntaxKind::Closure
        | SyntaxKind::Params
        | SyntaxKind::LetBinding
        | SyntaxKind::SetRule
        | SyntaxKind::ShowRule
        | SyntaxKind::Contextual
        | SyntaxKind::Conditional
        | SyntaxKind::WhileLoop
        | SyntaxKind::ForLoop
        | SyntaxKind::ModuleImport
        | SyntaxKind::ImportItems
        | SyntaxKind::ImportItemPath
        | SyntaxKind::RenamedImportItem
        | SyntaxKind::ModuleInclude
        | SyntaxKind::LoopBreak
        | SyntaxKind::LoopContinue
        | SyntaxKind::FuncReturn
        | SyntaxKind::Destructuring
        | SyntaxKind::DestructAssignment
        | SyntaxKind::Let
        | SyntaxKind::Set
        | SyntaxKind::Show
        | SyntaxKind::Context
        | SyntaxKind::If
        | SyntaxKind::Else
        | SyntaxKind::For
        | SyntaxKind::In
        | SyntaxKind::While
        | SyntaxKind::Break
        | SyntaxKind::Continue
        | SyntaxKind::Return
        | SyntaxKind::Import
        | SyntaxKind::Include
        | SyntaxKind::As => Some("embedded Typst code is not allowed in math fragments"),

        SyntaxKind::Markup
        | SyntaxKind::Text
        | SyntaxKind::Linebreak
        | SyntaxKind::Parbreak
        | SyntaxKind::SmartQuote
        | SyntaxKind::Strong
        | SyntaxKind::Emph
        | SyntaxKind::Raw
        | SyntaxKind::RawLang
        | SyntaxKind::RawDelim
        | SyntaxKind::RawTrimmed
        | SyntaxKind::Link
        | SyntaxKind::Label
        | SyntaxKind::Ref
        | SyntaxKind::RefMarker
        | SyntaxKind::Heading
        | SyntaxKind::HeadingMarker
        | SyntaxKind::ListItem
        | SyntaxKind::ListMarker
        | SyntaxKind::EnumItem
        | SyntaxKind::EnumMarker
        | SyntaxKind::TermItem
        | SyntaxKind::TermMarker
        | SyntaxKind::ContentBlock
        | SyntaxKind::Equation
        | SyntaxKind::Dollar => Some("non-math Typst content is not allowed in math fragments"),

        _ => None,
    }
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

fn merge_font_resources(
    target: &mut Vec<MathFontResource>,
    incoming: &[MathFontResource],
) -> Vec<(MathFontResourceId, MathFontResourceId)> {
    let mut id_map = Vec::with_capacity(incoming.len());

    for resource in incoming {
        let target_id =
            if let Some(existing) = target.iter().find(|existing| same_font(existing, resource)) {
                existing.id
            } else {
                let target_id = MathFontResourceId(target.len() as u32);
                let mut resource = resource.clone();
                resource.id = target_id;
                target.push(resource);
                target_id
            };
        id_map.push((resource.id, target_id));
    }

    id_map
}

fn remap_artifact_font_ids(
    artifact: &mut MathRunArtifact,
    id_map: &[(MathFontResourceId, MathFontResourceId)],
) {
    for resource in &mut artifact.font_resources {
        if let Some(target_id) = remapped_font_id(resource.id, id_map) {
            resource.id = target_id;
        }
    }

    if let Some(pdf_text) = &mut artifact.pdf_text {
        for glyph_run in &mut pdf_text.glyph_runs {
            if let Some(target_id) = remapped_font_id(glyph_run.font, id_map) {
                glyph_run.font = target_id;
            }
        }
    }
}

fn remapped_font_id(
    font_id: MathFontResourceId,
    id_map: &[(MathFontResourceId, MathFontResourceId)],
) -> Option<MathFontResourceId> {
    id_map
        .iter()
        .find_map(|(source_id, target_id)| (*source_id == font_id).then_some(*target_id))
}

fn same_font(a: &MathFontResource, b: &MathFontResource) -> bool {
    a.family == b.family
        && a.postscript_name == b.postscript_name
        && a.face_index == b.face_index
        && (a.units_per_em - b.units_per_em).abs() < f32::EPSILON
        && a.data == b.data
}

fn map_fragment_error(err: MathTypesetError, range: std::ops::Range<usize>) -> MathTypesetError {
    match err {
        MathTypesetError::EmptyMathFragment { .. } => MathTypesetError::EmptyMathFragment {
            start: range.start,
            end: range.end,
        },
        MathTypesetError::UnsupportedSyntax { position, message } => {
            MathTypesetError::UnsupportedSyntax {
                position: range.start + position,
                message,
            }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_backend_is_constructible() {
        let engine = AvengerTypst::new(TypstEngineConfig::default());
        assert!(engine.is_ok());
    }

    #[test]
    fn fragment_rejects_hash_code() {
        let engine = AvengerTypst::new(TypstEngineConfig::default()).unwrap();
        let err = engine
            .typeset_math_fragment("#let x = 1", &MathFragmentOptions::default())
            .unwrap_err();
        assert!(matches!(
            err,
            MathTypesetError::UnsupportedSyntax { position: 0, .. }
        ));
    }
}
