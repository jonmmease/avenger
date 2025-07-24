//! Binning transforms for continuous data

use super::core::{ChannelInfo, DataContext, Transform};
use crate::coords::{Cartesian, CoordinateSystem, Polar};
use crate::error::AvengerChartError;
use datafusion::logical_expr::Expr;
use std::marker::PhantomData;

/// Configuration for binning a single field
#[derive(Debug, Clone)]
struct BinConfig {
    /// The field to bin
    field: String,

    /// Output channel for bin start
    channel_start: String,

    /// Output channel for bin end  
    channel_end: String,

    /// Bin width (if specified)
    width: Option<f64>,

    /// Number of bins (if specified)
    bins: Option<usize>,

    /// Nice domain boundaries
    nice: bool,

    /// Domain bounds
    domain: Option<(f64, f64)>,
}

/// Transform that bins multiple fields based on coordinate system
pub struct BinNd<C: CoordinateSystem> {
    /// Configuration for each field being binned
    configs: Vec<BinConfig>,

    /// Optional aggregation expression
    agg: Option<Expr>,

    /// Additional aggregations that map to specific channels
    /// Each tuple is (channel, expression)
    extra_aggs: Vec<(&'static str, Expr)>,

    /// Phantom type for coordinate system
    _phantom: PhantomData<C>,
}

/// Transform that bins a single field based on coordinate system
pub struct Bin<C: CoordinateSystem> {
    /// Inner N-dimensional bin with single field
    inner: BinNd<C>,
}

// General constructor and configuration methods for N-dimensional binning
impl<C: CoordinateSystem> BinNd<C> {
    /// Create a bin transform with explicit field-to-channel mappings
    pub fn new<S: Into<String>, T: Into<String>>(fields: Vec<(S, T)>) -> Self {
        Self {
            configs: fields
                .into_iter()
                .map(|(field, channel)| {
                    let channel = channel.into();
                    BinConfig {
                        field: field.into(),
                        channel_start: channel.clone(),
                        channel_end: format!("{channel}2"),
                        width: None,
                        bins: None,
                        nice: true,
                        domain: None,
                    }
                })
                .collect(),
            agg: None,
            extra_aggs: Vec::new(),
            _phantom: PhantomData,
        }
    }

    /// Set the aggregation expression
    pub fn aggregate(mut self, agg: Expr) -> Self {
        self.agg = Some(agg);
        self
    }

    /// Add an extra aggregation that maps to a specific channel
    pub fn extra_aggregate(mut self, channel: &'static str, expr: Expr) -> Self {
        self.extra_aggs.push((channel, expr));
        self
    }

    /// Set the bin width for a specific field by index
    pub fn width_for(mut self, index: usize, width: f64) -> Self {
        if let Some(config) = self.configs.get_mut(index) {
            config.width = Some(width);
            config.bins = None; // Clear bins if width is set
        }
        self
    }

    /// Set the number of bins for a specific field by index
    pub fn bins_for(mut self, index: usize, bins: usize) -> Self {
        if let Some(config) = self.configs.get_mut(index) {
            config.bins = Some(bins);
            config.width = None; // Clear width if bins is set
        }
        self
    }

    /// Set nice domain boundaries for a specific field by index
    pub fn nice_for(mut self, index: usize, nice: bool) -> Self {
        if let Some(config) = self.configs.get_mut(index) {
            config.nice = nice;
        }
        self
    }

    /// Set the domain for a specific field by index
    pub fn domain_for(mut self, index: usize, min: f64, max: f64) -> Self {
        if let Some(config) = self.configs.get_mut(index) {
            config.domain = Some((min, max));
        }
        self
    }
}

// Cartesian-specific constructors and methods for N-dimensional binning
impl BinNd<Cartesian> {
    /// Create a bin transform for both x and y channels
    pub fn xy(x_field: impl Into<String>, y_field: impl Into<String>) -> Self {
        Self::new(vec![(x_field.into(), "x"), (y_field.into(), "y")])
    }

    // Convenience methods for x dimension (index 0)
    pub fn width_x(self, width: f64) -> Self {
        self.width_for(0, width)
    }

    pub fn bins_x(self, bins: usize) -> Self {
        self.bins_for(0, bins)
    }

    pub fn nice_x(self, nice: bool) -> Self {
        self.nice_for(0, nice)
    }

    pub fn domain_x(self, min: f64, max: f64) -> Self {
        self.domain_for(0, min, max)
    }

    // Convenience methods for y dimension (index 1)
    pub fn width_y(self, width: f64) -> Self {
        self.width_for(1, width)
    }

    pub fn bins_y(self, bins: usize) -> Self {
        self.bins_for(1, bins)
    }

    pub fn nice_y(self, nice: bool) -> Self {
        self.nice_for(1, nice)
    }

    pub fn domain_y(self, min: f64, max: f64) -> Self {
        self.domain_for(1, min, max)
    }
}

// Polar-specific constructors and methods for N-dimensional binning
impl BinNd<Polar> {
    /// Create a bin transform for both r and theta channels
    pub fn rtheta(r_field: impl Into<String>, theta_field: impl Into<String>) -> Self {
        Self::new(vec![(r_field.into(), "r"), (theta_field.into(), "theta")])
    }

    // Convenience methods for r dimension (index 0)
    pub fn width_r(self, width: f64) -> Self {
        self.width_for(0, width)
    }

    pub fn bins_r(self, bins: usize) -> Self {
        self.bins_for(0, bins)
    }

    // Convenience methods for theta dimension (index 1)
    pub fn width_theta(self, width: f64) -> Self {
        self.width_for(1, width)
    }

    pub fn bins_theta(self, bins: usize) -> Self {
        self.bins_for(1, bins)
    }
}

// Single-dimensional bin implementation
impl<C: CoordinateSystem> Bin<C> {
    /// Set the bin width
    pub fn width(self, width: f64) -> Self {
        Self {
            inner: self.inner.width_for(0, width),
        }
    }

    /// Set the number of bins
    pub fn bins(self, bins: usize) -> Self {
        Self {
            inner: self.inner.bins_for(0, bins),
        }
    }

    /// Set nice domain boundaries
    pub fn nice(self, nice: bool) -> Self {
        Self {
            inner: self.inner.nice_for(0, nice),
        }
    }

    /// Set the domain
    pub fn domain(self, min: f64, max: f64) -> Self {
        Self {
            inner: self.inner.domain_for(0, min, max),
        }
    }

    /// Set the aggregation expression
    pub fn aggregate(self, agg: Expr) -> Self {
        Self {
            inner: self.inner.aggregate(agg),
        }
    }

    /// Add an extra aggregation that maps to a specific channel
    pub fn extra_aggregate(self, channel: &'static str, expr: Expr) -> Self {
        Self {
            inner: self.inner.extra_aggregate(channel, expr),
        }
    }
}

// Cartesian-specific constructors for single-dimensional binning
impl Bin<Cartesian> {
    /// Create a bin transform for the x channel
    pub fn x(field: impl Into<String>) -> Self {
        Self {
            inner: BinNd::new(vec![(field, "x")]),
        }
    }

    /// Create a bin transform for the y channel
    pub fn y(field: impl Into<String>) -> Self {
        Self {
            inner: BinNd::new(vec![(field, "y")]),
        }
    }
}

// Polar-specific constructors for single-dimensional binning
impl Bin<Polar> {
    /// Create a bin transform for the radius channel
    pub fn r(field: impl Into<String>) -> Self {
        Self {
            inner: BinNd::new(vec![(field, "r")]),
        }
    }

    /// Create a bin transform for the theta/angle channel
    pub fn theta(field: impl Into<String>) -> Self {
        Self {
            inner: BinNd::new(vec![(field, "theta")]),
        }
    }
}

impl<C: CoordinateSystem> Transform for BinNd<C> {
    fn transform(&self, ctx: DataContext) -> Result<DataContext, AvengerChartError> {
        let df = ctx.dataframe().clone();

        // Validate each config has either width or bins
        for (i, config) in self.configs.iter().enumerate() {
            if config.width.is_none() && config.bins.is_none() {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Bin transform for field {} (index {}) requires either width or bins to be specified",
                    config.field, i
                )));
            }
        }

        // TODO: Implement actual binning logic
        // For now, we'll create a stub implementation

        // The binning would:
        // 1. Calculate bin boundaries for each field
        // 2. Create new columns for bin starts/ends
        // 3. Group by all binned dimensions
        // 4. Apply aggregation if specified

        let mut result = DataContext::new(df);

        // Map each binned field to its channels
        for config in &self.configs {
            // Create column names based on the field
            let bin_start_col = format!("{}_bin_start", config.field);
            let bin_end_col = format!("{}_bin_end", config.field);

            result = result
                .with_encoding(&config.channel_start, &bin_start_col)
                .with_encoding(&config.channel_end, &bin_end_col)
                .with_channel_metadata(
                    &config.channel_start,
                    serde_json::json!({
                        "transform": "bin",
                        "field": config.field,
                        "width": config.width,
                        "bins": config.bins,
                        "nice": config.nice,
                        "domain": config.domain,
                        "type": "quantitative"
                    }),
                );
        }

        // Add aggregation channel if specified
        // Note: We don't alias the aggregation result to preserve meaningful column names
        if let Some(ref _agg) = self.agg {
            // The aggregation column name will be determined by the actual aggregation expression
            // e.g., COUNT(*) might produce "count", SUM(sales) might produce "sum_sales"
            result = result.with_channel_metadata(
                "value",
                serde_json::json!({
                    "aggregate": format!("{:?}", self.agg),
                    "type": "quantitative"
                }),
            );
        }

        // Add additional aggregations with channel mappings
        for (channel, expr) in &self.extra_aggs {
            // DataFusion will generate column name from expression
            // e.g., AVG(price) -> "avg_price", COUNT(*) -> "count"
            result = result.with_channel_metadata(
                channel,
                serde_json::json!({
                    "aggregate": format!("{:?}", expr),
                    "type": "quantitative"
                }),
            );
        }

        Ok(result)
    }

    fn output_channels(&self) -> Vec<ChannelInfo> {
        let mut channels = Vec::new();

        // Add channels for each binned field
        for config in &self.configs {
            channels.push(ChannelInfo {
                name: config.channel_start.clone(),
                data_type: "quantitative".to_string(),
                required: true,
                description: format!("Bin start values for {}", config.field),
            });
            channels.push(ChannelInfo {
                name: config.channel_end.clone(),
                data_type: "quantitative".to_string(),
                required: true,
                description: format!("Bin end values for {}", config.field),
            });
        }

        if self.agg.is_some() {
            channels.push(ChannelInfo {
                name: "value".to_string(),
                data_type: "quantitative".to_string(),
                required: false,
                description: "Aggregated values".to_string(),
            });
        }

        channels
    }
}

// Delegate Transform implementation for single-dimensional Bin to BinNd
impl<C: CoordinateSystem> Transform for Bin<C> {
    fn transform(&self, ctx: DataContext) -> Result<DataContext, AvengerChartError> {
        self.inner.transform(ctx)
    }

    fn output_channels(&self) -> Vec<ChannelInfo> {
        self.inner.output_channels()
    }
}
