use crate::coords::CoordinateSystem;
use crate::plot::{Plot, ScaleSpec};
use crate::scales::{Auto, Scale, ScaleSpec as ScaleTypeSpec};
use std::sync::Arc;

/// Macro to generate scale methods for Plot
/// Generates both Auto version (keeps inferred type) and typed version (changes impl)
macro_rules! scale_methods {
    ($($method:ident => $channel:expr),* $(,)?) => {
        $(
            paste::paste! {
                /// Configure scale with inferred type
                pub fn $method<F>(mut self, f: F) -> Self
                where F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static
                {
                    self.scale_specs
                        .insert($channel.to_string(), ScaleSpec::Local(Arc::new(f)));
                    self
                }

                /// Configure scale with explicit type
                pub fn [<$method _with>]<S: ScaleTypeSpec>(mut self, f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static) -> Self
                {
                    self.scale_specs.insert(
                        $channel.to_string(),
                        ScaleSpec::Local(Arc::new(move |default_scale| {
                            // Convert the default scale to the requested type
                            // This changes the scale_impl to match type S while preserving domain/range/options
                            let typed_scale = default_scale.into_type::<S>();
                            f(typed_scale).into_auto()
                        })),
                    );
                    self
                }
            }
        )*
    };
}

/// Methods for adding scales to Plot
impl<C: CoordinateSystem> Plot<C> {
    /// Configure a scale by channel name
    pub fn scale<F>(mut self, channel: &str, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        self.scale_specs
            .insert(channel.to_string(), ScaleSpec::Local(Arc::new(f)));
        self
    }

    /// Configure a scale with explicit type
    pub fn scale_with<S: ScaleTypeSpec>(
        mut self,
        channel: &str,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        self.scale_specs.insert(
            channel.to_string(),
            ScaleSpec::Local(Arc::new(move |default_scale| {
                let typed_scale = default_scale.into_type::<S>();
                f(typed_scale).into_auto()
            })),
        );
        self
    }

    // Temporarily keep old methods for backwards compatibility
    scale_methods! {
        // Color scales
        scale_fill => "fill",
        scale_stroke => "stroke",
        scale_color => "color",

        // Size scales
        scale_size => "size",
        scale_stroke_width => "stroke_width",
        scale_font_size => "font_size",
        scale_width => "width",
        scale_height => "height",
        scale_outer_radius => "outer_radius",
        scale_inner_radius => "inner_radius",
        scale_corner_radius => "corner_radius",

        // Angle scales
        scale_angle => "angle",
        scale_start_angle => "start_angle",
        scale_end_angle => "end_angle",
        scale_pad_angle => "pad_angle",

        // Text-related scales
        scale_text => "text",
        scale_font => "font",
        scale_font_weight => "font_weight",
        scale_font_style => "font_style",
        scale_align => "align",
        scale_baseline => "baseline",
        scale_limit => "limit",

        // Shape scales
        scale_shape => "shape",
        scale_shape_index => "shape_index",

        // Stroke style scales
        scale_stroke_cap => "stroke_cap",
        scale_stroke_dash => "stroke_dash",

        // Boolean scales
        scale_defined => "defined",

        // Other scales
        scale_opacity => "opacity",
        scale_interpolate => "interpolate",
        scale_path => "path",
        scale_transform => "transform",
        scale_image => "image",
    }
}
