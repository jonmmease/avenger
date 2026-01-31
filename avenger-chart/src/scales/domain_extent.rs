//! Unified domain extent type for scale sharing across facets.
//!
//! This module provides `DomainExtent`, a unified type that represents
//! domain extents with optional radius padding information.
//!
//! It also provides `SerializableDomainValue` and `SerializableDataExtents`
//! for serializing domain values across boundaries.

use datafusion::common::ScalarValue;
use serde::{Deserialize, Serialize};

// ============================================================================
// SerializableDomainValue
// ============================================================================

/// Serializable representation of domain values
///
/// ScalarValue doesn't implement Serialize/Deserialize, so we convert
/// domain values to this enum for serialization.
///
/// This enum preserves type information for grouping and domain operations,
/// with variants for all common Arrow/DataFusion scalar types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SerializableDomainValue {
    String(String),
    Int(i64),
    /// Unsigned 64-bit integer - stored separately to avoid i64 overflow
    UInt64(u64),
    Float(f64),
    Bool(bool),
    /// Decimal128 stored as string to preserve precision
    /// Format: "value:precision:scale" (e.g., "12345:10:2" for 123.45)
    Decimal128(String),
    /// Timestamp in milliseconds since Unix epoch
    TimestampMs(i64),
    /// Timestamp in microseconds since Unix epoch
    TimestampUs(i64),
    /// Timestamp in nanoseconds since Unix epoch
    TimestampNs(i64),
    Null,
}

impl SerializableDomainValue {
    /// Convert from ScalarValue
    ///
    /// Handles all common Arrow scalar types, preserving type information
    /// for proper round-trip serialization.
    pub fn from_scalar(value: &ScalarValue) -> Self {
        match value {
            // String types
            ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => {
                SerializableDomainValue::String(s.clone())
            }
            // Handle Utf8View - DataFusion uses this for string views in newer versions
            ScalarValue::Utf8View(Some(s)) => SerializableDomainValue::String(s.clone()),

            // Signed integer types
            ScalarValue::Int8(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::Int16(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::Int32(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::Int64(Some(n)) => SerializableDomainValue::Int(*n),

            // Unsigned integer types - small ones fit in i64
            ScalarValue::UInt8(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::UInt16(Some(n)) => SerializableDomainValue::Int(*n as i64),
            ScalarValue::UInt32(Some(n)) => SerializableDomainValue::Int(*n as i64),
            // UInt64 uses dedicated variant to avoid overflow
            ScalarValue::UInt64(Some(n)) => SerializableDomainValue::UInt64(*n),

            // Float types
            ScalarValue::Float32(Some(n)) => SerializableDomainValue::Float(*n as f64),
            ScalarValue::Float64(Some(n)) => SerializableDomainValue::Float(*n),

            // Boolean
            ScalarValue::Boolean(Some(b)) => SerializableDomainValue::Bool(*b),

            // Decimal128 - serialize as string to preserve precision
            ScalarValue::Decimal128(Some(value), precision, scale) => {
                SerializableDomainValue::Decimal128(format!("{}:{}:{}", value, precision, scale))
            }

            // Timestamp types - preserve the time unit in variant
            ScalarValue::TimestampMillisecond(Some(ts), _) => {
                SerializableDomainValue::TimestampMs(*ts)
            }
            ScalarValue::TimestampMicrosecond(Some(ts), _) => {
                SerializableDomainValue::TimestampUs(*ts)
            }
            ScalarValue::TimestampNanosecond(Some(ts), _) => {
                SerializableDomainValue::TimestampNs(*ts)
            }
            ScalarValue::TimestampSecond(Some(ts), _) => {
                // Convert seconds to milliseconds for consistent storage
                SerializableDomainValue::TimestampMs(*ts * 1000)
            }

            // Date types - convert to milliseconds
            ScalarValue::Date32(Some(days)) => {
                // days since Unix epoch -> milliseconds
                SerializableDomainValue::TimestampMs(*days as i64 * 86_400_000)
            }
            ScalarValue::Date64(Some(ms)) => SerializableDomainValue::TimestampMs(*ms),

            // Dictionary types - unwrap to the underlying value
            // This handles Arrow Dictionary-encoded columns which are common for categorical data
            ScalarValue::Dictionary(_, inner) => {
                // Recursively convert the inner value
                Self::from_scalar(inner.as_ref())
            }

            // Everything else becomes Null
            _ => SerializableDomainValue::Null,
        }
    }

    /// Convert to ScalarValue for use in domain operations
    ///
    /// This enables round-trip: ScalarValue -> SerializableDomainValue -> ScalarValue
    pub fn to_scalar(&self) -> ScalarValue {
        match self {
            SerializableDomainValue::String(s) => ScalarValue::Utf8(Some(s.clone())),
            SerializableDomainValue::Int(n) => ScalarValue::Int64(Some(*n)),
            SerializableDomainValue::UInt64(n) => ScalarValue::UInt64(Some(*n)),
            SerializableDomainValue::Float(f) => ScalarValue::Float64(Some(*f)),
            SerializableDomainValue::Bool(b) => ScalarValue::Boolean(Some(*b)),
            SerializableDomainValue::Decimal128(s) => {
                // Parse "value:precision:scale" format
                let parts: Vec<&str> = s.split(':').collect();
                if parts.len() == 3 {
                    if let (Ok(value), Ok(precision), Ok(scale)) = (
                        parts[0].parse::<i128>(),
                        parts[1].parse::<u8>(),
                        parts[2].parse::<i8>(),
                    ) {
                        return ScalarValue::Decimal128(Some(value), precision, scale);
                    }
                }
                // Fallback to null if parsing fails
                ScalarValue::Null
            }
            SerializableDomainValue::TimestampMs(ts) => {
                ScalarValue::TimestampMillisecond(Some(*ts), None)
            }
            SerializableDomainValue::TimestampUs(ts) => {
                ScalarValue::TimestampMicrosecond(Some(*ts), None)
            }
            SerializableDomainValue::TimestampNs(ts) => {
                ScalarValue::TimestampNanosecond(Some(*ts), None)
            }
            SerializableDomainValue::Null => ScalarValue::Null,
        }
    }
}

// ============================================================================
// SerializableDataExtents
// ============================================================================

/// Serializable version of scale data extents
///
/// This enables passing pre-computed data extents (min/max or discrete values)
/// between facets, allowing shared scale domains to be computed from the full dataset
/// rather than per-column filtered data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SerializableDataExtents {
    /// Numeric interval: (min, max)
    Interval { min: f64, max: f64 },
    /// Numeric interval with radius-aware padding info for symbols.
    /// Stores the max radius values so domain expansion can be computed correctly
    /// when sharing domains across cells with radius-aware scales.
    RadiusAwareInterval {
        min: f64,
        max: f64,
        max_radius_lower: f64,
        max_radius_upper: f64,
    },
    /// Categorical: unique values
    Discrete(Vec<SerializableDomainValue>),
    /// Temporal interval: (min, max) as Unix timestamps
    Temporal { min: i64, max: i64 },
}

impl SerializableDataExtents {
    /// Create from a numeric interval
    pub fn interval(min: f64, max: f64) -> Self {
        Self::Interval { min, max }
    }

    /// Create from a numeric interval with radius-aware padding info
    pub fn radius_aware_interval(
        min: f64,
        max: f64,
        max_radius_lower: f64,
        max_radius_upper: f64,
    ) -> Self {
        Self::RadiusAwareInterval {
            min,
            max,
            max_radius_lower,
            max_radius_upper,
        }
    }

    /// Create from temporal interval (timestamps)
    pub fn temporal(min: i64, max: i64) -> Self {
        Self::Temporal { min, max }
    }

    /// Create from discrete values
    pub fn discrete(values: Vec<ScalarValue>) -> Self {
        Self::Discrete(
            values
                .iter()
                .map(SerializableDomainValue::from_scalar)
                .collect(),
        )
    }
}

// ============================================================================
// DomainExtent
// ============================================================================

/// Unified domain extent type with optional radius information.
///
/// This is the canonical type for representing domain extents throughout
/// the system.
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
            } => {
                DomainExtent::numeric_with_radius(*min, *max, *max_radius_lower, *max_radius_upper)
            }
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
        let serializable = SerializableDataExtents::Interval {
            min: 0.0,
            max: 100.0,
        };
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
        assert!(matches!(
            serializable,
            SerializableDataExtents::Interval {
                min: 0.0,
                max: 100.0
            }
        ));
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
            SerializableDataExtents::Interval {
                min: -10.0,
                max: 50.0,
            },
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
            SerializableDataExtents::Temporal {
                min: 1000,
                max: 2000,
            },
        ];

        for original in test_cases {
            let extent: DomainExtent = original.clone().into();
            let round_tripped: SerializableDataExtents = extent.into();
            assert_eq!(
                original, round_tripped,
                "Round-trip failed for {:?}",
                original
            );
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

    #[test]
    fn test_serializable_domain_values() {
        // Test string
        let s = SerializableDomainValue::from_scalar(&ScalarValue::Utf8(Some("test".to_string())));
        assert!(matches!(s, SerializableDomainValue::String(_)));
        assert_eq!(s.to_scalar(), ScalarValue::Utf8(Some("test".to_string())));

        // Test int
        let i = SerializableDomainValue::from_scalar(&ScalarValue::Int64(Some(42)));
        assert!(matches!(i, SerializableDomainValue::Int(42)));
        assert_eq!(i.to_scalar(), ScalarValue::Int64(Some(42)));

        // Test float
        let f = SerializableDomainValue::from_scalar(&ScalarValue::Float64(Some(3.14)));
        assert!(matches!(f, SerializableDomainValue::Float(_)));

        // Test UInt64 (previously overflowed when stored as i64)
        let large_uint = u64::MAX;
        let u = SerializableDomainValue::from_scalar(&ScalarValue::UInt64(Some(large_uint)));
        assert!(matches!(u, SerializableDomainValue::UInt64(v) if v == large_uint));
        assert_eq!(u.to_scalar(), ScalarValue::UInt64(Some(large_uint)));

        // Test Decimal128
        let d = SerializableDomainValue::from_scalar(&ScalarValue::Decimal128(Some(12345), 10, 2));
        assert!(matches!(d, SerializableDomainValue::Decimal128(_)));
        assert_eq!(d.to_scalar(), ScalarValue::Decimal128(Some(12345), 10, 2));

        // Test timestamps
        let ts_ms = SerializableDomainValue::from_scalar(&ScalarValue::TimestampMillisecond(
            Some(1609459200000),
            None,
        ));
        assert!(matches!(
            ts_ms,
            SerializableDomainValue::TimestampMs(1609459200000)
        ));
        assert_eq!(
            ts_ms.to_scalar(),
            ScalarValue::TimestampMillisecond(Some(1609459200000), None)
        );

        // Test null
        let n = SerializableDomainValue::from_scalar(&ScalarValue::Null);
        assert!(matches!(n, SerializableDomainValue::Null));
    }
}
