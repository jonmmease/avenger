//! Language-owned operations that retain function-call syntax.

/// Authored expression surface on which an intrinsic operation is legal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntrinsicOperationContext {
    EventExpression,
    SceneGeometry,
}

/// Semantic argument category used for resolution and editor presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntrinsicOperationArgumentKind {
    Selection,
    DatumField,
    NumericExpression,
    PathExpression,
}

impl IntrinsicOperationArgumentKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Selection => "selection",
            Self::DatumField => "datum field",
            Self::NumericExpression => "numeric expression",
            Self::PathExpression => "path expression",
        }
    }
}

/// Result boundary for an intrinsic operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntrinsicOperationResult {
    Boolean,
    Float64List,
    SceneGeometry,
}

impl IntrinsicOperationResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::Float64List => "list(float64)",
            Self::SceneGeometry => "scene geometry",
        }
    }

    pub const fn arrow_type(self) -> Option<&'static str> {
        match self {
            Self::Boolean => Some("boolean"),
            Self::Float64List => Some("list(float64)"),
            Self::SceneGeometry => None,
        }
    }
}

/// One canonical intrinsic-operation signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntrinsicOperationSignature {
    pub name: &'static str,
    pub arguments: &'static [IntrinsicOperationArgumentKind],
    pub contexts: &'static [IntrinsicOperationContext],
    pub result: IntrinsicOperationResult,
    pub docs: &'static str,
}

/// Normative v1 operation inventory.
pub const INTRINSIC_OPERATION_SIGNATURES: &[IntrinsicOperationSignature] = &[
    IntrinsicOperationSignature {
        name: "selection_contains",
        arguments: &[
            IntrinsicOperationArgumentKind::Selection,
            IntrinsicOperationArgumentKind::DatumField,
        ],
        contexts: &[IntrinsicOperationContext::EventExpression],
        result: IntrinsicOperationResult::Boolean,
        docs: "Test whether a contextual datum field belongs to a selection.",
    },
    IntrinsicOperationSignature {
        name: "span",
        arguments: &[
            IntrinsicOperationArgumentKind::NumericExpression,
            IntrinsicOperationArgumentKind::NumericExpression,
        ],
        contexts: &[IntrinsicOperationContext::EventExpression],
        result: IntrinsicOperationResult::Float64List,
        docs: "Construct an interval from two endpoints.",
    },
    IntrinsicOperationSignature {
        name: "span_ordered",
        arguments: &[
            IntrinsicOperationArgumentKind::NumericExpression,
            IntrinsicOperationArgumentKind::NumericExpression,
        ],
        contexts: &[IntrinsicOperationContext::EventExpression],
        result: IntrinsicOperationResult::Float64List,
        docs: "Construct an interval with its endpoints ordered.",
    },
    IntrinsicOperationSignature {
        name: "polygon",
        arguments: &[IntrinsicOperationArgumentKind::PathExpression],
        contexts: &[IntrinsicOperationContext::SceneGeometry],
        result: IntrinsicOperationResult::SceneGeometry,
        docs: "Construct scene-query polygon geometry from a path.",
    },
    IntrinsicOperationSignature {
        name: "rect",
        arguments: &[
            IntrinsicOperationArgumentKind::NumericExpression,
            IntrinsicOperationArgumentKind::NumericExpression,
            IntrinsicOperationArgumentKind::NumericExpression,
            IntrinsicOperationArgumentKind::NumericExpression,
        ],
        contexts: &[IntrinsicOperationContext::SceneGeometry],
        result: IntrinsicOperationResult::SceneGeometry,
        docs: "Construct scene-query rectangle geometry.",
    },
    IntrinsicOperationSignature {
        name: "circle",
        arguments: &[
            IntrinsicOperationArgumentKind::NumericExpression,
            IntrinsicOperationArgumentKind::NumericExpression,
            IntrinsicOperationArgumentKind::NumericExpression,
        ],
        contexts: &[IntrinsicOperationContext::SceneGeometry],
        result: IntrinsicOperationResult::SceneGeometry,
        docs: "Construct scene-query circle geometry.",
    },
];

pub fn intrinsic_operation_signature(name: &str) -> Option<&'static IntrinsicOperationSignature> {
    INTRINSIC_OPERATION_SIGNATURES
        .iter()
        .find(|signature| signature.name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{INTRINSIC_OPERATION_SIGNATURES, intrinsic_operation_signature};

    #[test]
    fn intrinsic_operation_inventory_is_unique_and_resolvable() {
        let mut names = BTreeSet::new();
        for signature in INTRINSIC_OPERATION_SIGNATURES {
            assert!(
                names.insert(signature.name),
                "duplicate intrinsic operation"
            );
            assert!(!signature.arguments.is_empty());
            assert!(!signature.contexts.is_empty());
            for argument in signature.arguments {
                assert!(!argument.as_str().is_empty());
            }
            assert!(!signature.result.as_str().is_empty());
            assert_eq!(
                signature.result.arrow_type().is_some(),
                !signature
                    .contexts
                    .contains(&super::IntrinsicOperationContext::SceneGeometry)
            );
            assert_eq!(
                intrinsic_operation_signature(signature.name),
                Some(signature)
            );
        }
    }
}
