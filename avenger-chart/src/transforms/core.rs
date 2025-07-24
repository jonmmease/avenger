//! Core types and traits for the transform system

use crate::error::AvengerChartError;
use crate::marks::channel::ChannelValue;
use datafusion::common::tree_node::{Transformed, TreeNode};
use datafusion::dataframe::DataFrame;
use indexmap::IndexMap;
use serde_json::Value;
use std::collections::HashMap;

/// Context that flows through the mark building pipeline
/// Carries both data and encoding information
#[derive(Debug, Clone)]
pub struct DataContext {
    /// The underlying DataFrame
    dataframe: DataFrame,

    /// Encodings mapping channels to expressions and scale info
    /// e.g., "x" → ChannelValue { expr: col("month"), scale: Default }
    encodings: IndexMap<String, ChannelValue>,

    /// Metadata associated with each channel
    /// e.g., "x" → {transform: "bin", width: 10.0}
    channel_metadata: HashMap<String, Value>,

    /// General metadata about applied transforms
    transform_metadata: Vec<Value>,
}

impl DataContext {
    /// Create from a DataFrame
    pub fn new(dataframe: DataFrame) -> Self {
        Self {
            dataframe,
            encodings: IndexMap::new(),
            channel_metadata: HashMap::new(),
            transform_metadata: Vec::new(),
        }
    }

    /// Create a default DataContext with a unit DataFrame
    pub fn default() -> Self {
        use datafusion::prelude::SessionContext;

        // Create a unit DataFrame (single row, no columns)
        let ctx = SessionContext::new();
        let df = ctx.read_empty().unwrap();

        Self::new(df)
    }

    /// Get the underlying DataFrame
    pub fn dataframe(&self) -> &DataFrame {
        &self.dataframe
    }

    /// Consume self and return the DataFrame
    pub fn into_dataframe(self) -> DataFrame {
        self.dataframe
    }

    /// Set or update an encoding (channel → column mapping)
    pub fn with_encoding(self, channel: &str, column: &str) -> Self {
        use datafusion::logical_expr::ident;

        // Create the expression from the column name
        let expr = ident(column);

        // Use with_encoding_expr to handle it
        self.with_encoding_expr(channel, expr)
    }

    /// Set or update an encoding with an arbitrary expression
    pub fn with_encoding_expr(
        mut self,
        channel: &str,
        expr: datafusion::logical_expr::Expr,
    ) -> Self {
        // Resolve any channel references in the expression
        let resolved_expr = self.resolve_channel_refs(expr);

        // Store the encoding
        self.encodings
            .insert(channel.to_string(), ChannelValue::new(resolved_expr));

        self
    }

    /// Set or update an encoding with a ChannelValue
    pub fn with_channel_value(mut self, channel: &str, mut value: ChannelValue) -> Self {
        // Resolve any channel references in the expression
        value.expr = self.resolve_channel_refs(value.expr);

        // Store the encoding
        self.encodings.insert(channel.to_string(), value);

        self
    }

    /// Add metadata for a channel
    pub fn with_channel_metadata(mut self, channel: &str, metadata: Value) -> Self {
        self.channel_metadata.insert(channel.to_string(), metadata);
        self
    }

    /// Add transform metadata
    pub fn with_transform_metadata(mut self, metadata: Value) -> Self {
        self.transform_metadata.push(metadata);
        self
    }

    /// Resolve a channel reference or column name
    /// - If starts with ':', treats as channel reference and looks up encoding
    /// - Otherwise returns the string as-is (direct column name)
    pub fn resolve(&self, channel_or_column: &str) -> Option<String> {
        if channel_or_column.starts_with(':') {
            // Channel reference - look up the encoding
            let channel_name = &channel_or_column[1..];
            self.encoding(channel_name)
        } else {
            // Direct column name
            Some(channel_or_column.to_string())
        }
    }

    /// Get the column name for a channel (without : prefix)
    /// For simple column references, returns the column name
    /// For complex expressions, returns ":channel"
    pub fn encoding(&self, channel: &str) -> Option<String> {
        self.encodings.get(channel).map(|channel_value| {
            if let datafusion::logical_expr::Expr::Column(col) = &channel_value.expr {
                col.name.clone()
            } else {
                format!(":{}", channel)
            }
        })
    }

    /// Get the expression for a channel
    pub fn encoding_expr(&self, channel: &str) -> Option<&datafusion::logical_expr::Expr> {
        self.encodings.get(channel).map(|cv| &cv.expr)
    }

    /// Get the expression string for a channel
    pub fn encoding_expr_string(&self, channel: &str) -> Option<String> {
        self.encodings.get(channel).map(|cv| cv.expr.to_string())
    }

    /// Get metadata for a channel
    pub fn channel_metadata(&self, channel: &str) -> Option<&Value> {
        self.channel_metadata.get(channel)
    }

    /// List all encoded channels
    pub fn channels(&self) -> Vec<&str> {
        self.encodings.keys().map(|s| s.as_str()).collect()
    }

    /// Get the encodings
    pub fn encodings(&self) -> &IndexMap<String, ChannelValue> {
        &self.encodings
    }

    /// Resolve channel references in an expression
    /// Replaces :channel column references with the actual expression for that channel
    fn resolve_channel_refs(
        &self,
        expr: datafusion::logical_expr::Expr,
    ) -> datafusion::logical_expr::Expr {
        use datafusion::logical_expr::Expr;

        // Use transform_down to replace column references
        let result = expr.transform_down(|e| {
            match &e {
                Expr::Column(col) if col.name.starts_with(':') => {
                    // This is a channel reference
                    let channel_name = &col.name[1..]; // Remove the ':' prefix

                    // Look up the expression for this channel
                    if let Some(channel_value) = self.encodings.get(channel_name) {
                        // Return the expression for this channel
                        // Note: we clone here to avoid infinite recursion
                        Ok(Transformed::yes(channel_value.expr.clone()))
                    } else {
                        // Channel not found, keep the original column reference
                        Ok(Transformed::no(e))
                    }
                }
                _ => Ok(Transformed::no(e)),
            }
        });

        // Extract the expression from the result
        match result {
            Ok(transformed) => transformed.data,
            Err(e) => {
                // Log the error and return a column reference as fallback
                eprintln!("Error resolving channel references: {:?}", e);
                datafusion::logical_expr::col("error")
            }
        }
    }
}

/// Trait for data that can be transformed
pub trait TransformInput {
    /// Convert to DataContext
    fn into_context(self) -> DataContext;
}

impl TransformInput for DataFrame {
    fn into_context(self) -> DataContext {
        DataContext::new(self)
    }
}

impl TransformInput for DataContext {
    fn into_context(self) -> DataContext {
        self
    }
}

/// Base trait for all transforms
pub trait Transform: Send + Sync {
    /// Apply the transform to data context
    fn transform(&self, ctx: DataContext) -> Result<DataContext, AvengerChartError>;

    /// Get a description of what channels this transform produces
    fn output_channels(&self) -> Vec<ChannelInfo>;
}

/// Information about a channel produced by a transform
#[derive(Debug, Clone)]
pub struct ChannelInfo {
    /// The channel name (e.g., "x", "y", "x2")
    pub name: String,

    /// The type of data (e.g., "quantitative", "ordinal", "temporal")
    pub data_type: String,

    /// Whether this channel is required or optional
    pub required: bool,

    /// Description of what this channel represents
    pub description: String,
}
