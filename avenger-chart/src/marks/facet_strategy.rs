//! Faceting strategies for marks

use serde::{Deserialize, Serialize};

/// Strategy for handling mark data in faceted plots
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FacetStrategy {
    /// Filter mark data by facet values (default)
    Filter,
    /// Show mark data in all facets (for reference marks)
    Broadcast,
    /// Skip this mark if facet variable not present in data
    Skip,
}
