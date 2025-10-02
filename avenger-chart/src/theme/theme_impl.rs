//! Implementation of theme methods for Theme

use super::Theme;
use crate::theme::{ThemeContext, ThemeValue, select_available_font};
use datafusion_common::ScalarValue;

impl Theme {
    /// Query a theme property for a given context
    pub fn query(&self, context: &ThemeContext, property: &str) -> ThemeValue {
        self.query_css(context, property)
    }

    /// Query with fallback value if property is not found
    pub fn query_or(&self, context: &ThemeContext, property: &str, fallback: ThemeValue) -> ThemeValue {
        let value = self.query(context, property);
        match value {
            ThemeValue::None => fallback,
            _ => value,
        }
    }

    /// Get the base font size in pixels (used for rem unit conversion)
    pub fn base_font_size(&self) -> f32 {
        self.base_font_size
    }

    /// Get font family for a context
    pub fn font_family(&self, context: &ThemeContext) -> String {
        let font_family_value = self.query(context, "font-family");

        // Get the list of fonts from the theme value
        let fonts = match font_family_value {
            ThemeValue::List(values) => {
                // It's already a list, extract the font names
                values
                    .into_iter()
                    .filter_map(|v| v.as_string().map(|s| s.to_string()))
                    .collect()
            }
            ThemeValue::String(s) => {
                // Single font
                vec![s]
            }
            _ => vec!["sans-serif".to_string()],
        };

        // Return the first available font from the list
        select_available_font(fonts)
    }

    /// Get font size for a context
    pub fn font_size(&self, context: &ThemeContext) -> f32 {
        self.query(context, "font-size")
            .as_pixels(self.base_font_size())
            .unwrap_or(12.0)
    }

    /// Get font weight for a context
    pub fn font_weight(&self, context: &ThemeContext) -> f32 {
        self.query(context, "font-weight")
            .as_pixels(self.base_font_size())
            .unwrap_or(400.0)
    }

    /// Get color for a context
    pub fn color(&self, context: &ThemeContext) -> String {
        self.query(context, "color")
            .to_string_value()
            .unwrap_or_else(|| "#000000".to_string())
    }

    /// Get fill color for a context
    pub fn fill_color(&self, context: &ThemeContext) -> String {
        self.query(context, "fill")
            .to_string_value()
            .unwrap_or_else(|| "#4682b4".to_string())
    }

    /// Get stroke color for a context
    pub fn stroke_color(&self, context: &ThemeContext) -> String {
        self.query(context, "stroke")
            .to_string_value()
            .unwrap_or_else(|| "#000000".to_string())
    }

    /// Get stroke width for a context
    pub fn stroke_width(&self, context: &ThemeContext) -> f32 {
        self.query(context, "stroke-width")
            .as_pixels(self.base_font_size())
            .unwrap_or(1.0)
    }

    /// Get opacity for a context
    pub fn opacity(&self, context: &ThemeContext) -> f32 {
        self.query(context, "opacity")
            .as_pixels(self.base_font_size())
            .unwrap_or(1.0)
    }

    /// Get canvas background color
    pub fn canvas_background(&self) -> Option<String> {
        let ctx = ThemeContext::new("canvas");
        match self.query(&ctx, "background-color") {
            ThemeValue::String(s) => Some(s),
            ThemeValue::Color(rgba) => {
                // Convert Color to hex string
                Some(format!(
                    "#{:02x}{:02x}{:02x}",
                    rgba.red, rgba.green, rgba.blue
                ))
            }
            _ => None,
        }
    }

    // Guide (coordinate system) theme methods

    /// Get guide background color
    ///
    /// The subtype parameter allows targeting specific coordinate systems,
    /// e.g., "cartesian", "polar"
    pub fn guide_background_color(&self, subtype: Option<&str>) -> Option<String> {
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(t) = subtype {
            guide_ctx = guide_ctx.with_subtype(t);
        }
        self.query(&guide_ctx, "background-color").to_string_value()
    }

    /// Get base font family
    pub fn base_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("base"))
    }

    // Legend-specific methods

    /// Get legend background fill
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_background_fill(&self, subtype: Option<&str>) -> Option<String> {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        let ctx = legend_ctx.child("background");
        self.query(&ctx, "fill").to_string_value()
    }

    /// Get legend background stroke
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_background_stroke(&self, subtype: Option<&str>) -> Option<String> {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        let ctx = legend_ctx.child("background");
        self.query(&ctx, "stroke").to_string_value()
    }

    /// Get legend background padding
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_background_padding(&self, subtype: Option<&str>) -> f32 {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        let ctx = legend_ctx.child("background");
        self.query(&ctx, "padding")
            .as_pixels(self.base_font_size())
            .unwrap_or(5.0)
    }

    /// Get legend background corner radius
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_background_corner_radius(&self, subtype: Option<&str>) -> f32 {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        let ctx = legend_ctx.child("background");
        self.query(&ctx, "corner-radius")
            .as_pixels(self.base_font_size())
            .unwrap_or(5.0)
    }

    /// Get legend title color
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_title_color(&self, subtype: Option<&str>) -> String {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        self.color(&legend_ctx.child("title"))
    }

    /// Get legend label color
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_label_color(&self, subtype: Option<&str>) -> String {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        self.color(&legend_ctx.child("label"))
    }

    /// Get legend tick color
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_tick_color(&self, subtype: Option<&str>) -> String {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        self.color(&legend_ctx.child("tick"))
    }

    /// Get legend title font family
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_title_font_family(&self, subtype: Option<&str>) -> String {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        self.font_family(&legend_ctx.child("title"))
    }

    /// Get legend label font family
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_label_font_family(&self, subtype: Option<&str>) -> String {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        self.font_family(&legend_ctx.child("label"))
    }

    /// Get legend tick font family
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_tick_font_family(&self, subtype: Option<&str>) -> String {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        self.font_family(&legend_ctx.child("tick"))
    }

    /// Get legend title font size
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_title_font_size(&self, subtype: Option<&str>) -> f32 {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        self.font_size(&legend_ctx.child("title"))
    }

    /// Get legend label font size
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_label_font_size(&self, subtype: Option<&str>) -> f32 {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        self.font_size(&legend_ctx.child("label"))
    }

    /// Get legend tick font size
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_tick_font_size(&self, subtype: Option<&str>) -> f32 {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        self.font_size(&legend_ctx.child("tick"))
    }

    /// Get legend title font weight
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_title_font_weight(&self, subtype: Option<&str>) -> f32 {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        self.font_weight(&legend_ctx.child("title"))
    }

    /// Get legend label font weight
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_label_font_weight(&self, subtype: Option<&str>) -> f32 {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        self.font_weight(&legend_ctx.child("label"))
    }

    /// Get legend tick font weight
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_tick_font_weight(&self, subtype: Option<&str>) -> f32 {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        self.font_weight(&legend_ctx.child("tick"))
    }

    // Title-specific methods

    /// Get title color
    pub fn title_color(&self) -> String {
        self.color(&ThemeContext::new("chart-title"))
    }

    /// Get subtitle color
    pub fn subtitle_color(&self) -> String {
        self.color(&ThemeContext::new("chart-subtitle"))
    }

    /// Get title font family
    pub fn title_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("chart-title"))
    }

    /// Get subtitle font family
    pub fn subtitle_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("chart-subtitle"))
    }

    /// Get title font size
    pub fn title_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("chart-title"))
    }

    /// Get subtitle font size
    pub fn subtitle_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("chart-subtitle"))
    }

    /// Get title font weight
    pub fn title_font_weight(&self) -> f32 {
        self.font_weight(&ThemeContext::new("chart-title"))
    }

    /// Get subtitle font weight
    pub fn subtitle_font_weight(&self) -> f32 {
        self.font_weight(&ThemeContext::new("chart-subtitle"))
    }

    // Axis-specific methods

    /// Get axis domain color
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_domain_color(&self, coord_type: Option<&str>, axis_type: Option<&str>) -> String {
        // Build context: guide[type="cartesian"] axis[type="x"] domain
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(ct) = coord_type {
            guide_ctx = guide_ctx.with_subtype(ct);
        }
        let mut axis_ctx = guide_ctx.child("axis");
        if let Some(at) = axis_type {
            axis_ctx = axis_ctx.with_subtype(at);
        }
        let ctx = axis_ctx.child("domain");
        self.query(&ctx, "stroke")
            .to_string_value()
            .unwrap_or_else(|| "#000000".to_string())
    }

    /// Get axis tick color
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_tick_color(&self, coord_type: Option<&str>, axis_type: Option<&str>) -> String {
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(ct) = coord_type {
            guide_ctx = guide_ctx.with_subtype(ct);
        }
        let mut axis_ctx = guide_ctx.child("axis");
        if let Some(at) = axis_type {
            axis_ctx = axis_ctx.with_subtype(at);
        }
        let ctx = axis_ctx.child("tick");
        self.query(&ctx, "stroke")
            .to_string_value()
            .unwrap_or_else(|| "#000000".to_string())
    }

    /// Get axis grid color
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_grid_color(&self, coord_type: Option<&str>, axis_type: Option<&str>) -> String {
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(ct) = coord_type {
            guide_ctx = guide_ctx.with_subtype(ct);
        }
        let mut axis_ctx = guide_ctx.child("axis");
        if let Some(at) = axis_type {
            axis_ctx = axis_ctx.with_subtype(at);
        }
        let ctx = axis_ctx.child("grid");
        self.stroke_color(&ctx)
    }

    /// Get axis grid opacity
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_grid_opacity(&self, coord_type: Option<&str>, axis_type: Option<&str>) -> f32 {
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(ct) = coord_type {
            guide_ctx = guide_ctx.with_subtype(ct);
        }
        let mut axis_ctx = guide_ctx.child("axis");
        if let Some(at) = axis_type {
            axis_ctx = axis_ctx.with_subtype(at);
        }
        let ctx = axis_ctx.child("grid");
        self.query(&ctx, "opacity")
            .as_pixels(self.base_font_size())
            .unwrap_or(0.5)
    }

    /// Get axis grid width
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_grid_width(&self, coord_type: Option<&str>, axis_type: Option<&str>) -> f32 {
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(ct) = coord_type {
            guide_ctx = guide_ctx.with_subtype(ct);
        }
        let mut axis_ctx = guide_ctx.child("axis");
        if let Some(at) = axis_type {
            axis_ctx = axis_ctx.with_subtype(at);
        }
        let ctx = axis_ctx.child("grid");
        self.stroke_width(&ctx)
    }

    /// Get axis label color
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_label_color(&self, coord_type: Option<&str>, axis_type: Option<&str>) -> String {
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(ct) = coord_type {
            guide_ctx = guide_ctx.with_subtype(ct);
        }
        let mut axis_ctx = guide_ctx.child("axis");
        if let Some(at) = axis_type {
            axis_ctx = axis_ctx.with_subtype(at);
        }
        self.color(&axis_ctx.child("label"))
    }

    /// Get axis title color
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_title_color(&self, coord_type: Option<&str>, axis_type: Option<&str>) -> String {
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(ct) = coord_type {
            guide_ctx = guide_ctx.with_subtype(ct);
        }
        let mut axis_ctx = guide_ctx.child("axis");
        if let Some(at) = axis_type {
            axis_ctx = axis_ctx.with_subtype(at);
        }
        self.color(&axis_ctx.child("title"))
    }

    /// Get axis tick length
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_tick_length(&self, coord_type: Option<&str>, axis_type: Option<&str>) -> f32 {
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(ct) = coord_type {
            guide_ctx = guide_ctx.with_subtype(ct);
        }
        let mut axis_ctx = guide_ctx.child("axis");
        if let Some(at) = axis_type {
            axis_ctx = axis_ctx.with_subtype(at);
        }
        let ctx = axis_ctx.child("tick");
        self.query(&ctx, "size")
            .as_pixels(self.base_font_size())
            .unwrap_or(5.0)
    }

    /// Get axis label font size
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_label_font_size(&self, coord_type: Option<&str>, axis_type: Option<&str>) -> f32 {
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(ct) = coord_type {
            guide_ctx = guide_ctx.with_subtype(ct);
        }
        let mut axis_ctx = guide_ctx.child("axis");
        if let Some(at) = axis_type {
            axis_ctx = axis_ctx.with_subtype(at);
        }
        self.font_size(&axis_ctx.child("label"))
    }

    /// Get axis label font weight
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_label_font_weight(&self, coord_type: Option<&str>, axis_type: Option<&str>) -> f32 {
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(ct) = coord_type {
            guide_ctx = guide_ctx.with_subtype(ct);
        }
        let mut axis_ctx = guide_ctx.child("axis");
        if let Some(at) = axis_type {
            axis_ctx = axis_ctx.with_subtype(at);
        }
        self.font_weight(&axis_ctx.child("label"))
    }

    /// Get axis title font size
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_title_font_size(&self, coord_type: Option<&str>, axis_type: Option<&str>) -> f32 {
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(ct) = coord_type {
            guide_ctx = guide_ctx.with_subtype(ct);
        }
        let mut axis_ctx = guide_ctx.child("axis");
        if let Some(at) = axis_type {
            axis_ctx = axis_ctx.with_subtype(at);
        }
        self.font_size(&axis_ctx.child("title"))
    }

    /// Get axis title font weight
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_title_font_weight(&self, coord_type: Option<&str>, axis_type: Option<&str>) -> f32 {
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(ct) = coord_type {
            guide_ctx = guide_ctx.with_subtype(ct);
        }
        let mut axis_ctx = guide_ctx.child("axis");
        if let Some(at) = axis_type {
            axis_ctx = axis_ctx.with_subtype(at);
        }
        self.font_weight(&axis_ctx.child("title"))
    }

    /// Get axis label font family
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_label_font_family(&self, coord_type: Option<&str>, axis_type: Option<&str>) -> String {
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(ct) = coord_type {
            guide_ctx = guide_ctx.with_subtype(ct);
        }
        let mut axis_ctx = guide_ctx.child("axis");
        if let Some(at) = axis_type {
            axis_ctx = axis_ctx.with_subtype(at);
        }
        self.font_family(&axis_ctx.child("label"))
    }

    /// Get axis title font family
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_title_font_family(&self, coord_type: Option<&str>, axis_type: Option<&str>) -> String {
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(ct) = coord_type {
            guide_ctx = guide_ctx.with_subtype(ct);
        }
        let mut axis_ctx = guide_ctx.child("axis");
        if let Some(at) = axis_type {
            axis_ctx = axis_ctx.with_subtype(at);
        }
        self.font_family(&axis_ctx.child("title"))
    }

    // Access to color palettes, shapes, and dashes

    /// Get categorical color palette
    pub fn categorical_colors(&self) -> Vec<String> {
        // Query categorical colors from CSS
        let context = ThemeContext::new("mark");
        let colors_value = self.query_css(&context, "color-discrete");

        // Parse the result into a list of colors
        match colors_value {
            ThemeValue::String(s) => {
                let colors = Self::parse_css_list(&s);
                if !colors.is_empty() {
                    return colors;
                }
            }
            ThemeValue::List(values) => {
                let mut colors = Vec::new();
                for val in values {
                    match val {
                        ThemeValue::String(s) => {
                            colors.push(s);
                        }
                        ThemeValue::Color(rgba) => {
                            let hex =
                                format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue);
                            colors.push(hex);
                        }
                        _ => {}
                    }
                }
                if !colors.is_empty() {
                    return colors;
                }
            }
            _ => {}
        }

        // Fallback to Okabe-Ito colorblind-safe palette
        vec![
            "#0072B2".to_string(), // Blue
            "#E69F00".to_string(), // Orange
            "#009E73".to_string(), // Bluish Green
            "#F0E442".to_string(), // Yellow
            "#D55E00".to_string(), // Vermillion
            "#56B4E9".to_string(), // Sky Blue
            "#CC79A7".to_string(), // Reddish Purple
            "#999999".to_string(), // Grey
        ]
    }

    /// Get shape names for shape channel
    pub fn shape_names(&self) -> Vec<String> {
        // Query shape names from CSS
        let context = ThemeContext::new("mark");
        let shapes_value = self.query_css(&context, "shape-discrete");

        // Parse the result into a list of shapes
        match shapes_value {
            ThemeValue::String(s) => {
                let shapes = Self::parse_css_list(&s);
                if !shapes.is_empty() {
                    return shapes;
                }
            }
            ThemeValue::List(values) => {
                let mut shapes = Vec::new();
                for val in values {
                    if let ThemeValue::String(s) = val {
                        shapes.push(s);
                    }
                }
                if !shapes.is_empty() {
                    return shapes;
                }
            }
            _ => {}
        }

        // Fallback to default shapes
        vec![
            "circle".to_string(),
            "cross".to_string(),
            "diamond".to_string(),
            "square".to_string(),
            "star".to_string(),
            "triangle-up".to_string(),
            "wye".to_string(),
            "cushion".to_string(),
        ]
    }

    /// Get dash pattern names
    pub fn dash_names(&self) -> Vec<String> {
        // Query dash patterns from CSS
        let context = ThemeContext::new("mark");
        let dashes_value = self.query_css(&context, "stroke-dash-discrete");

        // Parse the result into a list of dash patterns
        match dashes_value {
            ThemeValue::String(s) => {
                let dashes = Self::parse_css_list(&s);
                if !dashes.is_empty() {
                    return dashes;
                }
            }
            ThemeValue::List(values) => {
                let mut dashes = Vec::new();
                for val in values {
                    if let ThemeValue::String(s) = val {
                        dashes.push(s);
                    }
                }
                if !dashes.is_empty() {
                    return dashes;
                }
            }
            _ => {}
        }

        // Fallback to default dash patterns
        vec![
            "solid".to_string(),
            "dashed".to_string(),
            "dotted".to_string(),
            "long-dash".to_string(),
            "dash-dot".to_string(),
            "long-short".to_string(),
            "even-short".to_string(),
            "double-dash".to_string(),
        ]
    }

    /// Get default shape range for ordinal scales
    pub fn get_shape_range(&self, domain_cardinality: Option<usize>) -> crate::scales::ScaleRange {
        // Use the new get_range_for_channel with a default mark type
        self.get_range_for_channel(
            "symbol",
            "shape",
            avenger_scales::scales::RangeKind::Discrete,
            domain_cardinality,
        )
    }

    /// Get default dash pattern range for ordinal scales
    pub fn get_dash_range(&self, _domain_cardinality: Option<usize>) -> crate::scales::ScaleRange {
        use crate::scales::ScaleRange;
        use datafusion_common::ScalarValue;

        let patterns = self.dash_names();
        use crate::serialization::SerializableScalar;
        let scalars: Vec<SerializableScalar> = patterns
            .into_iter()
            .map(|p| SerializableScalar::new(ScalarValue::Utf8(Some(p))))
            .collect();
        ScaleRange::Discrete(scalars)
    }

    /// Get range for a specific channel based on mark type and range kind
    pub fn get_range_for_channel(
        &self,
        mark_type: &str,
        channel: &str,
        range_kind: avenger_scales::scales::RangeKind,
        domain_cardinality: Option<usize>,
    ) -> crate::scales::ScaleRange {
        use avenger_scales::scales::RangeKind;

        // Build property name based on channel and range kind
        // Convert underscores to hyphens for CSS-friendly property names
        // e.g., "glow_color" -> "glow-color-continuous"
        let css_channel = channel.replace('_', "-");
        let property = format!(
            "{}-{}",
            css_channel,
            match range_kind {
                RangeKind::Discrete => "discrete",
                RangeKind::Continuous => "continuous",
            }
        );

        // Try mark-specific first, then general mark
        let contexts = vec![
            ThemeContext::new("mark").with_mark(mark_type),
            ThemeContext::new("mark"),
        ];

        for context in contexts {
            let range_value = self.query_css(&context, &property);

            // Parse the range value into appropriate ScaleRange
            match range_value {
                ThemeValue::String(s) => {
                    // Parse comma-separated list
                    let values = Self::parse_css_list(&s);
                    if !values.is_empty() {
                        return self.create_scale_range(
                            &values,
                            channel,
                            range_kind,
                            domain_cardinality,
                        );
                    }
                }
                ThemeValue::List(values) => {
                    // Handle pre-parsed list of values
                    let mut parsed_values = Vec::new();
                    for val in values {
                        match val {
                            ThemeValue::String(s) => {
                                parsed_values.push(s.clone());
                            }
                            ThemeValue::Color(rgba) => {
                                let hex =
                                    format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue);
                                parsed_values.push(hex);
                            }
                            ThemeValue::Number(n) => {
                                parsed_values.push(n.to_string());
                            }
                            _ => {}
                        }
                    }
                    if !parsed_values.is_empty() {
                        return self.create_scale_range(
                            &parsed_values,
                            channel,
                            range_kind,
                            domain_cardinality,
                        );
                    }
                }
                ThemeValue::Variable(var_name) => {
                    // Try to resolve variable manually
                    if let Some(resolved) = self.variables.get(&var_name) {
                        match resolved {
                            ThemeValue::String(s) => {
                                let values = Self::parse_css_list(&s);
                                if !values.is_empty() {
                                    return self.create_scale_range(
                                        &values,
                                        channel,
                                        range_kind,
                                        domain_cardinality,
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        // Fall back to defaults based on channel and range kind
        self.default_range_for_channel(channel, range_kind, domain_cardinality)
    }

    /// Get mark default value for a channel
    ///
    /// Returns the default value for a specific mark type and channel.
    pub fn mark_default(
        &self,
        mark_type: &str,
        channel: &str,
    ) -> Option<ScalarValue> {
        // Query CSS theme for mark defaults
        let context = ThemeContext::new("mark").with_mark(mark_type);

        // Map channel to CSS property
        let css_property = match channel {
            "fill" => "fill",
            "stroke" => "stroke",
            "stroke_width" => "stroke-width",
            "size" => "size",
            _ => return None,
        };

        let theme_value = self.query_css(&context, css_property);

        match theme_value {
            ThemeValue::String(s) => Some(ScalarValue::Utf8(Some(s))),
            ThemeValue::Number(n) => Some(ScalarValue::Float32(Some(n as f32))),
            ThemeValue::Length(n, _) => Some(ScalarValue::Float32(Some(n as f32))),
            ThemeValue::Color(rgba) => {
                let hex = format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue);
                Some(ScalarValue::Utf8(Some(hex)))
            }
            _ => None,
        }
    }

    /// Parse a CSS list value into individual string values
    /// Handles formats like: "#E69F00", "#56B4E9", "#009E73"
    fn parse_css_list(value: &str) -> Vec<String> {
        value
            .split(',')
            .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Create a ScaleRange from parsed values
    fn create_scale_range(
        &self,
        values: &[String],
        _channel: &str,
        range_kind: avenger_scales::scales::RangeKind,
        domain_cardinality: Option<usize>,
    ) -> crate::scales::ScaleRange {
        use crate::scales::ScaleRange;
        use avenger_scales::scales::RangeKind;
        use datafusion::prelude::lit;
        use palette::Srgba;

        match range_kind {
            RangeKind::Discrete => {
                // For discrete ranges, take the requested number of values
                use crate::serialization::SerializableScalar;
                let scalars: Vec<SerializableScalar> = values
                    .iter()
                    .take(domain_cardinality.unwrap_or(values.len()))
                    .map(|v| {
                        // Check if it's a number or string
                        let scalar = if let Ok(num) = v.parse::<f64>() {
                            ScalarValue::Float32(Some(num as f32))
                        } else {
                            ScalarValue::Utf8(Some(v.clone()))
                        };
                        SerializableScalar::new(scalar)
                    })
                    .collect();
                ScaleRange::Discrete(scalars)
            }
            RangeKind::Continuous => {
                // First, try to parse as numbers - if successful, it's a numeric range
                // Only attempt color parsing if numeric parsing fails
                let first_is_number = values.first()
                    .and_then(|v| v.parse::<f64>().ok())
                    .is_some();

                if first_is_number {
                    // Numeric continuous range - expect 2 values (min, max)
                    if values.len() >= 2 {
                        let min = values[0].parse::<f64>().unwrap_or(0.0);
                        let max = values[1].parse::<f64>().unwrap_or(1.0);
                        ScaleRange::new_interval(lit(min), lit(max))
                    } else if values.len() == 1 {
                        // Single value, use it as max with 0 as min
                        let max = values[0].parse::<f64>().unwrap_or(1.0);
                        ScaleRange::new_interval(lit(0.0), lit(max))
                    } else {
                        // Default range
                        ScaleRange::new_interval(lit(0.0), lit(1.0))
                    }
                } else {
                    // Try to detect if values are colors by attempting to parse them
                    // This works for any channel, not just hardcoded ones
                    let colors: Vec<Srgba> = values
                        .iter()
                        .filter_map(|v| {
                            // Try to parse as color using avenger-scales color parser
                            // This handles hex colors, rgb(), hsl(), and named colors
                            crate::utils::parse_color_string(v).and_then(|cog| {
                                match cog {
                                    avenger_common::types::ColorOrGradient::Color(rgba) => {
                                        Some(Srgba::new(
                                            rgba[0],
                                            rgba[1],
                                            rgba[2],
                                            rgba[3],
                                        ))
                                    }
                                    _ => None,
                                }
                            })
                        })
                        .collect();

                    if !colors.is_empty() {
                        // Successfully parsed as colors, use color range
                        ScaleRange::new_color(colors)
                    } else {
                        // Couldn't parse as numbers or colors, use default numeric range
                        ScaleRange::new_interval(lit(0.0), lit(1.0))
                    }
                }
            }
        }
    }

    /// Default ranges for channels when not specified in CSS
    fn default_range_for_channel(
        &self,
        channel: &str,
        range_kind: avenger_scales::scales::RangeKind,
        domain_cardinality: Option<usize>,
    ) -> crate::scales::ScaleRange {
        use crate::scales::ScaleRange;
        use avenger_scales::scales::RangeKind;
        use datafusion::prelude::lit;

        match (channel, range_kind) {
            // Color channels
            ("fill" | "stroke" | "color", RangeKind::Discrete) => {
                // Use categorical_colors which queries from CSS or falls back to Okabe-Ito
                let colors = self.categorical_colors();
                use crate::serialization::SerializableScalar;
                let scalars: Vec<SerializableScalar> = colors
                    .iter()
                    .take(domain_cardinality.unwrap_or(colors.len()))
                    .map(|c| SerializableScalar::new(ScalarValue::Utf8(Some(c.clone()))))
                    .collect();
                ScaleRange::Discrete(scalars)
            }
            ("fill" | "stroke" | "color", RangeKind::Continuous) => {
                // Viridis-like gradient
                use palette::Srgba;
                let colors = vec![
                    Srgba::new(0.267, 0.004, 0.329, 1.0), // Dark purple
                    Srgba::new(0.193, 0.408, 0.556, 1.0), // Blue
                    Srgba::new(0.208, 0.718, 0.473, 1.0), // Green
                    Srgba::new(0.993, 0.906, 0.144, 1.0), // Yellow
                ];
                ScaleRange::new_color(colors)
            }

            // Size channels
            ("size", RangeKind::Discrete) => {
                let sizes = vec![20.0, 40.0, 60.0, 80.0, 100.0];
                use crate::serialization::SerializableScalar;
                let scalars: Vec<SerializableScalar> = sizes
                    .iter()
                    .take(domain_cardinality.unwrap_or(sizes.len()))
                    .map(|s| SerializableScalar::new(ScalarValue::Float32(Some(*s as f32))))
                    .collect();
                ScaleRange::Discrete(scalars)
            }
            ("size", RangeKind::Continuous) => ScaleRange::new_interval(lit(10.0), lit(200.0)),

            // Opacity channels
            ("opacity", RangeKind::Discrete) => {
                let opacities = vec![0.3, 0.5, 0.7, 0.9, 1.0];
                use crate::serialization::SerializableScalar;
                let scalars: Vec<SerializableScalar> = opacities
                    .iter()
                    .take(domain_cardinality.unwrap_or(opacities.len()))
                    .map(|o| SerializableScalar::new(ScalarValue::Float32(Some(*o as f32))))
                    .collect();
                ScaleRange::Discrete(scalars)
            }
            ("opacity", RangeKind::Continuous) => ScaleRange::new_interval(lit(0.2), lit(1.0)),

            // Stroke width channels
            ("stroke_width", RangeKind::Discrete) => {
                let widths = vec![1.0, 2.0, 3.0, 4.0, 5.0];
                use crate::serialization::SerializableScalar;
                let scalars: Vec<SerializableScalar> = widths
                    .iter()
                    .take(domain_cardinality.unwrap_or(widths.len()))
                    .map(|w| SerializableScalar::new(ScalarValue::Float32(Some(*w as f32))))
                    .collect();
                ScaleRange::Discrete(scalars)
            }
            ("stroke_width", RangeKind::Continuous) => ScaleRange::new_interval(lit(0.5), lit(5.0)),

            // Shape channel (always discrete)
            ("shape", _) => {
                let shapes = vec!["circle", "square", "triangle", "diamond", "cross"];
                use crate::serialization::SerializableScalar;
                let scalars: Vec<SerializableScalar> = shapes
                    .iter()
                    .take(domain_cardinality.unwrap_or(shapes.len()))
                    .map(|s| SerializableScalar::new(ScalarValue::Utf8(Some(s.to_string()))))
                    .collect();
                ScaleRange::Discrete(scalars)
            }

            // Default
            _ => match range_kind {
                RangeKind::Discrete => {
                    use crate::serialization::SerializableScalar;
                    ScaleRange::Discrete(vec![SerializableScalar::new(ScalarValue::Float32(Some(
                        1.0,
                    )))])
                }
                RangeKind::Continuous => ScaleRange::new_interval(lit(0.0), lit(1.0)),
            },
        }
    }
}
