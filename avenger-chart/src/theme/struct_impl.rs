//! Implementation of Theme trait for the struct-based theme

use super::{StructTheme, Theme, ThemeContext, ThemeProperty, ThemeValue};

impl Theme for StructTheme {
    fn query(&self, context: &ThemeContext, property: &ThemeProperty) -> ThemeValue {
        match (&context.element_type[..], property) {
            // Title queries
            ("title", ThemeProperty::FontFamily) => ThemeValue::String(
                self.title
                    .title_font_family
                    .as_deref()
                    .unwrap_or(&self.base_font_family)
                    .to_string(),
            ),
            ("title", ThemeProperty::FontSize) => {
                ThemeValue::Float((self.base_font_size * self.font_scale.title_scale).round())
            }
            ("title", ThemeProperty::FontWeight) => ThemeValue::Float(self.title.title_font_weight),
            ("title", ThemeProperty::Color) => ThemeValue::String(self.title.title_color.clone()),

            // Subtitle queries
            ("subtitle", ThemeProperty::FontFamily) => ThemeValue::String(
                self.title
                    .subtitle_font_family
                    .as_deref()
                    .unwrap_or(&self.base_font_family)
                    .to_string(),
            ),
            ("subtitle", ThemeProperty::FontSize) => {
                ThemeValue::Float((self.base_font_size * self.font_scale.subtitle_scale).round())
            }
            ("subtitle", ThemeProperty::FontWeight) => {
                ThemeValue::Float(self.title.subtitle_font_weight)
            }
            ("subtitle", ThemeProperty::Color) => {
                ThemeValue::String(self.title.subtitle_color.clone())
            }

            // Axis queries
            ("axis", property) => self.query_axis(context, property),

            // Legend queries
            ("legend", property) => self.query_legend(context, property),

            // Mark queries
            ("mark", property) => self.query_mark(context, property),

            // Canvas queries
            ("canvas", ThemeProperty::BackgroundColor) => {
                match &self.background.canvas_background {
                    Some(color) => ThemeValue::String(color.clone()),
                    None => ThemeValue::None,
                }
            }

            // Fallback to base values
            (_, ThemeProperty::FontFamily) => ThemeValue::String(self.base_font_family.clone()),
            (_, ThemeProperty::FontSize) => ThemeValue::Float(self.base_font_size),

            // Default values
            _ => ThemeValue::None,
        }
    }

    fn clone_box(&self) -> Box<dyn Theme> {
        Box::new(self.clone())
    }

    fn get_color_range(
        &self,
        scale_type: &str,
        domain_cardinality: Option<usize>,
    ) -> crate::scales::ScaleRange {
        // Call the implementation method
        self.get_color_range_impl(scale_type, domain_cardinality)
    }

    fn get_shape_range(&self, domain_cardinality: Option<usize>) -> crate::scales::ScaleRange {
        // Call the implementation method
        self.get_shape_range_impl(domain_cardinality)
    }

    fn categorical_colors(&self) -> Vec<String> {
        self.colors.categorical.clone()
    }

    fn shape_names(&self) -> Vec<String> {
        self.shapes.shapes.iter().map(|s| s.to_string()).collect()
    }

    fn dash_names(&self) -> Vec<String> {
        self.dashes.patterns.iter().map(|s| s.to_string()).collect()
    }

    fn mark_defaults_map(
        &self,
    ) -> indexmap::IndexMap<String, indexmap::IndexMap<String, datafusion_common::ScalarValue>>
    {
        self.mark_defaults.defaults.clone()
    }

    fn mark_default(
        &self,
        mark_type: &str,
        channel: &str,
    ) -> Option<datafusion_common::ScalarValue> {
        self.mark_defaults.get(mark_type, channel).cloned()
    }

    fn mark_default_with_computed_fonts(
        &self,
        mark_type: &str,
        channel: &str,
        computed_font_size: f32,
        base_font_family: &str,
    ) -> Option<datafusion_common::ScalarValue> {
        // First check if it's a text mark needing computed fonts
        use datafusion_common::ScalarValue;
        if mark_type == "text" {
            match channel {
                "font_size" => return Some(ScalarValue::Float32(Some(computed_font_size))),
                "font" => {
                    // First check if there's an override in mark defaults
                    if let Some(font) = self.mark_defaults.get(mark_type, channel) {
                        return Some(font.clone());
                    }
                    // Otherwise use the base font family
                    return Some(ScalarValue::Utf8(Some(base_font_family.to_string())));
                }
                _ => {}
            }
        }

        // For all other cases, use the regular mark_default
        self.mark_default(mark_type, channel)
    }
}

impl StructTheme {
    /// Query axis-specific properties
    fn query_axis(&self, context: &ThemeContext, property: &ThemeProperty) -> ThemeValue {
        // Check if it's asking about title, label, or tick
        let is_title = context.classes.contains(&"title".to_string());
        let is_label = context.classes.contains(&"label".to_string());
        let is_tick = context.classes.contains(&"tick".to_string());

        match property {
            ThemeProperty::FontFamily => {
                if is_title {
                    ThemeValue::String(
                        self.axis
                            .title_font_family
                            .as_deref()
                            .unwrap_or(&self.base_font_family)
                            .to_string(),
                    )
                } else if is_label {
                    ThemeValue::String(
                        self.axis
                            .label_font_family
                            .as_deref()
                            .unwrap_or(&self.base_font_family)
                            .to_string(),
                    )
                } else {
                    ThemeValue::String(self.base_font_family.clone())
                }
            }
            ThemeProperty::FontSize => {
                if is_title {
                    ThemeValue::Float(
                        (self.base_font_size * self.font_scale.axis_title_scale).round(),
                    )
                } else if is_label {
                    ThemeValue::Float(
                        (self.base_font_size * self.font_scale.axis_label_scale).round(),
                    )
                } else {
                    ThemeValue::Float(self.base_font_size)
                }
            }
            ThemeProperty::FontWeight => {
                if is_title {
                    ThemeValue::Float(self.axis.title_font_weight)
                } else if is_label {
                    ThemeValue::Float(self.axis.label_font_weight)
                } else {
                    ThemeValue::Float(400.0)
                }
            }
            ThemeProperty::Color => {
                if is_title {
                    ThemeValue::String(self.axis.title_color.clone())
                } else if is_label {
                    ThemeValue::String(self.axis.label_color.clone())
                } else if is_tick {
                    ThemeValue::String(self.axis.tick_color.clone())
                } else {
                    ThemeValue::String(self.axis.domain_color.clone())
                }
            }
            ThemeProperty::GridColor => ThemeValue::String(self.axis.grid_color.clone()),
            ThemeProperty::GridOpacity => ThemeValue::Float(self.axis.grid_opacity),
            ThemeProperty::StrokeWidth => {
                if context.classes.contains(&"grid".to_string()) {
                    ThemeValue::Float(self.axis.grid_width)
                } else if context.classes.contains(&"domain".to_string()) {
                    ThemeValue::Float(self.axis.domain_width)
                } else {
                    ThemeValue::Float(1.0)
                }
            }
            ThemeProperty::LabelAngle => ThemeValue::Float(self.axis.label_angle),
            _ => ThemeValue::None,
        }
    }

    /// Query legend-specific properties
    fn query_legend(&self, context: &ThemeContext, property: &ThemeProperty) -> ThemeValue {
        // Check if it's asking about title, label, or tick
        let is_title = context.classes.contains(&"title".to_string());
        let is_label = context.classes.contains(&"label".to_string());
        let is_tick = context.classes.contains(&"tick".to_string());

        match property {
            ThemeProperty::FontFamily => {
                if is_title {
                    ThemeValue::String(
                        self.legend
                            .title_font_family
                            .as_deref()
                            .unwrap_or(&self.base_font_family)
                            .to_string(),
                    )
                } else if is_label {
                    ThemeValue::String(
                        self.legend
                            .label_font_family
                            .as_deref()
                            .unwrap_or(&self.base_font_family)
                            .to_string(),
                    )
                } else if is_tick {
                    ThemeValue::String(
                        self.legend
                            .tick_font_family
                            .as_deref()
                            .unwrap_or(&self.base_font_family)
                            .to_string(),
                    )
                } else {
                    ThemeValue::String(self.base_font_family.clone())
                }
            }
            ThemeProperty::FontSize => {
                if is_title {
                    ThemeValue::Float(
                        (self.base_font_size * self.font_scale.legend_title_scale).round(),
                    )
                } else if is_label {
                    ThemeValue::Float(
                        (self.base_font_size * self.font_scale.legend_label_scale).round(),
                    )
                } else if is_tick {
                    ThemeValue::Float(
                        (self.base_font_size * self.font_scale.legend_tick_scale).round(),
                    )
                } else {
                    ThemeValue::Float(self.base_font_size)
                }
            }
            ThemeProperty::FontWeight => {
                if is_title {
                    ThemeValue::Float(self.legend.title_font_weight)
                } else if is_label {
                    ThemeValue::Float(self.legend.label_font_weight)
                } else if is_tick {
                    ThemeValue::Float(self.legend.tick_font_weight)
                } else {
                    ThemeValue::Float(400.0)
                }
            }
            ThemeProperty::Color => {
                if is_title {
                    ThemeValue::String(self.legend.title_color.clone())
                } else if is_label {
                    ThemeValue::String(self.legend.label_color.clone())
                } else if is_tick {
                    ThemeValue::String(self.legend.tick_color.clone())
                } else {
                    ThemeValue::String("#000000".to_string())
                }
            }
            ThemeProperty::BackgroundColor => match &self.legend.background_fill {
                Some(color) => ThemeValue::String(color.clone()),
                None => ThemeValue::None,
            },
            ThemeProperty::Padding => ThemeValue::Float(self.legend.background_padding),
            ThemeProperty::Spacing => {
                if is_label {
                    ThemeValue::Float(self.legend.label_padding)
                } else {
                    ThemeValue::Float(self.legend.item_spacing)
                }
            }
            ThemeProperty::CornerRadius => ThemeValue::Float(self.legend.background_corner_radius),
            ThemeProperty::Size => ThemeValue::Float(self.legend.symbol_size),
            _ => ThemeValue::None,
        }
    }

    /// Query mark-specific properties
    fn query_mark(&self, context: &ThemeContext, property: &ThemeProperty) -> ThemeValue {
        let mark_type = context.mark_type.as_deref().unwrap_or("symbol");
        let channel = match property {
            ThemeProperty::FillColor => "fill",
            ThemeProperty::StrokeColor => "stroke",
            ThemeProperty::StrokeWidth => "stroke_width",
            ThemeProperty::Size => "size",
            ThemeProperty::Opacity => "opacity",
            ThemeProperty::FontFamily => "font",
            ThemeProperty::FontSize => "font_size",
            ThemeProperty::FontWeight => "font_weight",
            _ => return ThemeValue::None,
        };

        // Use the mark defaults system
        if mark_type == "text" && channel == "font_size" {
            // Text marks have computed font size
            let computed = (self.base_font_size * self.font_scale.text_mark_scale).round();
            return ThemeValue::Float(computed);
        }

        if mark_type == "text" && channel == "font" {
            // Text marks inherit font family
            let text_font_size = (self.base_font_size * self.font_scale.text_mark_scale).round();
            if let Some(value) = self.mark_defaults.get_with_computed_fonts(
                mark_type,
                channel,
                text_font_size,
                &self.base_font_family,
            ) {
                if let datafusion_common::ScalarValue::Utf8(Some(s)) = value {
                    return ThemeValue::String(s);
                }
            }
        }

        // Check mark defaults
        if let Some(value) = self.mark_defaults.get(mark_type, channel) {
            match value {
                datafusion_common::ScalarValue::Utf8(Some(s)) => {
                    return ThemeValue::String(s.clone());
                }
                datafusion_common::ScalarValue::Float32(Some(f)) => {
                    return ThemeValue::Float(*f);
                }
                datafusion_common::ScalarValue::Float64(Some(f)) => {
                    return ThemeValue::Float(*f as f32);
                }
                datafusion_common::ScalarValue::Int32(Some(i)) => {
                    return ThemeValue::Integer(*i);
                }
                datafusion_common::ScalarValue::Boolean(Some(b)) => {
                    return ThemeValue::Boolean(*b);
                }
                _ => {}
            }
        }

        ThemeValue::None
    }
}
