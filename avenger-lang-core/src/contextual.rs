//! Language-owned contextual scalar accesses.
//!
//! Resolution owns the exact SQL AST shapes. This small inventory lets the
//! compiler, analysis layer, and documentation share the canonical spellings,
//! legal frames, result types, and meanings.

use crate::PhysicalType;

/// The scalar-expression frame in which a contextual access is meaningful.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextualAccessContext {
    MarkChannel,
    Event,
    BetweenEvent,
    LegendEvent,
    ItemFrame,
    InlineView,
}

/// One canonical contextual-access family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextualAccessSignature {
    /// Canonical source pattern. Angle-bracket components are semantic
    /// placeholders rather than literal source syntax.
    pub pattern: &'static str,
    pub context: ContextualAccessContext,
    /// Fixed Arrow type when the family has one. Schema-derived families use
    /// `None` and publish their exact type from semantic analysis.
    pub fixed_arrow_type: Option<ContextualAccessPhysicalType>,
    pub nullable: bool,
    pub docs: &'static str,
}

/// Fixed physical types used by contextual accesses.
///
/// Keeping this typed avoids making the compiler parse prose-oriented type
/// strings while preserving one shared inventory for diagnostics and editor
/// presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextualAccessPhysicalType {
    Float32,
    Float64,
    Utf8,
    UInt32,
    Float64List,
}

impl ContextualAccessPhysicalType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Float32 => "float32",
            Self::Float64 => "float64",
            Self::Utf8 => "utf8",
            Self::UInt32 => "uint32",
            Self::Float64List => "list(float64)",
        }
    }

    pub fn physical_type(self) -> PhysicalType {
        match self {
            Self::Float32 => PhysicalType::Float32,
            Self::Float64 => PhysicalType::Float64,
            Self::Utf8 => PhysicalType::Utf8,
            Self::UInt32 => PhysicalType::UInt32,
            Self::Float64List => PhysicalType::List(Box::new(PhysicalType::Float64)),
        }
    }
}

/// Normative v1 contextual-access inventory.
pub const CONTEXTUAL_ACCESS_SIGNATURES: &[ContextualAccessSignature] = &[
    ContextualAccessSignature {
        pattern: "channel.<channel>",
        context: ContextualAccessContext::MarkChannel,
        fixed_arrow_type: None,
        nullable: true,
        docs: "Another evaluated channel on the current mark.",
    },
    ContextualAccessSignature {
        pattern: "datum.\"<field>\"",
        context: ContextualAccessContext::Event,
        fixed_arrow_type: None,
        nullable: true,
        docs: "A logical pre-scale field from the hit mark row.",
    },
    ContextualAccessSignature {
        pattern: "event.coord.<channel>",
        context: ContextualAccessContext::Event,
        fixed_arrow_type: Some(ContextualAccessPhysicalType::Float64),
        nullable: true,
        docs: "The current event coordinate in the channel's scale space.",
    },
    ContextualAccessSignature {
        pattern: "event.start.coord.<channel>",
        context: ContextualAccessContext::BetweenEvent,
        fixed_arrow_type: Some(ContextualAccessPhysicalType::Float64),
        nullable: true,
        docs: "The gesture-start coordinate in the channel's scale space.",
    },
    ContextualAccessSignature {
        pattern: "event.domain.<channel>.start",
        context: ContextualAccessContext::Event,
        fixed_arrow_type: Some(ContextualAccessPhysicalType::Float64),
        nullable: true,
        docs: "The start of the event-time scale domain.",
    },
    ContextualAccessSignature {
        pattern: "event.domain.<channel>.end",
        context: ContextualAccessContext::Event,
        fixed_arrow_type: Some(ContextualAccessPhysicalType::Float64),
        nullable: true,
        docs: "The end of the event-time scale domain.",
    },
    ContextualAccessSignature {
        pattern: "event.path",
        context: ContextualAccessContext::BetweenEvent,
        fixed_arrow_type: Some(ContextualAccessPhysicalType::Float64List),
        nullable: true,
        docs: "The accumulated coordinate path of a between interaction.",
    },
    ContextualAccessSignature {
        pattern: "event.facet[n]",
        context: ContextualAccessContext::Event,
        fixed_arrow_type: Some(ContextualAccessPhysicalType::Utf8),
        nullable: true,
        docs: "A one-based logical facet-path component.",
    },
    ContextualAccessSignature {
        pattern: "event.legend.value",
        context: ContextualAccessContext::LegendEvent,
        fixed_arrow_type: Some(ContextualAccessPhysicalType::Utf8),
        nullable: true,
        docs: "The value under a continuous legend surface event.",
    },
    ContextualAccessSignature {
        pattern: "item.channel.<channel>",
        context: ContextualAccessContext::ItemFrame,
        fixed_arrow_type: None,
        nullable: true,
        docs: "An evaluated channel from the source item frame.",
    },
    ContextualAccessSignature {
        pattern: "item.data.\"<field>\"",
        context: ContextualAccessContext::ItemFrame,
        fixed_arrow_type: None,
        nullable: true,
        docs: "A physical Arrow field from the source item's logical row.",
    },
    ContextualAccessSignature {
        pattern: "item.bbox.<edge>",
        context: ContextualAccessContext::ItemFrame,
        fixed_arrow_type: Some(ContextualAccessPhysicalType::Float32),
        nullable: true,
        docs: "An edge of the source item's evaluated bounding box.",
    },
    ContextualAccessSignature {
        pattern: "<view>.x.domain.start",
        context: ContextualAccessContext::InlineView,
        fixed_arrow_type: Some(ContextualAccessPhysicalType::Float64),
        nullable: false,
        docs: "The start of an inline view's x domain.",
    },
    ContextualAccessSignature {
        pattern: "<view>.x.domain.end",
        context: ContextualAccessContext::InlineView,
        fixed_arrow_type: Some(ContextualAccessPhysicalType::Float64),
        nullable: false,
        docs: "The end of an inline view's x domain.",
    },
    ContextualAccessSignature {
        pattern: "<view>.y.domain.start",
        context: ContextualAccessContext::InlineView,
        fixed_arrow_type: Some(ContextualAccessPhysicalType::Float64),
        nullable: false,
        docs: "The start of an inline view's y domain.",
    },
    ContextualAccessSignature {
        pattern: "<view>.y.domain.end",
        context: ContextualAccessContext::InlineView,
        fixed_arrow_type: Some(ContextualAccessPhysicalType::Float64),
        nullable: false,
        docs: "The end of an inline view's y domain.",
    },
    ContextualAccessSignature {
        pattern: "<view>.x.pixels",
        context: ContextualAccessContext::InlineView,
        fixed_arrow_type: Some(ContextualAccessPhysicalType::UInt32),
        nullable: false,
        docs: "An inline view's x pixel count.",
    },
    ContextualAccessSignature {
        pattern: "<view>.y.pixels",
        context: ContextualAccessContext::InlineView,
        fixed_arrow_type: Some(ContextualAccessPhysicalType::UInt32),
        nullable: false,
        docs: "An inline view's y pixel count.",
    },
];

pub fn contextual_access_signature(pattern: &str) -> Option<&'static ContextualAccessSignature> {
    CONTEXTUAL_ACCESS_SIGNATURES
        .iter()
        .find(|signature| signature.pattern == pattern)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{CONTEXTUAL_ACCESS_SIGNATURES, contextual_access_signature};
    #[test]
    fn contextual_signature_inventory_is_unique_and_physically_typed() {
        let mut patterns = BTreeSet::new();
        for signature in CONTEXTUAL_ACCESS_SIGNATURES {
            assert!(
                patterns.insert(signature.pattern),
                "duplicate contextual signature `{}`",
                signature.pattern
            );
            assert_eq!(
                contextual_access_signature(signature.pattern),
                Some(signature)
            );
            if let Some(data_type) = signature.fixed_arrow_type {
                assert_eq!(
                    data_type.physical_type().to_string(),
                    data_type.as_str(),
                    "noncanonical fixed type for `{}`",
                    signature.pattern
                );
            }
        }
    }
}
