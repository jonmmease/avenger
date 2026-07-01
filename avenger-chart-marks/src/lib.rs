//! Primitive chart mark builders.
//!
//! Compound/statistical marks that expand to groups of these primitives live in
//! sibling crates such as `avenger-chart-marks-statistical`.

pub mod area;
pub mod compiled_data_context;
pub mod data_context;
pub mod facet_data_scope;
pub mod image;
pub mod line;
pub mod path;
pub mod rect;
pub mod rule;
pub mod state;
pub mod subplot;
pub mod symbol;
pub mod text;
pub mod trail;
pub mod uniform_raster_2d;
pub mod zero_d;

pub use area::{Area, AreaPartitionKey, area_channel_defaults};
pub use compiled_data_context::CompiledDataContext;
pub use data_context::DataContext;
pub use facet_data_scope::FacetDataScope;
pub use image::{Image, image_channel_defaults};
pub use line::{Line, PartitionKey, ensure_dictionary_array, line_channel_defaults};
pub use path::{PathMark, path_channel_defaults};
pub use rect::{Rect, rect_channel_defaults};
pub use rule::{Rule, rule_channel_defaults};
pub use state::{CompiledMarkState, MarkState};
pub use subplot::Subplot;
pub use symbol::{
    IntoDerivedPrimitiveMark, Symbol, symbol_channel_defaults, symbol_legend_renderer_kind,
};
pub use text::{Text, text_channel_defaults};
pub use trail::{Trail, TrailPartitionKey, trail_channel_defaults};
pub use uniform_raster_2d::{
    RasterChannelsConfig, RasterPositionConfig, UNIFORM_RASTER_2D_FILL_CHANNEL,
    UNIFORM_RASTER_2D_RASTER_CHANNEL, UniformRaster2D, UniformRaster2DFields,
    UniformRaster2DOptions, uniform_raster_2d_channel_defaults,
};
pub use zero_d::CompiledZeroDSymbol;
