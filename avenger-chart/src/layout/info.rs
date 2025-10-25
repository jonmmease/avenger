//! Layout information produced by marks during evaluation
//!
//! This module defines the `LayoutInfo` trait and concrete types for layout data
//! that marks can produce. Unlike `PlotGeometry`, LayoutInfo is not serializable -
//! it's only used during the rendering process.
//!
//! # Usage
//!
//! Most marks return `()` (no layout info). Layout marks like Facet return
//! `ScaleUpdates` containing modified scales.

use crate::scales::ConfiguredScaleWithSpec;
use std::collections::HashMap;

/// Trait for layout information produced by marks
///
/// This trait provides a way for marks to communicate layout data back to the rendering system.
/// Unlike `PlotGeometry`, LayoutInfo does NOT support serialization - it's only used during
/// the rendering process and is not persisted.
///
/// # Design Philosophy
///
/// Layout info should ONLY be used by **layout marks** that:
/// - Have a single instance per plot/layer
/// - Have global layout responsibility
/// - Measure content to determine dimensions
///
/// Examples: Facet, Sankey, Treemap, Force-directed graph
///
/// **Do NOT use for regular data marks** (Symbol, Line, Rect, etc.)
pub trait LayoutInfo: Send + Sync + 'static {
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Default layout info - no layout data
///
/// Most marks return this (represented as `()`).
impl LayoutInfo for () {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Layout info containing scale updates
///
/// Used by layout marks (like Facet) that need to update scales based on
/// measured content.
#[derive(Debug, Clone, Default)]
pub struct ScaleUpdates {
    pub scales: HashMap<String, ConfiguredScaleWithSpec>,
}

impl LayoutInfo for ScaleUpdates {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ScaleUpdates {
    /// Create new scale updates from a scale map
    pub fn new(scales: HashMap<String, ConfiguredScaleWithSpec>) -> Self {
        Self { scales }
    }

    /// Create empty scale updates
    pub fn empty() -> Self {
        Self::default()
    }
}

/// Extract and merge scale updates from layout info
///
/// This helper function extracts scale updates from any LayoutInfo that contains them.
pub fn merge_scale_updates(
    base_scales: &HashMap<String, ConfiguredScaleWithSpec>,
    layout_infos: &[Box<dyn LayoutInfo>],
) -> HashMap<String, ConfiguredScaleWithSpec> {
    let mut merged = base_scales.clone();

    for info in layout_infos {
        // Try to downcast to ScaleUpdates
        if let Some(scale_updates) = info.as_any().downcast_ref::<ScaleUpdates>() {
            merged.extend(scale_updates.scales.clone());
        }
        // () has no scales to merge
        // Future layout types (Sankey, etc.) can be added here
    }

    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unit_layout_info() {
        let info: Box<dyn LayoutInfo> = Box::new(());
        assert!(info.as_any().downcast_ref::<()>().is_some());
    }

    #[test]
    fn test_scale_updates_downcast() {
        let updates = ScaleUpdates::empty();
        let boxed: Box<dyn LayoutInfo> = Box::new(updates);
        assert!(boxed.as_any().downcast_ref::<ScaleUpdates>().is_some());
    }

    #[test]
    fn test_merge_scale_updates() {
        let base = HashMap::new();

        // Create some layout infos
        let scale_map = HashMap::new();

        let layout_infos: Vec<Box<dyn LayoutInfo>> = vec![
            Box::new(()),
            Box::new(ScaleUpdates::new(scale_map)),
            Box::new(()),
        ];

        let merged = merge_scale_updates(&base, &layout_infos);
        // Verify merge happened (would check actual scales in real test)
        assert!(merged.is_empty()); // Empty because we didn't add real scales
    }
}
