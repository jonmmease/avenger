//! Shared facet channel identifiers for subplot configuration.

use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::SerializableExpr;

/// Dimension configuration trait used by facet subplot marks.
pub trait FacetDimensionConfig: Clone + Send + Sync + 'static {
    /// Channel name used for this facet dimension.
    fn channel_name() -> &'static str;
}

/// Row faceting dimension configuration.
#[derive(Clone, Debug)]
pub struct RowDimensionConfig;

impl FacetDimensionConfig for RowDimensionConfig {
    fn channel_name() -> &'static str {
        "row"
    }
}

/// Column faceting dimension configuration.
#[derive(Clone, Debug)]
pub struct ColumnDimensionConfig;

impl FacetDimensionConfig for ColumnDimensionConfig {
    fn channel_name() -> &'static str {
        "column"
    }
}

/// Wrapped faceting dimension configuration.
#[derive(Clone, Debug)]
pub struct WrapDimensionConfig;

impl FacetDimensionConfig for WrapDimensionConfig {
    fn channel_name() -> &'static str {
        "wrap"
    }
}

/// Physical column selection mode for wrapped facets.
#[serde_as]
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub enum FacetWrapColumnMode {
    /// Use the default wrap heuristic, `ceil(sqrt(slot_count))`.
    #[default]
    Auto,
    /// Use an authored expression for an exact column count.
    Fixed(#[serde_as(as = "FromInto<SerializableExpr>")] LogicalExprNode),
    /// Choose columns from an approximate target leaf plot-area width.
    ResponsiveWidth(#[serde_as(as = "FromInto<SerializableExpr>")] LogicalExprNode),
}
