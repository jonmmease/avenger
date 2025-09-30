//! Implementation of the Theme trait for CSS-based themes

use super::CssTheme;
use crate::theme::{Theme, ThemeContext, ThemeValue};
use datafusion_common::ScalarValue;
use indexmap::IndexMap;

impl CssTheme {
    /// Parse a CSS list value into individual string values
    /// Handles formats like: "#E69F00", "#56B4E9", "#009E73"
    fn parse_css_list(value: &str) -> Vec<String> {
        value
            .split(',')
            .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

#[typetag::serde]
impl Theme for CssTheme {
    fn query(&self, context: &ThemeContext, property: &str) -> ThemeValue {
        self.query_css(context, property)
    }

    fn base_font_size(&self) -> f32 {
        self.base_font_size
    }

    fn clone_box(&self) -> Box<dyn Theme> {
        Box::new(self.clone())
    }

    fn get_range_for_channel(
        &self,
        mark_type: &str,
        channel: &str,
        range_kind: avenger_scales::scales::RangeKind,
        domain_cardinality: Option<usize>,
    ) -> crate::scales::ScaleRange {
        use avenger_scales::scales::RangeKind;

        // Build property name based on channel and range kind
        let property = format!(
            "{}-{}",
            channel,
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

    fn get_shape_range(&self, domain_cardinality: Option<usize>) -> crate::scales::ScaleRange {
        // Use the new get_range_for_channel with a default mark type
        self.get_range_for_channel(
            "symbol",
            "shape",
            avenger_scales::scales::RangeKind::Discrete,
            domain_cardinality,
        )
    }

    fn categorical_colors(&self) -> Vec<String> {
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

    fn shape_names(&self) -> Vec<String> {
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

    fn dash_names(&self) -> Vec<String> {
        // Query dash patterns from CSS
        let context = ThemeContext::new("mark");
        let dashes_value = self.query_css(&context, "stroke_dash-discrete");

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

    fn mark_defaults_map(&self) -> IndexMap<String, IndexMap<String, ScalarValue>> {
        let mut defaults = IndexMap::new();

        // Query mark defaults from CSS
        for mark_type in &["symbol", "rect", "line", "area", "text", "arc"] {
            let mut mark_defaults = IndexMap::new();
            let context = ThemeContext::new("mark").with_mark(*mark_type);

            // Handle fill - can be Color or Keyword (named color)
            match self.query_css(&context, "fill") {
                ThemeValue::Color(rgba) => {
                    let hex = format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue);
                    mark_defaults.insert("fill".to_string(), ScalarValue::Utf8(Some(hex)));
                }
                ThemeValue::String(s) => {
                    mark_defaults.insert("fill".to_string(), ScalarValue::Utf8(Some(s)));
                }
                _ => {}
            }

            // Handle stroke - can be Color or Keyword (named color)
            match self.query_css(&context, "stroke") {
                ThemeValue::Color(rgba) => {
                    let hex = format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue);
                    mark_defaults.insert("stroke".to_string(), ScalarValue::Utf8(Some(hex)));
                }
                ThemeValue::String(s) => {
                    mark_defaults.insert("stroke".to_string(), ScalarValue::Utf8(Some(s)));
                }
                _ => {}
            }

            if let Some(width) = self
                .query_css(&context, "stroke-width")
                .as_pixels(self.base_font_size)
            {
                mark_defaults.insert(
                    "stroke_width".to_string(),
                    ScalarValue::Float32(Some(width)),
                );
            }

            if let Some(size) = self
                .query_css(&context, "size")
                .as_pixels(self.base_font_size)
            {
                mark_defaults.insert("size".to_string(), ScalarValue::Float32(Some(size)));
            }

            if !mark_defaults.is_empty() {
                defaults.insert(mark_type.to_string(), mark_defaults);
            }
        }

        defaults
    }

    fn mark_default(&self, mark_type: &str, channel: &str) -> Option<ScalarValue> {
        let context = ThemeContext::new("mark").with_mark(mark_type);

        // Map channel to CSS property
        let css_property = match channel {
            "fill" => "fill",
            "stroke" => "stroke",
            "stroke_width" => "stroke-width",
            "size" => "size",
            "font" => "font-family",
            "font_size" => "font-size",
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

    fn mark_default_with_computed_fonts(
        &self,
        mark_type: &str,
        channel: &str,
        computed_font_size: f32,
        base_font_family: &str,
    ) -> Option<ScalarValue> {
        // Handle text marks with computed fonts
        if mark_type == "text" {
            match channel {
                "font_size" => return Some(ScalarValue::Float32(Some(computed_font_size))),
                "font" => return Some(ScalarValue::Utf8(Some(base_font_family.to_string()))),
                _ => {}
            }
        }

        // For all other cases, use the regular mark_default
        self.mark_default(mark_type, channel)
    }
}

impl CssTheme {
    /// Create a ScaleRange from parsed values
    fn create_scale_range(
        &self,
        values: &[String],
        channel: &str,
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
                // Check if this is a color channel
                if channel == "fill" || channel == "stroke" || channel == "color" {
                    // For continuous color ranges, parse as Srgba colors
                    let colors: Vec<Srgba> = values
                        .iter()
                        .filter_map(|v| {
                            // Parse hex color to Srgba
                            if v.starts_with('#') && v.len() >= 7 {
                                let r = u8::from_str_radix(&v[1..3], 16).ok()? as f32 / 255.0;
                                let g = u8::from_str_radix(&v[3..5], 16).ok()? as f32 / 255.0;
                                let b = u8::from_str_radix(&v[5..7], 16).ok()? as f32 / 255.0;
                                Some(Srgba::new(r, g, b, 1.0))
                            } else {
                                None
                            }
                        })
                        .collect();

                    if !colors.is_empty() {
                        ScaleRange::new_color(colors)
                    } else {
                        // Fallback to viridis-like palette
                        let colors = vec![
                            Srgba::new(0.267, 0.004, 0.329, 1.0), // Dark purple
                            Srgba::new(0.193, 0.408, 0.556, 1.0), // Blue
                            Srgba::new(0.208, 0.718, 0.473, 1.0), // Green
                            Srgba::new(0.993, 0.906, 0.144, 1.0), // Yellow
                        ];
                        ScaleRange::new_color(colors)
                    }
                } else {
                    // For numeric continuous ranges, expect 2 values (min, max)
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
