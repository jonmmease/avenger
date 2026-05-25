//! Shared row/column channel identifiers for facet subplot configuration.

/// Dimension configuration trait used by facet subplot marks.
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
