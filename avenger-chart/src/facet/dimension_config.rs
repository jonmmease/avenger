//! Minimal dimension identifiers for faceting.
//!
//! Earlier versions carried a broad trait surface for row/column behavior, but
//! the current facet implementation only relies on stable channel identifiers.

/// Dimension configuration trait used by facet marks.
pub trait FacetDimensionConfig: Clone + Send + Sync + 'static {
    /// Channel name used for this facet dimension ("row" or "column").
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
