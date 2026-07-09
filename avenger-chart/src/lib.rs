#![allow(
    clippy::borrowed_box,
    clippy::items_after_test_module,
    clippy::large_enum_variant,
    clippy::module_inception,
    clippy::too_many_arguments,
    clippy::type_complexity
)]

//! High-level chart authoring API.
//!
//! Mark effects are intentionally available on built-in primitive marks and on
//! compound-specific child configuration hooks, not on arbitrary plot marks or
//! compound containers.
//!
//! ```compile_fail
//! use avenger_chart::prelude::*;
//!
//! let _ = MarkGroup::<Cartesian>::new().adjust(|item| item);
//! ```
//!
//! ```compile_fail
//! use avenger_chart::prelude::*;
//!
//! let _ = MarkGroup::<Cartesian>::new()
//!     .adjust_transform(Nudge::new(1.0, 0.0), |group, _nudge| group);
//! ```
//!
//! ```compile_fail
//! use avenger_chart::prelude::*;
//!
//! let _ = Subplot::new(Plot::<Cartesian>::new()).derive(|item| Symbol::new());
//! ```
//!
//! ```compile_fail
//! use avenger_chart::prelude::*;
//!
//! let _ = BoxPlot::new().adjust(|item| item);
//! ```
//!
//! ```compile_fail
//! use avenger_chart::prelude::*;
//!
//! let _ = BoxPlot::new()
//!     .adjust_transform(Nudge::new(1.0, 0.0), |plot, _nudge| plot);
//! ```
//!
//! ```compile_fail
//! use avenger_chart::prelude::*;
//!
//! let _ = Violin::new().derive(|item| Symbol::new());
//! ```
//!
//! ```compile_fail
//! use avenger_chart::prelude::*;
//!
//! let _ = Violin::new().adjust(|item| item);
//! ```
//!
//! ```compile_fail
//! use avenger_chart::prelude::*;
//!
//! struct ExternalMark;
//!
//! let _ = ExternalMark.derive(|item| Symbol::new());
//! ```
//!
//! ```compile_fail
//! use avenger_chart::prelude::*;
//!
//! fn generic_plot_mark<M: IntoPlotMark<Cartesian>>(mark: M) {
//!     let _ = mark.adjust(|item| item);
//! }
//! ```
//!
//! ```compile_fail
//! use avenger_chart::prelude::*;
//!
//! fn dynamic_mark(mark: &dyn Mark<Cartesian>) {
//!     let _ = mark.derive(|item| Symbol::new());
//! }
//! ```
//!
//! ```compile_fail
//! use avenger_chart::prelude::*;
//!
//! let _ = Symbol::<Cartesian>::new()
//!     .unit_data()
//!     .derive(|_point| MarkGroup::<Cartesian>::new());
//! ```
//!
//! ```compile_fail
//! use avenger_chart::prelude::*;
//!
//! let _ = Symbol::<Cartesian>::new()
//!     .unit_data()
//!     .derive(|_point| BoxPlot::new());
//! ```

pub mod axis;
pub mod bake;
pub mod cartesian;
pub mod channel;
pub mod concat;
#[doc(hidden)]
pub mod container;
pub(crate) mod coordinate_slot_overlay;
pub mod coords;
pub mod doc;
pub mod error;
pub mod event;
pub mod facet;
pub mod fonts;
pub mod guide;
pub mod layout;
pub mod legend;
pub mod marks;
pub mod maybe;
pub mod param;
pub(crate) mod partition;
pub mod plot;
pub mod polar;
pub(crate) mod positioned_subplot;
pub mod prelude;
pub mod render;
pub mod repeat;
pub mod scales;
pub mod scene_query;
pub mod selection;
pub mod serialization;
pub(crate) mod task {
    use std::{future::Future, pin::Pin};

    #[cfg(target_arch = "wasm32")]
    pub(crate) type ChartFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) type ChartFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
}
pub mod theme;
pub mod tools;
pub mod transforms {
    pub use avenger_chart_transforms::*;
}
pub mod utils;
pub mod zerod;

#[cfg(feature = "parallel")]
pub mod parallel {
    pub use avenger_chart_parallel::*;
}

pub use avenger_chart_core::CartesianUnitAspect;
pub use avenger_chart_core::{
    define_common_mark_channels, define_position_channels, impl_mark_base, impl_mark_trait_common,
    impl_supported_channels,
};
pub use avenger_scenegraph::marks::pattern::{
    PatternAnchor, PatternFill, PatternInk, PatternLayer, PatternLayerOperation,
    PatternReferenceFrame, PatternSymbol, StripeDash, StripePatternLayer, SymbolLattice2d,
    SymbolPaint, SymbolPatternLayer,
};

#[cfg(test)]
mod serialization_tests {
    use datafusion::prelude::col;
    use datafusion_proto::protobuf::LogicalExprNode;

    use crate::{channel::value::ChannelValue, serialization::LogicalExprNodeExt};

    #[test]
    fn test_channel_value_serialization() {
        // Test that we can create a ChannelValue with LogicalExprNode
        let expr = col("test");
        let expr_node = LogicalExprNode::from_expr(expr).expect("Failed to serialize expr");

        let channel = ChannelValue::Scaled {
            expr: expr_node.clone(),
            scale_name: Some("x".to_string()),
            position_boundary: None,
            scale_config: None,
            nested_band_config: None,
            legend_config: None,
            axis_config: None,
            domain_coordination: None,
            transform_scope: None,
        };

        // Serialize to JSON
        let json = serde_json::to_string_pretty(&channel).expect("Failed to serialize to JSON");
        println!("Serialized ChannelValue:\n{}", json);

        // Deserialize back
        let _deserialized: ChannelValue =
            serde_json::from_str(&json).expect("Failed to deserialize from JSON");
        println!("✓ Successfully deserialized ChannelValue");
    }
}
