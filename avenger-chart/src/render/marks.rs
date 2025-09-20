//! Mark rendering and channel processing
//!
//! This module handles:
//! - Rendering marks with data processing
//! - Channel value processing and scaling
//! - Positional channel validation
//! - Conditional value handling
//! - Sorting support for marks

use super::PlotRenderer;
use crate::channel::ChannelValue;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::marks::Mark;
use crate::render::RenderContext;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::array::RecordBatch;
use datafusion::arrow::datatypes::{Field, Schema};
use std::collections::HashMap;
use std::sync::Arc;

impl<C: CoordinateSystem> PlotRenderer<'_, C> {
    /// Render a mark with full data processing and sorting support
    pub(super) async fn render_mark(
        &self,
        mark: &dyn Mark<C>,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Get channel mappings from DataContext
        let channels = mark.data_context().channels();

        // Resolve channel references (e.g., ":x" -> actual x expression)
        let channels = crate::channel::resolution::resolve_all_channel_refs(channels)?;

        // Check if any channel expressions reference columns
        let references_columns = channels.values().any(|channel_value| match channel_value {
            ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                !expr.column_refs().is_empty()
            }
            ChannelValue::Conditional {
                conditions,
                otherwise,
                ..
            } => {
                conditions.iter().any(|(condition, value)| {
                    !condition.column_refs().is_empty() || !value.expr().column_refs().is_empty()
                }) || !otherwise.expr().column_refs().is_empty()
            }
        });

        // Determine data source based on:
        // 1. If mark has explicit data, use it
        // 2. If no expressions reference columns, use unit (single row)
        // 3. Otherwise inherit from plot if available
        let df_ref = if let Some(mark_df) = mark.data_context().dataframe() {
            // Mark has explicit data
            Some(mark_df)
        } else if !references_columns {
            // No column references - use unit data
            None
        } else if let Some(plot_data) = &self.plot.data {
            // Inherit from plot
            Some(plot_data)
        } else {
            // No data available but columns are referenced
            return Err(AvengerChartError::InternalError(
                "Mark expressions reference columns but no data is available".to_string(),
            ));
        };

        // Check if mark has a sorting channel and apply sorting if needed
        let df = if let Some(df_ref) = df_ref {
            if let Some(sort_channel_name) = mark.sorting_channel() {
                if let Some(sort_channel) = channels.get(sort_channel_name) {
                    // Apply sorting transformation
                    let sort_expr =
                        self.apply_channel_scale(sort_channel_name, sort_channel, scales)?;

                    // Sort the DataFrame by the sorting expression
                    let sorted_df = df_ref.clone().sort(vec![sort_expr.sort(true, false)])?;
                    Arc::new(sorted_df)
                } else {
                    Arc::new(df_ref.clone())
                }
            } else {
                Arc::new(df_ref.clone())
            }
        } else {
            // Unit data source - create minimal DataFrame with single row
            // This allows scalar expressions to be evaluated
            use datafusion::prelude::*;
            let ctx = SessionContext::new();
            let empty_df = ctx
                .sql("SELECT 1 as _dummy")
                .await
                .map_err(|e| AvengerChartError::DataFusionError(e))?;
            Arc::new(empty_df)
        };

        // Get supported channels from the mark
        let supported_channels = mark.supported_channels();

        // Separate channels into those that need array data vs scalar data
        let mut array_channels = Vec::new();
        let mut scalar_channels = Vec::new();
        let mut has_array_data = false;

        for channel_desc in &supported_channels {
            if let Some(channel_value) = channels.get(channel_desc.name) {
                // Apply scaling to get the final expression
                let scaled_expr =
                    self.apply_channel_scale(channel_desc.name, channel_value, scales)?;

                // Check if this channel references columns (needs array data)
                if channel_desc.allow_column_ref && scaled_expr.any_column_refs() {
                    array_channels.push((channel_desc.name, scaled_expr));
                    has_array_data = true;
                } else {
                    scalar_channels.push((channel_desc.name, scaled_expr));
                }
            }
        }

        // Build array data batch if needed
        let data_batch = if has_array_data {
            let mut select_exprs = vec![];
            for (name, expr) in &array_channels {
                select_exprs.push(expr.clone().alias(*name));
            }

            let batch = (*df).clone().select(select_exprs)?.collect().await?;

            if batch.is_empty() {
                None
            } else {
                Some(batch[0].clone())
            }
        } else {
            None
        };

        // Build scalar batch - always needed, even if empty
        let scalar_batch = {
            let mut select_exprs = vec![];
            for (name, expr) in &scalar_channels {
                select_exprs.push(expr.clone().alias(*name));
            }

            if select_exprs.is_empty() {
                // Create empty batch with single row - need at least one column
                use datafusion::arrow::array::Int32Array;
                let schema = Arc::new(Schema::new(vec![Field::new(
                    "_dummy",
                    datafusion::arrow::datatypes::DataType::Int32,
                    false,
                )]));
                let array = Arc::new(Int32Array::from(vec![0]));
                RecordBatch::try_new(schema, vec![array]).unwrap()
            } else {
                // Execute query to get scalar values
                let batches = (*df)
                    .clone()
                    .select(select_exprs)?
                    .limit(0, Some(1))? // Only need one row for scalars
                    .collect()
                    .await?;

                if batches.is_empty() {
                    RecordBatch::try_new(Arc::new(Schema::new(vec![] as Vec<Field>)), vec![])
                        .unwrap()
                } else {
                    batches[0].clone()
                }
            }
        };

        // Validate positional channel data types before rendering
        self.validate_positional_channel_types(&data_batch, &scalar_batch)?;

        // Create render context with theme and dimensions
        let theme = self.plot.get_theme();
        let context = RenderContext::new(theme, plot_width, plot_height);

        // Call the mark's render_from_data method with context and coordinate system
        mark.render_from_data(
            data_batch.as_ref(),
            &scalar_batch,
            &context,
            self.plot.coord_system(),
        )
    }

    /// Validate that positional channels have numeric data types
    fn validate_positional_channel_types(
        &self,
        data_batch: &Option<RecordBatch>,
        scalar_batch: &RecordBatch,
    ) -> Result<(), AvengerChartError> {
        // Check each positional channel
        for channel_name in self.plot.coord_system().required_channels() {
            // Check in data batch first
            if let Some(data) = data_batch {
                if let Some(column) = data.column_by_name(channel_name) {
                    let dtype = column.data_type();
                    if !Self::is_numeric_type(dtype) {
                        return self.create_positional_type_error(channel_name, dtype);
                    }
                }
            }

            // Check in scalar batch
            if let Some(column) = scalar_batch.column_by_name(channel_name) {
                let dtype = column.data_type();
                if !Self::is_numeric_type(dtype) {
                    return self.create_positional_type_error(channel_name, dtype);
                }
            }
        }

        Ok(())
    }

    /// Check if a data type is numeric
    fn is_numeric_type(dtype: &datafusion::arrow::datatypes::DataType) -> bool {
        use datafusion::arrow::datatypes::DataType;
        matches!(
            dtype,
            DataType::Int8
                | DataType::Int16
                | DataType::Int32
                | DataType::Int64
                | DataType::UInt8
                | DataType::UInt16
                | DataType::UInt32
                | DataType::UInt64
                | DataType::Float16
                | DataType::Float32
                | DataType::Float64
        )
    }

    /// Create error for non-numeric positional channel
    fn create_positional_type_error(
        &self,
        channel_name: &str,
        dtype: &datafusion::arrow::datatypes::DataType,
    ) -> Result<(), AvengerChartError> {
        use datafusion::arrow::datatypes::DataType;

        // Get coordinate system name
        let coord_system_name = std::any::type_name::<C>()
            .split("::")
            .last()
            .unwrap_or("Unknown")
            .to_string();

        // Provide helpful suggestion based on the data type
        let (literal_value, suggestion) = match dtype {
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => (
                "string literal".to_string(),
                "Use col(\"column_name\") to reference a data column instead of a string literal.\n  \
                         If you need a fixed position, use a numeric value like lit(100.0)".to_string(),
            ),
            _ => (
                format!("{:?} value", dtype),
                "Positional channels require numeric values. \
                         Use col(\"column_name\") to reference a numeric column.".to_string(),
            ),
        };

        Err(AvengerChartError::PositionalScaleLiteralError {
            scale_name: channel_name.to_string(),
            coord_system: coord_system_name,
            literal_value,
            suggestion,
        })
    }

    /// Apply scaling transformation to a channel expression
    fn apply_channel_scale(
        &self,
        channel_name: &str,
        channel_value: &ChannelValue,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Result<datafusion::logical_expr::Expr, AvengerChartError> {
        use crate::channel::value::strip_trailing_numbers;

        match channel_value {
            ChannelValue::Value { expr } => {
                // No scaling requested, return expression as-is
                Ok(expr.clone())
            }
            ChannelValue::Conditional {
                conditions,
                otherwise,
                ..
            } => {
                // Build a CASE WHEN expression from the conditions
                use crate::channel::{ConditionalValue, value::strip_trailing_numbers};
                use datafusion::arrow::datatypes::DataType;
                use datafusion::logical_expr::{lit, when};
                use datafusion::scalar::ScalarValue;

                // Helper to convert color string literals to the proper ScalarValue format
                let convert_color_literal =
                    |expr: &datafusion::logical_expr::Expr| -> datafusion::logical_expr::Expr {
                        // Check if this is a string literal that might be a color
                        if let datafusion::logical_expr::Expr::Literal(scalar_value, _) = expr {
                            if let ScalarValue::Utf8(Some(s)) = scalar_value {
                                // Use our existing color parsing utility
                                if let Some(color_or_gradient) = crate::utils::parse_color_string(s)
                                {
                                    use avenger_common::types::ColorOrGradient;
                                    if let ColorOrGradient::Color(rgba) = color_or_gradient {
                                        // Convert to List ScalarValue with Float32 values
                                        let values: Vec<ScalarValue> = rgba
                                            .into_iter()
                                            .map(|v| ScalarValue::Float32(Some(v)))
                                            .collect();

                                        // Create the list array and wrap in ScalarValue
                                        let list_array = ScalarValue::new_list_nullable(
                                            &values,
                                            &DataType::Float32,
                                        );
                                        let scalar_list = ScalarValue::List(list_array);
                                        return lit(scalar_list);
                                    }
                                }
                            }
                        }
                        // Not a color literal or failed to parse, return as-is
                        expr.clone()
                    };

                // For conditional values, the scale name is derived from the channel name
                let scale_key = strip_trailing_numbers(channel_name).to_string();

                // Get the scale if it exists (for applying to Field branches)
                let scale_opt = scales.get(&scale_key);

                // Process the otherwise value
                let otherwise_expr = match otherwise {
                    ConditionalValue::Scaled { expr } => {
                        // This needs scaling
                        if let Some(scale) = scale_opt {
                            use crate::scales::ConfiguredScaleDataFusionExt;
                            scale.to_expr(expr.clone())?
                        } else {
                            // No scale available, use expression as-is
                            expr.clone()
                        }
                    }
                    ConditionalValue::Value { expr } => {
                        // Literal value - convert if it's a color string
                        convert_color_literal(expr)
                    }
                };

                // Build CASE WHEN expression from conditions (evaluated in order added)
                // We iterate in reverse because we're building nested when() calls from inside out
                // The last condition in the array should be the innermost (evaluated last)
                let mut case_expr = otherwise_expr;
                for (test, value) in conditions.iter().rev() {
                    let value_expr = match value {
                        ConditionalValue::Scaled { expr } => {
                            // This needs scaling
                            if let Some(scale) = scale_opt {
                                use crate::scales::ConfiguredScaleDataFusionExt;
                                scale.to_expr(expr.clone())?
                            } else {
                                expr.clone()
                            }
                        }
                        ConditionalValue::Value { expr } => {
                            // Literal value - convert if it's a color string
                            convert_color_literal(expr)
                        }
                    };

                    // Wrap in WHEN clause
                    case_expr = when(test.clone(), value_expr)
                        .otherwise(case_expr)?
                        .alias(channel_name);
                }

                Ok(case_expr)
            }
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                ..
            } => {
                // Determine scale name (custom or derived from channel)
                let scale_key = scale_name
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| strip_trailing_numbers(channel_name).to_string());

                // Scale MUST exist if scaling was requested
                let scale = scales.get(&scale_key).ok_or_else(|| {
                    AvengerChartError::ScaleNotFound(format!(
                        "Scale '{}' requested for channel '{}' but not found",
                        scale_key, channel_name
                    ))
                })?;

                // Use ConfiguredScale's extension methods
                use crate::scales::ConfiguredScaleDataFusionExt;

                // Apply the scale transformation with optional band parameter
                if let Some(band_value) = band {
                    scale.to_expr_with_band(expr.clone(), *band_value)
                } else {
                    scale.to_expr(expr.clone())
                }
            }
        }
    }
}
