//! Unified domain extent type for scale sharing across facets.
//!
//! This module provides `DomainExtent`, a unified type that represents
//! domain extents with optional radius padding information. This replaces
//! the split between `DataExtents` (internal cache) and `SerializableDataExtents`
//! (cross-boundary transport) with a single type that preserves radius
//! information throughout the pipeline.
//!
//! # Background
//!
//! The Level(N) vs Shared domain bug was caused by different types:
//! - `DataExtents`: No radius variant (3 variants)
//! - `SerializableDataExtents`: Has `RadiusAwareInterval` variant (4 variants)
//!
//! When Level(N) domains were computed from raw DataFrames, radius info
//! was lost. This unified type ensures radius info is always preserved.

use crate::facet::coordination::{SerializableDataExtents, SerializableDomainValue};
use serde::{Deserialize, Serialize};

/// Unified domain extent type with optional radius information.
///
/// This is the canonical type for representing domain extents throughout
/// the system. It replaces both `DataExtents` and `SerializableDataExtents`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DomainExtent {
    /// The domain bounds (numeric, discrete, or temporal)
    pub bounds: DomainBounds,
    /// Optional radius padding for symbol marks.
    /// Present when the domain was extracted from a radius-aware scale builder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<RadiusPadding>,
}

/// The bounds of a domain extent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DomainBounds {
    /// Numeric interval with min and max values
    Numeric { min: f64, max: f64 },
    /// Categorical domain with discrete values
    Discrete(Vec<SerializableDomainValue>),
    /// Temporal interval with min and max timestamps (milliseconds since epoch)
    Temporal { min: i64, max: i64 },
}

/// Radius padding information for symbol marks.
///
/// When a domain is shared across facets containing symbol marks,
/// the domain needs to be padded to account for the symbol radius.
/// This struct stores the maximum radius values observed in the data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RadiusPadding {
    /// Maximum radius extending below (towards min) the data points
    pub max_lower: f64,
    /// Maximum radius extending above (towards max) the data points
    pub max_upper: f64,
}

impl DomainExtent {
    /// Create a numeric extent without radius padding.
    pub fn numeric(min: f64, max: f64) -> Self {
        Self {
            bounds: DomainBounds::Numeric { min, max },
            radius: None,
        }
    }

    /// Create a numeric extent with radius padding.
    pub fn numeric_with_radius(
        min: f64,
        max: f64,
        max_radius_lower: f64,
        max_radius_upper: f64,
    ) -> Self {
        Self {
            bounds: DomainBounds::Numeric { min, max },
            radius: Some(RadiusPadding {
                max_lower: max_radius_lower,
                max_upper: max_radius_upper,
            }),
        }
    }

    /// Create a discrete (categorical) extent.
    pub fn discrete(values: Vec<SerializableDomainValue>) -> Self {
        Self {
            bounds: DomainBounds::Discrete(values),
            radius: None,
        }
    }

    /// Create a temporal extent.
    pub fn temporal(min: i64, max: i64) -> Self {
        Self {
            bounds: DomainBounds::Temporal { min, max },
            radius: None,
        }
    }

    /// Check if this extent has radius padding information.
    pub fn has_radius(&self) -> bool {
        self.radius.is_some()
    }

    /// Get the effective numeric bounds considering radius padding.
    ///
    /// Returns `None` if this is not a numeric extent.
    /// If radius padding is present, the bounds are expanded to include it.
    pub fn effective_numeric_bounds(&self) -> Option<(f64, f64)> {
        match &self.bounds {
            DomainBounds::Numeric { min, max } => {
                if let Some(radius) = &self.radius {
                    Some((min - radius.max_lower, max + radius.max_upper))
                } else {
                    Some((*min, *max))
                }
            }
            _ => None,
        }
    }

    /// Get the numeric bounds without radius padding.
    ///
    /// Returns `None` if this is not a numeric extent.
    pub fn numeric_bounds(&self) -> Option<(f64, f64)> {
        match &self.bounds {
            DomainBounds::Numeric { min, max } => Some((*min, *max)),
            _ => None,
        }
    }

    /// Get the temporal bounds.
    ///
    /// Returns `None` if this is not a temporal extent.
    pub fn temporal_bounds(&self) -> Option<(i64, i64)> {
        match &self.bounds {
            DomainBounds::Temporal { min, max } => Some((*min, *max)),
            _ => None,
        }
    }

    /// Get the discrete values.
    ///
    /// Returns `None` if this is not a discrete extent.
    pub fn discrete_values(&self) -> Option<&[SerializableDomainValue]> {
        match &self.bounds {
            DomainBounds::Discrete(values) => Some(values),
            _ => None,
        }
    }
}

// ============================================================================
// Conversions between DomainExtent and SerializableDataExtents
// ============================================================================

impl From<SerializableDataExtents> for DomainExtent {
    fn from(extents: SerializableDataExtents) -> Self {
        match extents {
            SerializableDataExtents::Interval { min, max } => DomainExtent::numeric(min, max),
            SerializableDataExtents::RadiusAwareInterval {
                min,
                max,
                max_radius_lower,
                max_radius_upper,
            } => DomainExtent::numeric_with_radius(min, max, max_radius_lower, max_radius_upper),
            SerializableDataExtents::Discrete(values) => DomainExtent::discrete(values),
            SerializableDataExtents::Temporal { min, max } => DomainExtent::temporal(min, max),
        }
    }
}

impl From<DomainExtent> for SerializableDataExtents {
    fn from(extent: DomainExtent) -> Self {
        match extent.bounds {
            DomainBounds::Numeric { min, max } => {
                if let Some(radius) = extent.radius {
                    SerializableDataExtents::RadiusAwareInterval {
                        min,
                        max,
                        max_radius_lower: radius.max_lower,
                        max_radius_upper: radius.max_upper,
                    }
                } else {
                    SerializableDataExtents::Interval { min, max }
                }
            }
            DomainBounds::Discrete(values) => SerializableDataExtents::Discrete(values),
            DomainBounds::Temporal { min, max } => SerializableDataExtents::Temporal { min, max },
        }
    }
}

impl From<&SerializableDataExtents> for DomainExtent {
    fn from(extents: &SerializableDataExtents) -> Self {
        match extents {
            SerializableDataExtents::Interval { min, max } => DomainExtent::numeric(*min, *max),
            SerializableDataExtents::RadiusAwareInterval {
                min,
                max,
                max_radius_lower,
                max_radius_upper,
            } => DomainExtent::numeric_with_radius(*min, *max, *max_radius_lower, *max_radius_upper),
            SerializableDataExtents::Discrete(values) => DomainExtent::discrete(values.clone()),
            SerializableDataExtents::Temporal { min, max } => DomainExtent::temporal(*min, *max),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_numeric_extent_creation() {
        let extent = DomainExtent::numeric(0.0, 100.0);
        assert_eq!(extent.numeric_bounds(), Some((0.0, 100.0)));
        assert!(!extent.has_radius());
        assert_eq!(extent.effective_numeric_bounds(), Some((0.0, 100.0)));
    }

    #[test]
    fn test_numeric_with_radius_creation() {
        let extent = DomainExtent::numeric_with_radius(0.0, 100.0, 5.0, 10.0);
        assert_eq!(extent.numeric_bounds(), Some((0.0, 100.0)));
        assert!(extent.has_radius());
        // Effective bounds should include radius padding
        assert_eq!(extent.effective_numeric_bounds(), Some((-5.0, 110.0)));
    }

    #[test]
    fn test_discrete_extent_creation() {
        let values = vec![
            SerializableDomainValue::String("a".to_string()),
            SerializableDomainValue::String("b".to_string()),
        ];
        let extent = DomainExtent::discrete(values.clone());
        assert_eq!(extent.discrete_values(), Some(values.as_slice()));
        assert!(!extent.has_radius());
        assert_eq!(extent.numeric_bounds(), None);
    }

    #[test]
    fn test_temporal_extent_creation() {
        let extent = DomainExtent::temporal(1000, 2000);
        assert_eq!(extent.temporal_bounds(), Some((1000, 2000)));
        assert!(!extent.has_radius());
    }

    #[test]
    fn test_from_serializable_interval() {
        let serializable = SerializableDataExtents::Interval { min: 0.0, max: 100.0 };
        let extent: DomainExtent = serializable.into();
        assert_eq!(extent.numeric_bounds(), Some((0.0, 100.0)));
        assert!(!extent.has_radius());
    }

    #[test]
    fn test_from_serializable_radius_aware() {
        let serializable = SerializableDataExtents::RadiusAwareInterval {
            min: 0.0,
            max: 100.0,
            max_radius_lower: 5.0,
            max_radius_upper: 10.0,
        };
        let extent: DomainExtent = serializable.into();
        assert_eq!(extent.numeric_bounds(), Some((0.0, 100.0)));
        assert!(extent.has_radius());
        assert_eq!(
            extent.radius,
            Some(RadiusPadding {
                max_lower: 5.0,
                max_upper: 10.0
            })
        );
    }

    #[test]
    fn test_into_serializable_interval() {
        let extent = DomainExtent::numeric(0.0, 100.0);
        let serializable: SerializableDataExtents = extent.into();
        assert!(matches!(serializable, SerializableDataExtents::Interval { min: 0.0, max: 100.0 }));
    }

    #[test]
    fn test_into_serializable_radius_aware() {
        let extent = DomainExtent::numeric_with_radius(0.0, 100.0, 5.0, 10.0);
        let serializable: SerializableDataExtents = extent.into();
        assert!(matches!(
            serializable,
            SerializableDataExtents::RadiusAwareInterval {
                min: 0.0,
                max: 100.0,
                max_radius_lower: 5.0,
                max_radius_upper: 10.0
            }
        ));
    }

    #[test]
    fn test_roundtrip_conversion() {
        // Test all variants round-trip correctly
        let test_cases = vec![
            SerializableDataExtents::Interval { min: -10.0, max: 50.0 },
            SerializableDataExtents::RadiusAwareInterval {
                min: 0.0,
                max: 100.0,
                max_radius_lower: 5.0,
                max_radius_upper: 10.0,
            },
            SerializableDataExtents::Discrete(vec![
                SerializableDomainValue::String("a".to_string()),
                SerializableDomainValue::Int(42),
            ]),
            SerializableDataExtents::Temporal { min: 1000, max: 2000 },
        ];

        for original in test_cases {
            let extent: DomainExtent = original.clone().into();
            let round_tripped: SerializableDataExtents = extent.into();
            assert_eq!(original, round_tripped, "Round-trip failed for {:?}", original);
        }
    }

    #[test]
    fn test_serde_roundtrip() {
        let extent = DomainExtent::numeric_with_radius(0.0, 100.0, 5.0, 10.0);
        let json = serde_json::to_string(&extent).unwrap();
        let deserialized: DomainExtent = serde_json::from_str(&json).unwrap();
        assert_eq!(extent, deserialized);
    }

    #[test]
    fn test_serde_no_radius_omitted() {
        let extent = DomainExtent::numeric(0.0, 100.0);
        let json = serde_json::to_string(&extent).unwrap();
        // radius should be omitted when None
        assert!(!json.contains("radius"));
    }
}
