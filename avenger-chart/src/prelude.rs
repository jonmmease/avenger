//! Prelude for avenger-chart
//!
//! This module provides a convenient way to import the most commonly used types and traits
//! when working with avenger-chart.
//!
//! # Example
//! ```rust,ignore
//! use avenger_chart::prelude::*;
//!
//! let plot = Plot::<Cartesian>::new()
//!     .mark(Symbol::new()
//!         .x(col("x"))
//!         .y(col("y"))
//!         .fill_with(col("category"), |c| c
//!             .scale(|s| s.scheme("category10"))
//!             .legend(|l| l.title("Category"))
//!         )
//!     );
//! ```

// Re-export coordinate systems
pub use crate::cartesian::Cartesian;
pub use crate::polar::Polar;
pub use crate::zerod::ZeroDCoord;

// Re-export the Plot type
pub use crate::plot::{Plot, PlotSubtitle, PlotTitle, TitleAlign};

// Re-export theme types
pub use crate::theme::{FontWeight, NamedFontWeight, Theme};

// Re-export marks
pub use crate::marks::line::Line;
pub use crate::marks::rect::Rect;
pub use crate::marks::symbol::Symbol;

// Re-export mark traits and types
pub use crate::marks::{
    ChannelValue, ConditionalValue, FacetStrategy, Mark, MarkState, RadiusExpression,
};

// Re-export channel config traits - ESSENTIAL for using channel methods
pub use crate::channel_config_traits::{ChannelConfig, LegendableChannel};

// Re-export channel configs for direct use
pub use crate::channel_configs::{
    AngleChannelConfig, ColorChannelConfig, OpacityChannelConfig, ShapeChannelConfig,
    SizeChannelConfig, StrokeDashChannelConfig, StrokeWidthChannelConfig,
};

// Re-export position channel config for Cartesian
pub use crate::cartesian::channels::{CartesianPositionConfig, PositionConfig};

// Re-export scale types
pub use crate::scales::{
    Auto, Band, Linear, Log, Ordinal, Point, Pow, Quantile, Quantize, Scale, Sqrt, Symlog,
    Threshold, Time,
};

// Re-export legend types
pub use crate::legend::{Legend, LegendOrientation, LegendPosition};
pub use crate::legend_builder::{
    AngleLegendBuilder, ColorLegendBuilder, LegendBuilder, OpacityLegendBuilder,
    ShapeLegendBuilder, SizeLegendBuilder, StrokeDashLegendBuilder, StrokeWidthLegendBuilder,
};

// Re-export axis types
pub use crate::axis::{Axis, AxisPosition};
pub use crate::cartesian::CartesianAxis;
pub use crate::polar::PolarAxis;

// Re-export rendering types
pub use crate::render::{CanvasExt, PlotRenderer};

// Re-export error type
pub use crate::error::AvengerChartError;

// Re-export DataFusion prelude for data manipulation
pub use datafusion::prelude::{Expr, col, lit};

// Re-export common DataFrame type
pub use datafusion::dataframe::DataFrame;
