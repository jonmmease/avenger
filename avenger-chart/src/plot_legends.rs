use crate::plot::Plot;
use crate::coords::CoordinateSystem;
use crate::legend::Legend;

/// Macro to generate legend methods for Plot
macro_rules! legend_methods {
    ($($method:ident => $channel:expr),* $(,)?) => {
        $(
            pub fn $method<F>(mut self, f: F) -> Self 
            where F: FnOnce(Legend) -> Legend
            {
                let current = self.legends.remove($channel)
                    .unwrap_or_else(|| Legend::new());
                let legend = f(current);
                self.legends.insert($channel.to_string(), legend);
                self
            }
        )*
    };
}

/// Methods for adding legends to Plot
impl<C: CoordinateSystem> Plot<C> {
    legend_methods! {
        // Color legends
        legend_fill => "fill",
        legend_stroke => "stroke",
        legend_color => "color",
        
        // Size legends
        legend_size => "size",
        legend_stroke_width => "stroke_width",
        legend_font_size => "font_size",
        legend_width => "width",
        legend_height => "height",
        legend_outer_radius => "outer_radius",
        legend_inner_radius => "inner_radius",
        legend_corner_radius => "corner_radius",
        
        // Angle legends
        legend_angle => "angle",
        legend_start_angle => "start_angle",
        legend_end_angle => "end_angle",
        legend_pad_angle => "pad_angle",
        
        // Text-related legends
        legend_text => "text",
        legend_font => "font",
        legend_font_weight => "font_weight",
        legend_font_style => "font_style",
        legend_align => "align",
        legend_baseline => "baseline",
        legend_limit => "limit",
        
        // Shape legends
        legend_shape => "shape",
        legend_shape_index => "shape_index",
        
        // Stroke style legends
        legend_stroke_cap => "stroke_cap",
        legend_stroke_dash => "stroke_dash",
        
        // Boolean legends
        legend_defined => "defined",
        
        // Other legends
        legend_opacity => "opacity",
        legend_interpolate => "interpolate",
        legend_path => "path",
        legend_transform => "transform",
        legend_image => "image",
    }
}