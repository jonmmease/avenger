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
use crate::scales::{ConfiguredScaleWithSpec, builder::ScaleBuilder};
use datafusion::common::ScalarValue;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::{Expr, lit};
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
            ScaleSharing::Shared => GroupKey {
                row: None,
                col: None,
            },
            ScaleSharing::Free => GroupKey {
                row: Some(row_idx),
                col: Some(col_idx),
            },
            ScaleSharing::SharedInRow => GroupKey {
                row: Some(row_idx),
                col: None,
            },
            ScaleSharing::SharedInColumn => GroupKey {
                row: None,
                col: Some(col_idx),
            },
            ScaleSharing::Level(n) => {
                // Hierarchical level-based sharing
                // Level(0) = Free: independent per cell
                // Level(u8::MAX) = Shared: global
                // Intermediate levels: treat as Free for grid-based grouping
                // (full level-based domain propagation handled elsewhere)
                if n == 0 {
                    GroupKey {
                        row: Some(row_idx),
                        col: Some(col_idx),
                    }
                } else if n == u8::MAX {
                    GroupKey {
                        row: None,
                        col: None,
                    }
                } else {
                    // Intermediate levels behave like Free in grid context
                    GroupKey {
                        row: Some(row_idx),
                        col: Some(col_idx),
                    }
                }
            }
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
                        df.clone()
                            .filter(row_expr.clone().eq(lit(row_val.clone())))?
                    }
                    (None, Some(col_idx)) => {
                        // SharedInColumn: filter by col value only (include all rows)
                        let col_val = &col_domain_vals[col_idx];
                        df.clone()
                            .filter(col_expr.clone().eq(lit(col_val.clone())))?
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

            channel_groupings.insert(channel.clone(), ChannelGrouping { mode, builders });
        }

        Ok(Self {
            channel_groupings,
            fallback_builder,
        })
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
            GroupKey {
                row: None,
                col: None
            }
        );
        assert_eq!(
            GroupKey::for_cell(1, 2, ScaleSharing::Shared),
            GroupKey {
                row: None,
                col: None
            }
        );

        // Free mode
        assert_eq!(
            GroupKey::for_cell(0, 0, ScaleSharing::Free),
            GroupKey {
                row: Some(0),
                col: Some(0)
            }
        );
        assert_eq!(
            GroupKey::for_cell(1, 2, ScaleSharing::Free),
            GroupKey {
                row: Some(1),
                col: Some(2)
            }
        );

        // SharedInRow mode
        assert_eq!(
            GroupKey::for_cell(0, 0, ScaleSharing::SharedInRow),
            GroupKey {
                row: Some(0),
                col: None
            }
        );
        assert_eq!(
            GroupKey::for_cell(0, 2, ScaleSharing::SharedInRow),
            GroupKey {
                row: Some(0),
                col: None
            }
        );
        assert_eq!(
            GroupKey::for_cell(1, 0, ScaleSharing::SharedInRow),
            GroupKey {
                row: Some(1),
                col: None
            }
        );

        // SharedInColumn mode
        assert_eq!(
            GroupKey::for_cell(0, 0, ScaleSharing::SharedInColumn),
            GroupKey {
                row: None,
                col: Some(0)
            }
        );
        assert_eq!(
            GroupKey::for_cell(2, 0, ScaleSharing::SharedInColumn),
            GroupKey {
                row: None,
                col: Some(0)
            }
        );
        assert_eq!(
            GroupKey::for_cell(0, 1, ScaleSharing::SharedInColumn),
            GroupKey {
                row: None,
                col: Some(1)
            }
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

    #[test]
    fn test_scale_sharing_to_level() {
        // Free => 0
        assert_eq!(ScaleSharing::Free.to_level(), 0);

        // Level(n) => n for various values
        assert_eq!(ScaleSharing::Level(0).to_level(), 0);
        assert_eq!(ScaleSharing::Level(1).to_level(), 1);
        assert_eq!(ScaleSharing::Level(2).to_level(), 2);
        assert_eq!(ScaleSharing::Level(10).to_level(), 10);
        assert_eq!(ScaleSharing::Level(u8::MAX).to_level(), u8::MAX);

        // Shared => u8::MAX
        assert_eq!(ScaleSharing::Shared.to_level(), u8::MAX);

        // Deprecated variants => u8::MAX (treated as globally shared)
        assert_eq!(ScaleSharing::SharedInRow.to_level(), u8::MAX);
        assert_eq!(ScaleSharing::SharedInColumn.to_level(), u8::MAX);
    }

    #[test]
    fn test_scale_sharing_from_level() {
        // 0 => Free
        assert_eq!(ScaleSharing::from_level(0), ScaleSharing::Free);

        // u8::MAX => Shared
        assert_eq!(ScaleSharing::from_level(u8::MAX), ScaleSharing::Shared);

        // 1..254 => Level(n)
        assert_eq!(ScaleSharing::from_level(1), ScaleSharing::Level(1));
        assert_eq!(ScaleSharing::from_level(2), ScaleSharing::Level(2));
        assert_eq!(ScaleSharing::from_level(127), ScaleSharing::Level(127));
        assert_eq!(ScaleSharing::from_level(254), ScaleSharing::Level(254));
    }

    #[test]
    fn test_scale_sharing_level_round_trip() {
        // Test that from_level(to_level(x)) preserves semantics
        // Note: Level(0) round-trips to Free, Level(u8::MAX) round-trips to Shared
        // This is by design - they are semantically equivalent

        // Free <-> 0
        assert_eq!(
            ScaleSharing::from_level(ScaleSharing::Free.to_level()),
            ScaleSharing::Free
        );

        // Shared <-> u8::MAX
        assert_eq!(
            ScaleSharing::from_level(ScaleSharing::Shared.to_level()),
            ScaleSharing::Shared
        );

        // Level(n) for intermediate values
        for n in [1u8, 2, 10, 100, 200, 254] {
            assert_eq!(
                ScaleSharing::from_level(ScaleSharing::Level(n).to_level()),
                ScaleSharing::Level(n)
            );
        }

        // Level(0) normalizes to Free
        assert_eq!(
            ScaleSharing::from_level(ScaleSharing::Level(0).to_level()),
            ScaleSharing::Free
        );

        // Level(u8::MAX) normalizes to Shared
        assert_eq!(
            ScaleSharing::from_level(ScaleSharing::Level(u8::MAX).to_level()),
            ScaleSharing::Shared
        );
    }

    #[test]
    fn test_scale_sharing_should_share_with_parent() {
        // Free and Level(0) should NOT share with parent
        assert!(!ScaleSharing::Free.should_share_with_parent());
        assert!(!ScaleSharing::Level(0).should_share_with_parent());

        // Level(1+) should share with parent
        assert!(ScaleSharing::Level(1).should_share_with_parent());
        assert!(ScaleSharing::Level(2).should_share_with_parent());
        assert!(ScaleSharing::Level(10).should_share_with_parent());

        // Shared and Level(u8::MAX) should share
        assert!(ScaleSharing::Shared.should_share_with_parent());
        assert!(ScaleSharing::Level(u8::MAX).should_share_with_parent());

        // Deprecated variants should share
        assert!(ScaleSharing::SharedInRow.should_share_with_parent());
        assert!(ScaleSharing::SharedInColumn.should_share_with_parent());
    }

    #[test]
    fn test_scale_sharing_is_fully_shared() {
        // Only Shared, Level(u8::MAX), and deprecated variants are fully shared
        assert!(ScaleSharing::Shared.is_fully_shared());
        assert!(ScaleSharing::Level(u8::MAX).is_fully_shared());
        assert!(ScaleSharing::SharedInRow.is_fully_shared());
        assert!(ScaleSharing::SharedInColumn.is_fully_shared());

        // Free and Level(0..254) are NOT fully shared
        assert!(!ScaleSharing::Free.is_fully_shared());
        assert!(!ScaleSharing::Level(0).is_fully_shared());
        assert!(!ScaleSharing::Level(1).is_fully_shared());
        assert!(!ScaleSharing::Level(100).is_fully_shared());
        assert!(!ScaleSharing::Level(254).is_fully_shared());
    }

    #[test]
    fn test_scale_sharing_is_free() {
        // Only Free and Level(0) are free
        assert!(ScaleSharing::Free.is_free());
        assert!(ScaleSharing::Level(0).is_free());

        // Everything else is not free
        assert!(!ScaleSharing::Level(1).is_free());
        assert!(!ScaleSharing::Level(100).is_free());
        assert!(!ScaleSharing::Level(u8::MAX).is_free());
        assert!(!ScaleSharing::Shared.is_free());
        assert!(!ScaleSharing::SharedInRow.is_free());
        assert!(!ScaleSharing::SharedInColumn.is_free());
    }

    #[test]
    fn test_scale_sharing_level_serde() {
        // Test Level variant serialization
        let level1 = ScaleSharing::Level(1);
        let json = serde_json::to_string(&level1).unwrap();
        assert_eq!(json, "{\"level\":1}");

        let level42 = ScaleSharing::Level(42);
        let json = serde_json::to_string(&level42).unwrap();
        assert_eq!(json, "{\"level\":42}");

        let level_max = ScaleSharing::Level(u8::MAX);
        let json = serde_json::to_string(&level_max).unwrap();
        assert_eq!(json, "{\"level\":255}");

        // Test Level variant deserialization
        let level1: ScaleSharing = serde_json::from_str("{\"level\":1}").unwrap();
        assert_eq!(level1, ScaleSharing::Level(1));

        let level42: ScaleSharing = serde_json::from_str("{\"level\":42}").unwrap();
        assert_eq!(level42, ScaleSharing::Level(42));

        let level_max: ScaleSharing = serde_json::from_str("{\"level\":255}").unwrap();
        assert_eq!(level_max, ScaleSharing::Level(255));

        // Level(0) serializes as level:0, distinct from "free"
        let level0 = ScaleSharing::Level(0);
        let json = serde_json::to_string(&level0).unwrap();
        assert_eq!(json, "{\"level\":0}");

        let level0: ScaleSharing = serde_json::from_str("{\"level\":0}").unwrap();
        assert_eq!(level0, ScaleSharing::Level(0));
    }

    #[test]
    fn test_scale_sharing_group_key_with_level() {
        // Level(0) should behave like Free
        assert_eq!(
            GroupKey::for_cell(0, 0, ScaleSharing::Level(0)),
            GroupKey {
                row: Some(0),
                col: Some(0)
            }
        );
        assert_eq!(
            GroupKey::for_cell(1, 2, ScaleSharing::Level(0)),
            GroupKey {
                row: Some(1),
                col: Some(2)
            }
        );

        // Level(u8::MAX) should behave like Shared
        assert_eq!(
            GroupKey::for_cell(0, 0, ScaleSharing::Level(u8::MAX)),
            GroupKey {
                row: None,
                col: None
            }
        );
        assert_eq!(
            GroupKey::for_cell(1, 2, ScaleSharing::Level(u8::MAX)),
            GroupKey {
                row: None,
                col: None
            }
        );

        // Intermediate levels behave like Free in grid context (per implementation)
        assert_eq!(
            GroupKey::for_cell(0, 0, ScaleSharing::Level(1)),
            GroupKey {
                row: Some(0),
                col: Some(0)
            }
        );
        assert_eq!(
            GroupKey::for_cell(1, 2, ScaleSharing::Level(5)),
            GroupKey {
                row: Some(1),
                col: Some(2)
            }
        );
    }
}
