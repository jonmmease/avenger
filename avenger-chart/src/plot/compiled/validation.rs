//! Validation methods for CompiledPlot

use std::collections::{HashMap, HashSet};

use crate::{error::AvengerChartError, scales::ConfiguredScaleWithSpec};

use super::CompiledPlot;

impl CompiledPlot {
    /// Validate that all required positional scales exist
    pub(super) fn validate_positional_scales_exist(
        &self,
        _scales: &HashMap<String, ConfiguredScaleWithSpec>,
    ) -> Result<(), AvengerChartError> {
        // Get the required positional channels from the coordinate system
        let required_channels = self.coord_transform.required_channels();

        // Build a set of positional channel names to check
        // Include both base channels and their interval variants (e.g., "x" and "x2")
        let mut positional_channels = HashSet::new();
        for &channel in required_channels {
            positional_channels.insert(channel.to_string());
            // Also check for interval variant (e.g., "x2" for "x")
            positional_channels.insert(format!("{}2", channel));
        }

        // Check which marks use positional channels
        let mut used_channels = HashSet::new();
        for mark in &self.marks {
            for channel_name in mark.data_context().channels().keys() {
                if positional_channels.contains(channel_name) {
                    used_channels.insert(channel_name.clone());
                }
            }
        }

        // For now, we'll just return Ok since scale building happens elsewhere
        // In the future, we might want to validate that scales exist for used channels
        Ok(())
    }

    /// Validate that positional channels have numeric data types
    pub(super) fn validate_positional_channel_types(
        &self,
        data_batch: &Option<datafusion::arrow::record_batch::RecordBatch>,
        scalar_batch: &datafusion::arrow::record_batch::RecordBatch,
    ) -> Result<(), AvengerChartError> {
        // Get coordinate system name for error messages
        let coord_system_name = std::any::type_name_of_val(&*self.coord_transform);

        // Check each positional channel
        for channel_name in self.coord_transform.required_channels() {
            // Check in data batch first
            if let Some(data) = data_batch {
                if let Some(column) = data.column_by_name(channel_name) {
                    let dtype = column.data_type();
                    if !Self::is_numeric_type(dtype) {
                        return self.create_positional_type_error(
                            channel_name,
                            dtype,
                            coord_system_name,
                        );
                    }
                }
            }

            // Check in scalar batch
            if let Some(column) = scalar_batch.column_by_name(channel_name) {
                let dtype = column.data_type();
                if !Self::is_numeric_type(dtype) {
                    return self.create_positional_type_error(
                        channel_name,
                        dtype,
                        coord_system_name,
                    );
                }
            }
        }

        Ok(())
    }

    /// Create error for non-numeric positional channel
    fn create_positional_type_error(
        &self,
        channel_name: &str,
        dtype: &datafusion::arrow::datatypes::DataType,
        coord_system_name: &str,
    ) -> Result<(), AvengerChartError> {
        use datafusion::arrow::datatypes::DataType;

        // Extract coordinate system name (remove module path)
        let coord_system = coord_system_name
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
            coord_system,
            literal_value,
            suggestion,
        })
    }
}
