use crate::cartesian::{Cartesian, CartesianAxis};
use crate::impl_mark_trait_common;
use crate::marks::{ChannelType, Mark, RadiusExpression};
use arrow::array::RecordBatch;
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use datafusion::logical_expr::{Expr, lit};
use datafusion_common::ScalarValue;
use crate::cartesian::coord::CartesianGeneral;
// Import Symbol for the macro, then re-export it
use crate::error::AvengerChartError;
pub use crate::marks::symbol::Symbol;
use crate::utils::ScalarValueHelpers;

// Implement position channels for CartesianGeneric Symbol
impl<A: CartesianAxis + Default + 'static> Symbol<CartesianGeneral<A>> {
    pub fn x<V: Into<crate::marks::ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    pub fn x_with<F>(self, value: impl Into<crate::marks::ChannelValue>, f: F) -> Self
    where
        F: FnOnce(
            crate::cartesian::channels::CartesianPositionChannel<A>,
        ) -> crate::cartesian::channels::CartesianPositionChannel<A>,
    {
        let channel_value: crate::marks::ChannelValue = value.into();
        let channel = crate::cartesian::channels::CartesianPositionChannel::<A>::new(channel_value);
        let configured = f(channel);

        // Extract axis config before consuming channel
        let axis_config = configured.axis_config().cloned();

        // Store channel value
        let mut mark = self.with_channel_value("x", configured.into_inner());

        // Store axis config if present
        if let Some(axis_config) = axis_config {
            mark.state_mut()
                .axis_configs
                .insert("x".to_string(), axis_config);
        }

        mark
    }

    pub fn y<V: Into<crate::marks::ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    pub fn y_with<F>(self, value: impl Into<crate::marks::ChannelValue>, f: F) -> Self
    where
        F: FnOnce(
            crate::cartesian::channels::CartesianPositionChannel<A>,
        ) -> crate::cartesian::channels::CartesianPositionChannel<A>,
    {
        let channel_value: crate::marks::ChannelValue = value.into();
        let channel = crate::cartesian::channels::CartesianPositionChannel::<A>::new(channel_value);
        let configured = f(channel);

        // Extract axis config before consuming channel
        let axis_config = configured.axis_config().cloned();

        // Store channel value
        let mut mark = self.with_channel_value("y", configured.into_inner());

        // Store axis config if present
        if let Some(axis_config) = axis_config {
            mark.state_mut()
                .axis_configs
                .insert("y".to_string(), axis_config);
        }

        mark
    }

    pub fn position_channel_descriptors() -> Vec<crate::marks::ChannelDescriptor> {
        use crate::marks::ChannelDescriptor;
        vec![
            ChannelDescriptor {
                name: "x",
                channel_type: ChannelType::Numeric,
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "y",
                channel_type: ChannelType::Numeric,
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    pub fn all_channel_descriptors() -> Vec<crate::marks::ChannelDescriptor> {
        let mut descriptors = Self::common_channel_descriptors();
        descriptors.extend(Self::position_channel_descriptors());
        descriptors
    }
}

// Implement Mark trait for CartesianGeneric Symbol with any axis type
impl<A: CartesianAxis + Default + 'static> Mark<CartesianGeneral<A>> for Symbol<CartesianGeneral<A>> {
    impl_mark_trait_common!(Symbol, CartesianGeneral<A>, "symbol");

    fn default_channel_value(&self, channel: &str) -> Option<ScalarValue> {
        match channel {
            "size" => Some(ScalarValue::Float32(Some(64.0))), // Default area
            "shape" => Some(ScalarValue::Utf8(Some("circle".to_string()))), // Default shape
            "angle" => Some(ScalarValue::Float32(Some(0.0))), // Default angle
            "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))), // Default blue
            "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))), // Default black
            "stroke_width" => Some(ScalarValue::Float32(Some(1.0))), // Default stroke width
            "opacity" => Some(ScalarValue::Float32(Some(1.0))), // Fully opaque
            _ => None,
        }
    }

    fn radius_expression(
        &self,
        dimension: &str,
        resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        match dimension {
            "x" | "y" => {
                // Get size and stroke_width expressions (either mapped or default)
                let size_expr = resolve_channel("size");
                let stroke_width_expr = resolve_channel("stroke_width");

                // For symbols: radius = sqrt(area) * 0.5 + stroke_width / 2
                // The size channel represents the area of the bounding square
                // The base circle SVG path has radius 0.5 for a unit square (size=1)
                // Add half the stroke width since stroke extends both inward and outward
                use datafusion::functions::expr_fn::sqrt;
                let radius_expr = sqrt(size_expr) * lit(0.5) + stroke_width_expr / lit(2.0);

                Some(RadiusExpression::Symmetric(radius_expr))
            }
            _ => None,
        }
    }

    fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use crate::marks::util::{
            coerce_color_channel_with_mark, coerce_numeric_channel_with_mark,
        };
        use avenger_scales::scales::coerce::Coercer;

        // Symbols can render with just scalar data
        let coercer = Coercer::default();

        // Extract position data - these can be scalars or arrays
        let x = coerce_numeric_channel_with_mark(self, data, scalars, "x", 0.0)?;
        let y = coerce_numeric_channel_with_mark(self, data, scalars, "y", 0.0)?;

        // Extract other channels using mark defaults
        let size = coerce_numeric_channel_with_mark(self, data, scalars, "size", 64.0)?;
        let fill = coerce_color_channel_with_mark(
            self,
            data,
            scalars,
            "fill",
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0],
        )?;
        let stroke =
            coerce_color_channel_with_mark(self, data, scalars, "stroke", [0.0, 0.0, 0.0, 1.0])?;
        let angle = coerce_numeric_channel_with_mark(self, data, scalars, "angle", 0.0)?;

        // Determine the number of symbols from any array channel
        let len = data.map_or(1, |data| data.num_rows()) as u32;

        // Handle shape channel efficiently - get default from mark
        let shape_default = self
            .default_channel_value("shape")
            .and_then(|scalar| {
                match scalar {
                    ScalarValue::Utf8(Some(s)) => {
                        // Convert string to SymbolShape using from_vega_str
                        avenger_common::types::SymbolShape::from_vega_str(&s).ok()
                    }
                    _ => None,
                }
            })
            .unwrap_or(avenger_common::types::SymbolShape::Circle);

        let (shapes, shape_index) =
            if let Some(shape_array) = data.and_then(|d| d.column_by_name("shape")) {
                // Array data for shapes - use efficient coercion
                coercer.to_symbol_shape(shape_array, Some(shape_default))?
            } else if let Some(shape_scalar) = scalars.column_by_name("shape") {
                // Scalar shape - still use to_symbol_shape for consistency
                coercer.to_symbol_shape(shape_scalar, Some(shape_default))?
            } else {
                // Default shape from mark
                (vec![shape_default], ScalarOrArray::new_scalar(0))
            };

        // Stroke width is scalar only - get default from mark
        let stroke_width_default = self
            .default_channel_value("stroke_width")
            .and_then(|scalar| scalar.as_f32().ok())
            .unwrap_or(1.0);

        let stroke_width = if let Some(width_scalar) = scalars.column_by_name("stroke_width") {
            Some(
                *coercer
                    .to_numeric(width_scalar, Some(stroke_width_default))?
                    .first()
                    .unwrap(),
            )
        } else {
            Some(stroke_width_default)
        };

        let symbol_mark = SceneSymbolMark {
            name: "symbol".to_string(),
            clip: true,
            len,
            gradients: vec![],
            shapes,
            stroke_width,
            shape_index,
            x,
            y,
            fill,
            size,
            stroke,
            angle,
            indices: None,
            zindex: self.get_zindex(),
            x_adjustment: None,
            y_adjustment: None,
        };

        Ok(vec![SceneMark::Symbol(symbol_mark)])
    }
}
