use crate::coords::CoordinateSystem;
use crate::plot::Plot;
use crate::scales::Scale;

/// Macro to generate scale methods for Plot
macro_rules! scale_methods {
    ($($method:ident => $channel:expr),* $(,)?) => {
        $(
            pub fn $method<F>(mut self, f: F) -> Self
            where F: FnOnce(Scale) -> Scale
            {
                let scale = self.get_or_create_scale($channel);
                let scale = f(scale);
                self.scales.insert($channel.to_string(), scale);
                self
            }
        )*
    };
}

/// Methods for adding scales to Plot
impl<C: CoordinateSystem> Plot<C> {
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
