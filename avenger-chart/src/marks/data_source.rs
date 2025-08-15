//! Data source and faceting strategies for marks

/// Data source strategy for marks in faceted plots
#[derive(Debug, Clone)]
pub enum DataSource {
    /// Inherit data from plot level (default for new marks)
    Inherited,
    /// Use explicit mark-level data
    Explicit,
}

/// Strategy for handling mark data in faceted plots
#[derive(Debug, Clone)]
pub enum FacetStrategy {
    /// Filter mark data by facet values (default)
    Filter,
    /// Show mark data in all facets (for reference marks)
    Broadcast,
    /// Skip this mark if facet variable not present in data
    Skip,
}