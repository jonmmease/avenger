//! Grouping transforms for aggregating data

use datafusion::logical_expr::Expr;
use std::marker::PhantomData;
use crate::coords::{CoordinateSystem, Cartesian, Polar};
use crate::error::AvengerChartError;
use super::core::{Transform, DataContext, ChannelInfo};

/// Transform that groups by one or more fields based on coordinate system
pub struct Group<C: CoordinateSystem> {
    /// The fields to group by and their output channels
    /// e.g., [("category", "x"), ("region", "y")]
    group_fields: Vec<(String, String)>,
    
    /// Default output channel for the primary aggregation
    default_agg_channel: String,
    
    /// Primary aggregation (maps to the complement channel)
    primary_agg: Option<Expr>,
    
    /// Additional aggregations that map to specific channels
    /// Each tuple is (channel, expression)
    extra_aggs: Vec<(&'static str, Expr)>,
    
    /// Phantom type for coordinate system
    _phantom: PhantomData<C>,
}

// General constructor
impl<C: CoordinateSystem> Group<C> {
    /// Create a group transform with explicit field-to-channel mappings
    pub fn new<S: Into<String>, T: Into<String>>(fields: Vec<(S, T)>, default_agg_channel: &str) -> Self {
        Self {
            group_fields: fields.into_iter()
                .map(|(field, channel)| (field.into(), channel.into()))
                .collect(),
            default_agg_channel: default_agg_channel.to_string(),
            primary_agg: None,
            extra_aggs: Vec::new(),
            _phantom: PhantomData,
        }
    }
    
    /// Set the primary aggregation (maps to the complement channel)
    pub fn aggregate(mut self, expr: Expr) -> Self {
        self.primary_agg = Some(expr);
        self
    }
    
    /// Add an extra aggregation that maps to a specific channel
    pub fn extra_aggregate(mut self, channel: &'static str, expr: Expr) -> Self {
        self.extra_aggs.push((channel, expr));
        self
    }
}

// Cartesian-specific constructors
impl Group<Cartesian> {
    /// Create a group transform for the x channel
    pub fn x(field: impl Into<String>) -> Self {
        Self::new(vec![(field, "x")], "y")
    }
    
    /// Create a group transform for the y channel
    pub fn y(field: impl Into<String>) -> Self {
        Self::new(vec![(field, "y")], "x")
    }
    
    /// Create a group transform for both x and y channels
    /// Requires specifying the aggregation channel since there's no natural complement
    pub fn xy(x_field: impl Into<String>, y_field: impl Into<String>, agg_channel: &str) -> Self {
        Self::new(vec![(x_field.into(), "x"), (y_field.into(), "y")], agg_channel)
    }
    
    /// Create a group transform for x channel and fill (for stacking)
    /// Aggregations naturally go to y channel
    pub fn xfill(x_field: impl Into<String>, fill_field: impl Into<String>) -> Self {
        Self::new(vec![(x_field.into(), "x"), (fill_field.into(), "fill")], "y")
    }
    
    /// Create a group transform for y channel and fill (for horizontal stacking)
    /// Aggregations naturally go to x channel
    pub fn yfill(y_field: impl Into<String>, fill_field: impl Into<String>) -> Self {
        Self::new(vec![(y_field.into(), "y"), (fill_field.into(), "fill")], "x")
    }
    
    /// Create a group transform for x, y channels and fill
    /// Requires specifying the aggregation channel since there's no natural complement
    pub fn xyfill(x_field: impl Into<String>, y_field: impl Into<String>, fill_field: impl Into<String>, agg_channel: &str) -> Self {
        Self::new(vec![
            (x_field.into(), "x"), 
            (y_field.into(), "y"), 
            (fill_field.into(), "fill")
        ], agg_channel)
    }
}

// Polar-specific constructors
impl Group<Polar> {
    /// Create a group transform for the radius channel
    pub fn r(field: impl Into<String>) -> Self {
        Self::new(vec![(field, "r")], "theta")
    }
    
    /// Create a group transform for the theta/angle channel
    pub fn theta(field: impl Into<String>) -> Self {
        Self::new(vec![(field, "theta")], "r")
    }
    
    pub fn rtheta(r_field: impl Into<String>, theta_field: impl Into<String>, agg_channel: &str) -> Self {
        Self::new(vec![(r_field.into(), "r"), (theta_field.into(), "theta")], agg_channel)
    }
}

impl<C: CoordinateSystem> Transform for Group<C> {
    fn transform(&self, ctx: DataContext) -> Result<DataContext, AvengerChartError> {
        let df = ctx.dataframe().clone();
        
        // TODO: Implement actual grouping logic
        // This would use DataFrame.aggregate() with appropriate expressions
        
        let mut result = DataContext::new(df);
        
        // Preserve existing encodings from input context
        for channel in ctx.channels() {
            if let Some(column) = ctx.encoding(channel) {
                result = result.with_encoding(channel, &column);
            }
        }
        
        // Map each group field to its channel
        for (field, channel) in &self.group_fields {
            result = result
                .with_encoding(channel, field)
                .with_channel_metadata(channel, serde_json::json!({
                    "field": field,
                    "type": "ordinal" // Could be determined from data
                }));
        }
        
        // Add channel for primary aggregation if present
        if let Some(ref expr) = self.primary_agg {
            result = result
                .with_encoding(&self.default_agg_channel, &format!("agg_{}", self.default_agg_channel))
                .with_channel_metadata(&self.default_agg_channel, serde_json::json!({
                    "aggregate": format!("{:?}", expr),
                    "type": "quantitative"
                }));
        }
        
        // Add additional aggregations with channel mappings
        for (channel, expr) in &self.extra_aggs {
            // DataFusion will generate column name from expression
            // e.g., AVG(price) -> "avg_price", COUNT(*) -> "count"
            result = result
                .with_channel_metadata(channel, serde_json::json!({
                    "aggregate": format!("{:?}", expr),
                    "type": "quantitative"
                }));
        }
        
        Ok(result)
    }
    
    fn output_channels(&self) -> Vec<ChannelInfo> {
        let mut channels: Vec<ChannelInfo> = self.group_fields.iter()
            .map(|(field, channel)| ChannelInfo {
                name: channel.clone(),
                data_type: "ordinal".to_string(),
                required: true,
                description: format!("Grouped by {}", field),
            })
            .collect();
        
        if self.primary_agg.is_some() {
            channels.push(ChannelInfo {
                name: self.default_agg_channel.clone(),
                data_type: "quantitative".to_string(),
                required: false,
                description: "Primary aggregated values".to_string(),
            });
        }
        
        channels
    }
}