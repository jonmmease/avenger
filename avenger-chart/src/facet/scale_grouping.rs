//! Scale grouping for grid facets with partial sharing modes
//!
//! This module provides infrastructure to group ScaleBuilders based on sharing mode:
//! - Shared: One builder for all subplots
//! - Free: One builder per subplot (row_idx, col_idx)
//! - SharedInRow: One builder per row
//! - SharedInColumn: One builder per column

use crate::channel::config_traits::ScaleSharing;
use crate::error::AvengerChartError;
use crate::plot::CompiledPlot;
use crate::scales::{builder::ScaleBuilder, ConfiguredScaleWithSpec};
use datafusion::common::ScalarValue;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::{lit, Expr};
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;
use std::collections::HashMap;

/// Grouping key for scale builders
///
/// Uses Option pattern to represent sharing modes:
/// - `GroupKey { row: None, col: None }`: Shared across all subplots
/// - `GroupKey { row: Some(r), col: None }`: Shared within row r (SharedInRow)
/// - `GroupKey { row: None, col: Some(c) }`: Shared within column c (SharedInColumn)
/// - `GroupKey { row: Some(r), col: Some(c) }`: Independent for cell (r, c) (Free)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GroupKey {
    row: Option<usize>,
    col: Option<usize>,
}

impl GroupKey {
    /// Compute group key for a cell position given sharing mode
    fn for_cell(row_idx: usize, col_idx: usize, mode: ScaleSharing) -> Self {
        match mode {
            ScaleSharing::Shared => GroupKey { row: None, col: None },
            ScaleSharing::Free => GroupKey { row: Some(row_idx), col: Some(col_idx) },
            ScaleSharing::SharedInRow => GroupKey { row: Some(row_idx), col: None },
            ScaleSharing::SharedInColumn => GroupKey { row: None, col: Some(col_idx) },
        }
    }
}

/// Scale grouping for a single channel
struct ChannelGrouping {
    mode: ScaleSharing,
    builders: HashMap<GroupKey, ScaleBuilder>,
}

impl ChannelGrouping {
    /// Get the ScaleBuilder for a specific subplot position
    fn get_builder(&self, row_idx: usize, col_idx: usize) -> Option<&ScaleBuilder> {
        let key = GroupKey::for_cell(row_idx, col_idx, self.mode);
        self.builders.get(&key)
    }
}

/// Manages scale grouping for all channels in a grid facet
///
/// This struct builds and caches ScaleBuilders based on the sharing mode for each channel.
/// It enables efficient two-pass rendering by building the ScaleBuilders once and then
/// generating final scales with different dimensions in each pass.
///
/// The `fallback_builder` is used for empty cells where no data exists - it's built from
/// the full dataset and ensures all required scales exist even for empty subplots.
pub struct ScaleGrouping {
    channel_groupings: HashMap<String, ChannelGrouping>,
    fallback_builder: ScaleBuilder,
}

impl ScaleGrouping {
    /// Build scale grouping from channel sharing configuration
    ///
    /// # Arguments
    /// * `compiled_subplot` - The subplot plot definition
    /// * `scale_sharing_by_channel` - Map from channel name to sharing mode
    /// * `row_domain_vals` - Domain values for row faceting dimension
    /// * `col_domain_vals` - Domain values for column faceting dimension
    /// * `df` - Full dataset (before facet filtering)
    /// * `row_expr` - Expression for row faceting channel
    /// * `col_expr` - Expression for column faceting channel
    /// * `ctx` - DataFusion session context
    /// * `params` - Parameters for evaluation (does not include FacetContext)
    pub async fn build(
        compiled_subplot: &CompiledPlot,
        scale_sharing_by_channel: &HashMap<String, ScaleSharing>,
        row_domain_vals: &[ScalarValue],
        col_domain_vals: &[ScalarValue],
        df: &DataFrame,
        row_expr: &Expr,
        col_expr: &Expr,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<Self, AvengerChartError> {
        // Build fallback builder from full dataset (for empty cells)
        let fallback_builder = compiled_subplot
            .build_scale_builder_from_dataframe(ctx, params, df)
            .await?;

        let mut channel_groupings = HashMap::new();

        for (channel, &mode) in scale_sharing_by_channel {
            let mut builders = HashMap::new();

            // Collect unique group keys for this mode
            let mut group_keys = std::collections::HashSet::new();
            for row_idx in 0..row_domain_vals.len() {
                for col_idx in 0..col_domain_vals.len() {
                    let key = GroupKey::for_cell(row_idx, col_idx, mode);
                    group_keys.insert(key);
                }
            }

            // Build one ScaleBuilder per group
            for key in group_keys {
                let filter_df = match (key.row, key.col) {
                    (None, None) => {
                        // Shared: use full dataset
                        df.clone()
                    }
                    (Some(row_idx), None) => {
                        // SharedInRow: filter by row value only (include all columns)
                        let row_val = &row_domain_vals[row_idx];
                        df.clone().filter(row_expr.clone().eq(lit(row_val.clone())))?
                    }
                    (None, Some(col_idx)) => {
                        // SharedInColumn: filter by col value only (include all rows)
                        let col_val = &col_domain_vals[col_idx];
                        df.clone().filter(col_expr.clone().eq(lit(col_val.clone())))?
                    }
                    (Some(row_idx), Some(col_idx)) => {
                        // Free: filter by both row and column
                        let row_val = &row_domain_vals[row_idx];
                        let col_val = &col_domain_vals[col_idx];
                        df.clone()
                            .filter(row_expr.clone().eq(lit(row_val.clone())))?
                            .filter(col_expr.clone().eq(lit(col_val.clone())))?
                    }
                };

                // Build ScaleBuilder from filtered data
                // Even for empty groups, build a builder to maintain alignment
                let builder = compiled_subplot
                    .build_scale_builder_from_dataframe(ctx, params, &filter_df)
                    .await?;

                builders.insert(key, builder);
            }

            channel_groupings.insert(
                channel.clone(),
                ChannelGrouping { mode, builders },
            );
        }

        Ok(Self { channel_groupings, fallback_builder })
    }

    /// Build scales for a specific subplot position
    ///
    /// This method looks up the appropriate ScaleBuilder for each channel based on
    /// the subplot position and sharing mode, then builds the final scales with
    /// the given dimensions.
    ///
    /// For empty cells (where the ScaleBuilder returns no scales), this method falls
    /// back to using the shared/full dataset builder to ensure all required scales exist.
    ///
    /// # Arguments
    /// * `compiled_subplot` - The subplot plot definition
    /// * `row_idx` - Row index of the subplot
    /// * `col_idx` - Column index of the subplot
    /// * `width` - Width for scale building
    /// * `height` - Height for scale building
    /// * `ctx` - DataFusion session context
    /// * `params` - Parameters for evaluation (should include FacetContext)
    pub async fn build_scales_for_position(
        &self,
        compiled_subplot: &CompiledPlot,
        row_idx: usize,
        col_idx: usize,
        width: f32,
        height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        let mut scales = HashMap::new();

        // First pass: build scales from position-specific builders
        // Each channel grouping only contributes ITS OWN scale to avoid overwrites
        for (channel, grouping) in &self.channel_groupings {
            if let Some(builder) = grouping.get_builder(row_idx, col_idx) {
                let channel_scales = compiled_subplot
                    .build_scales_from_builder(builder, width, height, ctx, params)
                    .await?;

                // Only insert the scale for THIS channel to avoid overwriting other channels' scales
                // that may have been built from different data subsets
                if let Some(scale) = channel_scales.get(channel) {
                    scales.insert(channel.clone(), scale.clone());
                }
            }
        }

        // Second pass: backfill any missing scales from fallback builder
        // This ensures required positional channels (x, y, x2, y2) are always present
        if !scales.is_empty() {
            let fallback_scales = compiled_subplot
                .build_scales_from_builder(&self.fallback_builder, width, height, ctx, params)
                .await?;

            for (scale_name, scale) in fallback_scales {
                // Only insert if not already present (don't overwrite channel-specific scales)
                scales.entry(scale_name).or_insert(scale);
            }
        } else {
            // If we got no scales at all (empty cell), use all fallback scales
            let fallback_scales = compiled_subplot
                .build_scales_from_builder(&self.fallback_builder, width, height, ctx, params)
                .await?;

            for (scale_name, scale) in fallback_scales {
                scales.insert(scale_name, scale);
            }
        }

        Ok(scales)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_group_key_for_cell() {
        // Shared mode
        assert_eq!(
            GroupKey::for_cell(0, 0, ScaleSharing::Shared),
            GroupKey { row: None, col: None }
        );
        assert_eq!(
            GroupKey::for_cell(1, 2, ScaleSharing::Shared),
            GroupKey { row: None, col: None }
        );

        // Free mode
        assert_eq!(
            GroupKey::for_cell(0, 0, ScaleSharing::Free),
            GroupKey { row: Some(0), col: Some(0) }
        );
        assert_eq!(
            GroupKey::for_cell(1, 2, ScaleSharing::Free),
            GroupKey { row: Some(1), col: Some(2) }
        );

        // SharedInRow mode
        assert_eq!(
            GroupKey::for_cell(0, 0, ScaleSharing::SharedInRow),
            GroupKey { row: Some(0), col: None }
        );
        assert_eq!(
            GroupKey::for_cell(0, 2, ScaleSharing::SharedInRow),
            GroupKey { row: Some(0), col: None }
        );
        assert_eq!(
            GroupKey::for_cell(1, 0, ScaleSharing::SharedInRow),
            GroupKey { row: Some(1), col: None }
        );

        // SharedInColumn mode
        assert_eq!(
            GroupKey::for_cell(0, 0, ScaleSharing::SharedInColumn),
            GroupKey { row: None, col: Some(0) }
        );
        assert_eq!(
            GroupKey::for_cell(2, 0, ScaleSharing::SharedInColumn),
            GroupKey { row: None, col: Some(0) }
        );
        assert_eq!(
            GroupKey::for_cell(0, 1, ScaleSharing::SharedInColumn),
            GroupKey { row: None, col: Some(1) }
        );
    }

    #[test]
    fn test_scale_sharing_from_bool() {
        assert_eq!(ScaleSharing::from(true), ScaleSharing::Shared);
        assert_eq!(ScaleSharing::from(false), ScaleSharing::Free);
    }

    #[test]
    fn test_scale_sharing_serde() {
        // Test serialization
        let shared = ScaleSharing::Shared;
        let json = serde_json::to_string(&shared).unwrap();
        assert_eq!(json, "\"shared\"");

        let free = ScaleSharing::Free;
        let json = serde_json::to_string(&free).unwrap();
        assert_eq!(json, "\"free\"");

        let in_row = ScaleSharing::SharedInRow;
        let json = serde_json::to_string(&in_row).unwrap();
        assert_eq!(json, "\"shared_in_row\"");

        let in_col = ScaleSharing::SharedInColumn;
        let json = serde_json::to_string(&in_col).unwrap();
        assert_eq!(json, "\"shared_in_column\"");

        // Test deserialization
        let shared: ScaleSharing = serde_json::from_str("\"shared\"").unwrap();
        assert_eq!(shared, ScaleSharing::Shared);

        let free: ScaleSharing = serde_json::from_str("\"free\"").unwrap();
        assert_eq!(free, ScaleSharing::Free);

        let in_row: ScaleSharing = serde_json::from_str("\"shared_in_row\"").unwrap();
        assert_eq!(in_row, ScaleSharing::SharedInRow);

        let in_col: ScaleSharing = serde_json::from_str("\"shared_in_column\"").unwrap();
        assert_eq!(in_col, ScaleSharing::SharedInColumn);
    }
}
