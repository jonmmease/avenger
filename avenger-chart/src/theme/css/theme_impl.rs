//! Implementation of the Theme trait for CSS-based themes

use super::Theme as CssTheme;
use crate::theme::{LengthUnit, Theme as ThemeTrait, ThemeContext, ThemeProperty, ThemeValue};
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

    /// Convert a ThemeProperty to a CSS property name
    fn property_to_css(&self, property: &ThemeProperty) -> &'static str {
        match property {
            ThemeProperty::FontFamily => "font-family",
            ThemeProperty::FontSize => "font-size",
            ThemeProperty::FontWeight => "font-weight",
            ThemeProperty::Color => "color",
            ThemeProperty::BackgroundColor => "background-color",
            ThemeProperty::FillColor => "fill",
            ThemeProperty::StrokeColor => "stroke",
            ThemeProperty::GridColor => "grid-color",
            ThemeProperty::StrokeWidth => "stroke-width",
            ThemeProperty::Size => "size",
            ThemeProperty::Padding => "padding",
            ThemeProperty::Spacing => "spacing",
            ThemeProperty::GridOpacity => "grid-opacity",
            ThemeProperty::Opacity => "opacity",
            ThemeProperty::CornerRadius => "corner-radius",
            ThemeProperty::LabelAngle => "label-angle",
            ThemeProperty::Custom(name) => {
                // For custom properties, we'll need to leak the string to get a &'static str
                // This is not ideal but works for now. In a real implementation,
                // we might want to return a Cow<'static, str> or String instead.
                Box::leak(name.clone().into_boxed_str())
            }
        }
    }
}

impl ThemeTrait for CssTheme {
    fn query(&self, context: &ThemeContext, property: &ThemeProperty) -> ThemeValue {
        // Context-aware property mapping
        let css_property =
            if context.element_type == "axis" && context.classes.contains(&"grid".to_string()) {
                match property {
                    ThemeProperty::GridColor => "stroke", // Grid lines use stroke, not grid-color
                    ThemeProperty::GridOpacity => "opacity",
                    _ => self.property_to_css(property),
                }
            } else {
                self.property_to_css(property)
            };

        let value = self.query_css(context, css_property);

        // Convert rem/em units to pixels for font-size
        if matches!(property, ThemeProperty::FontSize) {
            if let ThemeValue::Length(size, LengthUnit::Rem) = value {
                // Convert rem to pixels (rem is relative to base font size)
                return ThemeValue::Float((size * self.base_font_size as f64) as f32);
            } else if let ThemeValue::Length(size, LengthUnit::Em) = value {
                // For font-size, em is relative to parent's font size
                // For now, treat it like rem (this could be improved with parent context)
                return ThemeValue::Float((size * self.base_font_size as f64) as f32);
            }
        }

        value
    }

    fn clone_box(&self) -> Box<dyn ThemeTrait> {
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
                ThemeValue::String(s) | ThemeValue::Keyword(s) => {
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
                            ThemeValue::String(s) | ThemeValue::Keyword(s) => {
                                parsed_values.push(s.clone());
                            }
                            ThemeValue::Color(rgba) => {
                                let hex =
                                    format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue);
                                parsed_values.push(hex);
                            }
                            ThemeValue::Double(n) => {
                                parsed_values.push(n.to_string());
                            }
                            ThemeValue::Float(f) => {
                                parsed_values.push(f.to_string());
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
                            ThemeValue::String(s) | ThemeValue::Keyword(s) => {
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
        // Return default categorical colors
        vec![
            "#4c78a8".to_string(),
            "#f58518".to_string(),
            "#54a24b".to_string(),
            "#e45756".to_string(),
            "#72b7b2".to_string(),
            "#eeca3b".to_string(),
            "#b279a2".to_string(),
            "#ff9da6".to_string(),
            "#9d755d".to_string(),
            "#bab0ac".to_string(),
        ]
    }

    fn shape_names(&self) -> Vec<String> {
        vec![
            "circle".to_string(),
            "square".to_string(),
            "triangle".to_string(),
            "diamond".to_string(),
            "cross".to_string(),
        ]
    }

    fn dash_names(&self) -> Vec<String> {
        vec![
            "solid".to_string(),
            "dashed".to_string(),
            "dotted".to_string(),
            "dash-dot".to_string(),
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
                ThemeValue::String(s) | ThemeValue::Keyword(s) => {
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
                ThemeValue::String(s) | ThemeValue::Keyword(s) => {
                    mark_defaults.insert("stroke".to_string(), ScalarValue::Utf8(Some(s)));
                }
                _ => {}
            }

            if let Some(width) = self.query_css(&context, "stroke-width").as_float() {
                mark_defaults.insert(
                    "stroke_width".to_string(),
                    ScalarValue::Float32(Some(width)),
                );
            }

            if let Some(size) = self.query_css(&context, "size").as_float() {
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
            ThemeValue::String(s) | ThemeValue::Keyword(s) => Some(ScalarValue::Utf8(Some(s))),
            ThemeValue::Double(n) => Some(ScalarValue::Float32(Some(n as f32))),
            ThemeValue::Float(f) => Some(ScalarValue::Float32(Some(f))),
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
        _channel: &str,
        range_kind: avenger_scales::scales::RangeKind,
        domain_cardinality: Option<usize>,
    ) -> crate::scales::ScaleRange {
        use crate::scales::ScaleRange;
        use avenger_scales::scales::RangeKind;
        use datafusion::prelude::lit;

        match range_kind {
            RangeKind::Discrete => {
                // For discrete ranges, take the requested number of values
                let scalars: Vec<ScalarValue> = values
                    .iter()
                    .take(domain_cardinality.unwrap_or(values.len()))
                    .map(|v| {
                        // Check if it's a number or string
                        if let Ok(num) = v.parse::<f64>() {
                            ScalarValue::Float32(Some(num as f32))
                        } else {
                            ScalarValue::Utf8(Some(v.clone()))
                        }
                    })
                    .collect();
                ScaleRange::Discrete(scalars)
            }
            RangeKind::Continuous => {
                // For continuous ranges, expect 2 values (min, max)
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
                let colors = vec![
                    "#4c78a8", "#f58518", "#54a24b", "#e45756", "#72b7b2", "#eeca3b", "#b279a2",
                    "#ff9da6", "#9d755d", "#bab0ac",
                ];
                let scalars: Vec<ScalarValue> = colors
                    .iter()
                    .take(domain_cardinality.unwrap_or(colors.len()))
                    .map(|c| ScalarValue::Utf8(Some(c.to_string())))
                    .collect();
                ScaleRange::Discrete(scalars)
            }
            ("fill" | "stroke" | "color", RangeKind::Continuous) => {
                // Blue gradient
                ScaleRange::Discrete(vec![
                    ScalarValue::Utf8(Some("#f7fbff".to_string())),
                    ScalarValue::Utf8(Some("#08306b".to_string())),
                ])
            }

            // Size channels
            ("size", RangeKind::Discrete) => {
                let sizes = vec![20.0, 40.0, 60.0, 80.0, 100.0];
                let scalars: Vec<ScalarValue> = sizes
                    .iter()
                    .take(domain_cardinality.unwrap_or(sizes.len()))
                    .map(|s| ScalarValue::Float32(Some(*s as f32)))
                    .collect();
                ScaleRange::Discrete(scalars)
            }
            ("size", RangeKind::Continuous) => ScaleRange::new_interval(lit(10.0), lit(200.0)),

            // Opacity channels
            ("opacity", RangeKind::Discrete) => {
                let opacities = vec![0.3, 0.5, 0.7, 0.9, 1.0];
                let scalars: Vec<ScalarValue> = opacities
                    .iter()
                    .take(domain_cardinality.unwrap_or(opacities.len()))
                    .map(|o| ScalarValue::Float32(Some(*o as f32)))
                    .collect();
                ScaleRange::Discrete(scalars)
            }
            ("opacity", RangeKind::Continuous) => ScaleRange::new_interval(lit(0.2), lit(1.0)),

            // Stroke width channels
            ("stroke_width", RangeKind::Discrete) => {
                let widths = vec![1.0, 2.0, 3.0, 4.0, 5.0];
                let scalars: Vec<ScalarValue> = widths
                    .iter()
                    .take(domain_cardinality.unwrap_or(widths.len()))
                    .map(|w| ScalarValue::Float32(Some(*w as f32)))
                    .collect();
                ScaleRange::Discrete(scalars)
            }
            ("stroke_width", RangeKind::Continuous) => ScaleRange::new_interval(lit(0.5), lit(5.0)),

            // Shape channel (always discrete)
            ("shape", _) => {
                let shapes = vec!["circle", "square", "triangle", "diamond", "cross"];
                let scalars: Vec<ScalarValue> = shapes
                    .iter()
                    .take(domain_cardinality.unwrap_or(shapes.len()))
                    .map(|s| ScalarValue::Utf8(Some(s.to_string())))
                    .collect();
                ScaleRange::Discrete(scalars)
            }

            // Default
            _ => match range_kind {
                RangeKind::Discrete => ScaleRange::Discrete(vec![ScalarValue::Float32(Some(1.0))]),
                RangeKind::Continuous => ScaleRange::new_interval(lit(0.0), lit(1.0)),
            },
        }
    }
}
