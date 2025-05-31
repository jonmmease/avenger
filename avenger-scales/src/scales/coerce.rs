use crate::error::AvengerScaleError;
use crate::formatter::Formatters;
use crate::scalar::Scalar;
use crate::scales::ordinal::OrdinalScale;
use arrow::array::{Array, AsArray, Float32Array, ListArray, StringArray, StructArray};
use arrow::compute::is_not_null;
use arrow::compute::kernels::zip::zip;
use arrow::datatypes::{Float32Type, UInt32Type, UInt8Type};
use arrow::{
    array::ArrayRef,
    compute::kernels::cast,
    datatypes::{DataType, Field},
};
use avenger_common::types::{
    AreaOrientation, ImageAlign, ImageBaseline, PathTransform, StrokeCap, StrokeJoin, SymbolShape,
};
use avenger_common::{types::ColorOrGradient, value::ScalarOrArray};
use avenger_image::{make_image_fetcher, RgbaImage};
use avenger_text::types::{FontStyle, FontWeight, TextAlign, TextBaseline};
use css_color_parser::Color;
use lyon_extra::parser::{ParserOptions, Source};
use lyon_path::geom::point;
use paste::paste;
use std::f32::NAN;
use std::fmt::Debug;
use std::str::FromStr;
use std::sync::Arc;
use strum::VariantNames;
use svgtypes::Transform;

pub trait ColorCoercer: Debug + Send + Sync + 'static {
    fn coerce(
        &self,
        value: &ArrayRef,
        default_value: Option<ColorOrGradient>,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError>;
}

pub trait NumericCoercer: Debug + Send + Sync + 'static {
    fn coerce(
        &self,
        value: &ArrayRef,
        default_value: Option<f32>,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError>;

    fn coerce_usize(&self, value: &ArrayRef) -> Result<ScalarOrArray<usize>, AvengerScaleError>;
    fn coerce_boolean(&self, value: &ArrayRef) -> Result<ScalarOrArray<bool>, AvengerScaleError>;
}

#[derive(Default, Debug, Clone, Copy)]
pub struct CastNumericCoercer;

impl NumericCoercer for CastNumericCoercer {
    fn coerce(
        &self,
        value: &ArrayRef,
        default_value: Option<f32>,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let cast_array = cast(value, &DataType::Float32)?;
        let result = cast_array.as_primitive::<Float32Type>();

        if result.null_count() > 0 {
            let mask = is_not_null(result)?;
            let fill_array = Float32Array::from(vec![default_value.unwrap_or(NAN); result.len()]);
            let filled = zip(&mask, &result, &fill_array)?;
            let result_vec = filled.as_primitive::<Float32Type>().values().to_vec();
            Ok(ScalarOrArray::new_array(result_vec))
        } else {
            Ok(ScalarOrArray::new_array(result.values().to_vec()))
        }
    }

    fn coerce_usize(&self, value: &ArrayRef) -> Result<ScalarOrArray<usize>, AvengerScaleError> {
        let cast_array = cast(value, &DataType::UInt32)?;
        Ok(ScalarOrArray::new_array(
            cast_array
                .as_primitive::<UInt32Type>()
                .values()
                .iter()
                .map(|el| *el as usize)
                .collect(),
        ))
    }

    fn coerce_boolean(&self, value: &ArrayRef) -> Result<ScalarOrArray<bool>, AvengerScaleError> {
        let cast_array = cast(value, &DataType::UInt8)?;
        Ok(ScalarOrArray::new_array(
            cast_array
                .as_primitive::<UInt8Type>()
                .values()
                .iter()
                .map(|el| *el != 0)
                .collect(),
        ))
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub struct CssColorCoercer;

impl ColorCoercer for CssColorCoercer {
    fn coerce(
        &self,
        value: &ArrayRef,
        default_value: Option<ColorOrGradient>,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        let dtype = value.data_type();
        let default_value = default_value.unwrap_or(ColorOrGradient::transparent());
        match dtype {
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
                // cast to normalize to utf8
                let cast_array = cast(value, &DataType::Utf8)?;
                let string_array = cast_array.as_string::<i32>();
                let result = string_array
                    .iter()
                    .map(|el| match el {
                        Some(el) => el
                            .parse::<Color>()
                            .map(|color| {
                                ColorOrGradient::Color([
                                    color.r as f32 / 255.0,
                                    color.g as f32 / 255.0,
                                    color.b as f32 / 255.0,
                                    color.a,
                                ])
                            })
                            .unwrap_or_else(|_| default_value.clone()),
                        _ => default_value.clone(),
                    })
                    .collect::<Vec<_>>();
                Ok(ScalarOrArray::new_array(result))
            }

            DataType::List(field)
            | DataType::ListView(field)
            | DataType::FixedSizeList(field, _)
            | DataType::LargeList(field)
            | DataType::LargeListView(field)
                if field.data_type().is_numeric() =>
            {
                // Cast to normalize to list of f32 arrays
                let cast_type = DataType::List(Field::new("item", DataType::Float32, true).into());
                let cast_array = cast(value, &cast_type)?;
                let list_array = cast_array.as_list::<i32>();
                let result = list_array
                    .iter()
                    .map(|el| match el {
                        Some(el) if el.len() == 4 => {
                            let values = el.as_primitive::<Float32Type>();
                            ColorOrGradient::Color([
                                values.value(0),
                                values.value(1),
                                values.value(2),
                                values.value(3),
                            ])
                        }
                        _ => default_value.clone(),
                    })
                    .collect::<Vec<_>>();
                Ok(ScalarOrArray::new_array(result))
            }
            _ => Err(AvengerScaleError::InternalError(format!(
                "Unsupported data type for coercing to color: {:?}",
                dtype
            ))),
        }
    }
}

macro_rules! define_enum_coercer {
    ($enum_type:ty) => {
        paste! {
            pub fn [<to_ $enum_type:snake> ](
                &self,
                values: &ArrayRef,
            ) -> Result<ScalarOrArray<$enum_type>, AvengerScaleError> {
                let domain = Arc::new(StringArray::from(Vec::from(<$enum_type>::VARIANTS))) as ArrayRef;
                let scale = OrdinalScale::new(domain.clone()).with_range(domain);
                scale.[<scale_to_ $enum_type:snake>](values)
            }
        }
    };
}

#[derive(Debug, Clone)]
pub struct Coercer {
    pub color_coercer: Arc<dyn ColorCoercer>,
    pub number_coercer: Arc<dyn NumericCoercer>,
    pub formatters: Formatters,
}

impl Default for Coercer {
    fn default() -> Self {
        Self {
            color_coercer: Arc::new(CssColorCoercer),
            number_coercer: Arc::new(CastNumericCoercer),
            formatters: Formatters::default(),
        }
    }
}

impl Coercer {
    pub fn to_numeric(
        &self,
        values: &ArrayRef,
        default_value: Option<f32>,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        self.number_coercer.coerce(values, default_value)
    }

    pub fn to_usize(&self, values: &ArrayRef) -> Result<ScalarOrArray<usize>, AvengerScaleError> {
        self.number_coercer.coerce_usize(values)
    }

    pub fn to_boolean(&self, values: &ArrayRef) -> Result<ScalarOrArray<bool>, AvengerScaleError> {
        self.number_coercer.coerce_boolean(values)
    }

    pub fn to_color(
        &self,
        values: &ArrayRef,
        default_value: Option<ColorOrGradient>,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        self.color_coercer.coerce(values, default_value)
    }

    pub fn to_string(
        &self,
        values: &ArrayRef,
        default_value: Option<&str>,
    ) -> Result<ScalarOrArray<String>, AvengerScaleError> {
        self.formatters.format(values, default_value)
    }

    define_enum_coercer!(StrokeCap);
    define_enum_coercer!(StrokeJoin);
    define_enum_coercer!(ImageAlign);
    define_enum_coercer!(ImageBaseline);
    define_enum_coercer!(AreaOrientation);
    define_enum_coercer!(TextAlign);
    define_enum_coercer!(TextBaseline);
    define_enum_coercer!(FontWeight);
    define_enum_coercer!(FontStyle);

    pub fn to_image(
        &self,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<RgbaImage>, AvengerScaleError> {
        let dtype = values.data_type();
        let mut result = Vec::new();
        match dtype {
            // Handle strings
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
                let fetcher = make_image_fetcher()?;
                let cast_array = cast(values, &DataType::Utf8)?;
                let string_array = cast_array.as_string::<i32>();
                for s in string_array.iter() {
                    if let Some(s) = s {
                        let img = RgbaImage::from_str(s, Some(fetcher.clone()))?;
                        result.push(img);
                    }
                }
            }
            // Handle raw rgba image data
            DataType::Struct(fields) => {
                let field_names = fields.iter().map(|f| f.name().as_str()).collect::<Vec<_>>();

                let msg = format!(
                    "Unsupported struct data type for coercing to image: {:?}\n
Expected struct with fields [width(UInt32), height(UInt32), data(List[UInt8])]",
                    field_names
                );

                // Check field names and order
                if field_names != ["width", "height", "data"] {
                    return Err(AvengerScaleError::InternalError(msg));
                }

                // Check field types
                let width = fields.first().unwrap();
                let height = fields.get(1).unwrap();
                let data = fields.get(2).unwrap();

                let expected_data_type = DataType::new_list(DataType::UInt8, false);
                if data.data_type() != &expected_data_type
                    || height.data_type() != &DataType::UInt32
                    || width.data_type() != &DataType::UInt32
                {
                    return Err(AvengerScaleError::InternalError(msg));
                }

                // Cast to struct
                let struct_array = values.as_struct();
                let width = struct_array.column(0);
                let height = struct_array.column(1);
                let data = struct_array.column(2);

                let width = width.as_primitive::<UInt32Type>();
                let height = height.as_primitive::<UInt32Type>();
                let data = data.as_list::<i32>();

                for i in 0..values.len() {
                    let width = width.value(i);
                    let height = height.value(i);
                    let data = data.value(i);
                    let data = data.as_primitive::<UInt8Type>();
                    let data = data.values().to_vec();
                    let img = RgbaImage {
                        width,
                        height,
                        data,
                    };
                    result.push(img);
                }
            }
            _ => {
                return Err(AvengerScaleError::InternalError(format!(
                    "Unsupported data type for coercing to image: {:?}",
                    dtype
                )))
            }
        }
        Ok(ScalarOrArray::new_array(result))
    }

    pub fn to_stroke_dash(
        &self,
        value: &ArrayRef,
    ) -> Result<ScalarOrArray<Vec<f32>>, AvengerScaleError> {
        let dtype = value.data_type();
        let mut result = Vec::new();

        match dtype {
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
                // Convert strings to stroke dash vectors
                let cast_array = cast(value, &DataType::Utf8)?;
                let cast_array = cast_array.as_string::<i32>();
                for s in cast_array.iter() {
                    if let Some(s) = s {
                        let s = s.replace(",", "");
                        let v = s
                            .split(" ")
                            .filter_map(|p| p.parse::<f32>().ok())
                            .collect::<Vec<_>>();
                        result.push(v);
                    } else {
                        result.push(Vec::new());
                    }
                }
            }
            DataType::List(field)
            | DataType::ListView(field)
            | DataType::FixedSizeList(field, _)
            | DataType::LargeList(field)
            | DataType::LargeListView(field)
                if field.data_type().is_numeric() =>
            {
                // Convert list of numbers
                let cast_array = cast(value, &DataType::new_list(DataType::Float32, false))?;
                let list_array = cast_array.as_list::<i32>();
                for i in 0..list_array.len() {
                    let values = list_array.value(i);
                    let values = values.as_primitive::<Float32Type>().values().to_vec();
                    result.push(values);
                }
            }
            _ => {
                return Err(AvengerScaleError::InternalError(format!(
                    "Unsupported data type for coercing to color: {:?}",
                    dtype
                )))
            }
        }
        Ok(ScalarOrArray::new_array(result))
    }

    pub fn to_path_transform(
        &self,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<PathTransform>, AvengerScaleError> {
        let dtype = values.data_type();
        let mut result = Vec::new();
        match dtype {
            // Handle svg-style transform strings
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
                // e.g. "rotate(-10 50 100) translate(-36 45.5) skewX(40) scale(1 0.5)"
                let cast_array = cast(values, &DataType::Utf8)?;
                let string_array = cast_array.as_string::<i32>();
                for s in string_array.iter() {
                    if let Some(s) = s {
                        let ts = Transform::from_str(s)?;
                        let transform = PathTransform::new(
                            ts.a as f32,
                            ts.b as f32,
                            ts.c as f32,
                            ts.d as f32,
                            ts.e as f32,
                            ts.f as f32,
                        );
                        result.push(transform);
                    }
                }
            }
            // Handle struct with fields for each transform component
            DataType::Struct(fields) => {
                let field_names = fields.iter().map(|f| f.name().as_str()).collect::<Vec<_>>();

                let msg = format!(
                    "Unsupported struct data type for coercing to path transform: {:?}\n
Expected struct with fields [a(Float32), b(Float32), c(Float32), d(Float32), e(Float32), f(Float32)]",
                    field_names
                );

                // Check field names and order
                if field_names != ["a", "b", "c", "d", "e", "f"] {
                    return Err(AvengerScaleError::InternalError(msg));
                }

                // Check field types
                let a_type = fields.first().unwrap();
                let b_type = fields.get(1).unwrap();
                let c_type = fields.get(2).unwrap();
                let d_type = fields.get(3).unwrap();
                let e_type = fields.get(4).unwrap();
                let f_type = fields.get(5).unwrap();

                if a_type.data_type() != &DataType::Float32
                    || b_type.data_type() != &DataType::Float32
                    || c_type.data_type() != &DataType::Float32
                    || d_type.data_type() != &DataType::Float32
                    || e_type.data_type() != &DataType::Float32
                    || f_type.data_type() != &DataType::Float32
                {
                    return Err(AvengerScaleError::InternalError(msg));
                }

                // Cast to struct
                let struct_array = values.as_struct();
                let a = struct_array.column(0).as_primitive::<Float32Type>();
                let b = struct_array.column(1).as_primitive::<Float32Type>();
                let c = struct_array.column(2).as_primitive::<Float32Type>();
                let d = struct_array.column(3).as_primitive::<Float32Type>();
                let e = struct_array.column(4).as_primitive::<Float32Type>();
                let f = struct_array.column(5).as_primitive::<Float32Type>();

                for i in 0..values.len() {
                    let a = a.value(i);
                    let b = b.value(i);
                    let c = c.value(i);
                    let d = d.value(i);
                    let e = e.value(i);
                    let f = f.value(i);
                    let transform = PathTransform::new(a, b, c, d, e, f);
                    result.push(transform);
                }
            }
            _ => {
                return Err(AvengerScaleError::InternalError(format!(
                    "Unsupported data type for coercing to path transform: {:?}",
                    dtype
                )))
            }
        }
        Ok(ScalarOrArray::new_array(result))
    }

    pub fn to_symbol_shapes(&self, value: &Scalar) -> Result<Vec<SymbolShape>, AvengerScaleError> {
        match value.data_type() {
            DataType::List(_) => {
                let list_array = value
                    .array()
                    .as_any()
                    .downcast_ref::<ListArray>()
                    .ok_or_else(|| {
                        AvengerScaleError::InternalError(
                            "Failed to downcast to ListArray".to_string(),
                        )
                    })?;
                let val = list_array.value(0);
                match val.data_type() {
                    DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
                        let cast_array = cast(&val, &DataType::Utf8)?;
                        let string_array = cast_array.as_string::<i32>();
                        let mut result = Vec::new();
                        for s in string_array.iter() {
                            if let Some(s) = s {
                                let symbol_shape = SymbolShape::from_vega_str(s)?;
                                result.push(symbol_shape);
                            } else {
                                result.push(SymbolShape::default());
                            }
                        }
                        Ok(result)
                    }
                    _ => {
                        Err(AvengerScaleError::InternalError(format!(
                            "Unsupported data type for coercing to symbol shape: {:?}",
                            val.data_type()
                        )))
                    }
                }
            }
            _ => {
                Err(AvengerScaleError::InternalError(format!(
                    "Unsupported data type for coercing to symbol shape: {:?}",
                    value.data_type()
                )))
            }
        }
    }

    pub fn to_path(
        &self,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<lyon_path::Path>, AvengerScaleError> {
        let dtype = values.data_type();
        let mut result = Vec::new();

        match dtype {
            // Handle svg-style path strings
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
                // e.g. "M 10 10 L 100 100"
                let cast_array = cast(values, &DataType::Utf8)?;
                let string_array = cast_array.as_string::<i32>();
                for s in string_array.iter() {
                    if let Some(s) = s {
                        let path = parse_svg_path(s)?;
                        result.push(path);
                    }
                }
            }
            // Handle struct with fields for path verbs and points
            DataType::Struct(fields) => {
                let field_names = fields.iter().map(|f| f.name().as_str()).collect::<Vec<_>>();
                let msg = format!(
                    "Unsupported struct data type for coercing to path: {:?}\n
Expected struct with fields [verbs(List[UInt8]), points(List[Float32])]",
                    field_names
                );

                // Check field names and order
                if field_names != ["verbs", "points"] {
                    return Err(AvengerScaleError::InternalError(msg));
                }

                // Check field types
                let verbs_type = fields.first().unwrap();
                let points_type = fields.get(1).unwrap();

                if !(verbs_type.data_type() == &DataType::new_list(DataType::UInt8, false)
                    || verbs_type.data_type() == &DataType::new_list(DataType::UInt8, true))
                    || !(points_type.data_type() == &DataType::new_list(DataType::Float32, false)
                        || points_type.data_type() == &DataType::new_list(DataType::Float32, true))
                {
                    return Err(AvengerScaleError::InternalError(msg));
                }

                result.extend(arrow_array_to_paths(values)?);
            }
            _ => {
                return Err(AvengerScaleError::InternalError(format!(
                    "Unsupported data type for coercing to path transform: {:?}",
                    dtype
                )))
            }
        }
        Ok(ScalarOrArray::new_array(result))
    }
}

fn parse_svg_path(path: &str) -> Result<lyon_path::Path, AvengerScaleError> {
    let mut source = Source::new(path.chars());
    let mut parser = lyon_extra::parser::PathParser::new();
    let opts = ParserOptions::DEFAULT;
    let mut builder = lyon_path::Path::builder();
    parser.parse(&opts, &mut source, &mut builder)?;
    Ok(builder.build())
}

fn verbs_and_points_to_path(verbs: &[u8], points: &[f32]) -> lyon_path::Path {
    let mut builder = lyon_path::Path::builder();
    let mut point_idx = 0;

    for &verb in verbs {
        match verb {
            0 => {
                // Begin
                let x = points[point_idx];
                let y = points[point_idx + 1];
                builder.begin(point(x, y));
                point_idx += 2;
            }
            1 => {
                // Line
                let x = points[point_idx];
                let y = points[point_idx + 1];
                builder.line_to(point(x, y));
                point_idx += 2;
            }
            2 => {
                // Quadratic
                let cx = points[point_idx];
                let cy = points[point_idx + 1];
                let x = points[point_idx + 2];
                let y = points[point_idx + 3];
                builder.quadratic_bezier_to(point(cx, cy), point(x, y));
                point_idx += 4;
            }
            3 => {
                // Cubic
                let c1x = points[point_idx];
                let c1y = points[point_idx + 1];
                let c2x = points[point_idx + 2];
                let c2y = points[point_idx + 3];
                let x = points[point_idx + 4];
                let y = points[point_idx + 5];
                builder.cubic_bezier_to(point(c1x, c1y), point(c2x, c2y), point(x, y));
                point_idx += 6;
            }
            4 => {
                // End
                builder.end(false);
            }
            5 => {
                // Close
                builder.end(true);
            }
            _ => panic!("Invalid verb"),
        }
    }

    builder.build()
}

pub fn arrow_array_to_paths(values: &ArrayRef) -> Result<Vec<lyon_path::Path>, AvengerScaleError> {
    // Cast to struct
    let struct_array = values.as_struct();
    let verbs = struct_array.column(0).as_list::<i32>();
    let points = struct_array.column(1).as_list::<i32>();

    let mut result = Vec::new();

    for i in 0..values.len() {
        let verbs = verbs.value(i);
        let verbs = verbs.as_primitive::<UInt8Type>().values().to_vec();
        let points = points.value(i);
        let points = points.as_primitive::<Float32Type>().values().to_vec();
        let path = verbs_and_points_to_path(&verbs, &points);
        result.push(path);
    }

    Ok(result)
}

/// Create an Arrow array from a collection of paths
pub fn paths_to_arrow_array(paths: &[lyon_path::Path]) -> ArrayRef {
    // Create builders
    let mut verbs_builder =
        arrow::array::builder::ListBuilder::new(arrow::array::builder::UInt8Builder::new());
    let mut points_builder =
        arrow::array::builder::ListBuilder::new(arrow::array::builder::Float32Builder::new());

    // Add each path's data to the builders
    for path in paths {
        let verbs_values = verbs_builder.values();
        let points_values = points_builder.values();

        for event in path.iter() {
            match event {
                lyon_path::Event::Begin { at } => {
                    verbs_values.append_value(0);
                    points_values.append_value(at.x);
                    points_values.append_value(at.y);
                }
                lyon_path::Event::Line { to, .. } => {
                    verbs_values.append_value(1);
                    points_values.append_value(to.x);
                    points_values.append_value(to.y);
                }
                lyon_path::Event::Quadratic { ctrl, to, .. } => {
                    verbs_values.append_value(2);
                    points_values.append_value(ctrl.x);
                    points_values.append_value(ctrl.y);
                    points_values.append_value(to.x);
                    points_values.append_value(to.y);
                }
                lyon_path::Event::Cubic {
                    ctrl1, ctrl2, to, ..
                } => {
                    verbs_values.append_value(3);
                    points_values.append_value(ctrl1.x);
                    points_values.append_value(ctrl1.y);
                    points_values.append_value(ctrl2.x);
                    points_values.append_value(ctrl2.y);
                    points_values.append_value(to.x);
                    points_values.append_value(to.y);
                }
                lyon_path::Event::End { close, .. } => {
                    verbs_values.append_value(if close { 5 } else { 4 });
                }
            }
        }

        // Start a new list for this path's verbs and points
        verbs_builder.append(true);
        points_builder.append(true);
    }

    // Create the struct array containing both lists
    let struct_array = StructArray::from(vec![
        (
            Arc::new(Field::new(
                "verbs",
                DataType::new_list(DataType::UInt8, true),
                false,
            )),
            Arc::new(verbs_builder.finish()) as ArrayRef,
        ),
        (
            Arc::new(Field::new(
                "points",
                DataType::new_list(DataType::Float32, true),
                false,
            )),
            Arc::new(points_builder.finish()) as ArrayRef,
        ),
    ]);

    Arc::new(struct_array)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use arrow::array::{Float32Array, ListArray, StringArray};
    use arrow::buffer::OffsetBuffer;
    use arrow::datatypes::Field;
    use avenger_common::types::ColorOrGradient;
    use svgtypes::Transform;

    fn assert_color_approx_eq(actual: [f32; 4], expected: [f32; 4], tolerance: f32) {
        for i in 0..4 {
            assert!(
                (actual[i] - expected[i]).abs() < tolerance,
                "Color component {} differs: actual={}, expected={}, tolerance={}",
                i,
                actual[i],
                expected[i],
                tolerance
            );
        }
    }

    #[test]
    fn test_css_color_parsing_basic() {
        let coercer = CssColorCoercer;
        let colors = StringArray::from(vec![
            "red", "green", "blue", "#ff0000", "#00ff00", "#0000ff",
        ]);
        let result = coercer
            .coerce(&(Arc::new(colors) as ArrayRef), None)
            .unwrap();
        let colors_vec = result.as_vec(6, None);

        // Test named colors
        if let ColorOrGradient::Color(red) = &colors_vec[0] {
            assert_color_approx_eq(*red, [1.0, 0.0, 0.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        if let ColorOrGradient::Color(green) = &colors_vec[1] {
            assert_color_approx_eq(*green, [0.0, 0.5, 0.0, 1.0], 0.002); // CSS green is [0, 128, 0]
        } else {
            panic!("Expected Color variant");
        }

        if let ColorOrGradient::Color(blue) = &colors_vec[2] {
            assert_color_approx_eq(*blue, [0.0, 0.0, 1.0, 1.0], 0.002);
        } else {
            panic!("Expected Color variant");
        }

        // Test hex colors
        if let ColorOrGradient::Color(hex_red) = &colors_vec[3] {
            assert_color_approx_eq(*hex_red, [1.0, 0.0, 0.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }
    }

    #[test]
    fn test_css_color_parsing_hex_formats() {
        let coercer = CssColorCoercer;
        let colors = StringArray::from(vec![
            "#f00",      // 3-digit hex
            "#ff0000",   // 6-digit hex
            "#ff000080", // 8-digit hex with alpha
            "#FF0000",   // Uppercase
        ]);
        let result = coercer
            .coerce(&(Arc::new(colors) as ArrayRef), None)
            .unwrap();
        let colors_vec = result.as_vec(4, None);

        // All should be red with different alpha values
        if let ColorOrGradient::Color(red_3) = &colors_vec[0] {
            assert_color_approx_eq(*red_3, [1.0, 0.0, 0.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        if let ColorOrGradient::Color(red_6) = &colors_vec[1] {
            assert_color_approx_eq(*red_6, [1.0, 0.0, 0.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        if let ColorOrGradient::Color(red_8) = &colors_vec[2] {
            // 8-digit hex colors are not supported by css_color_parser, falls back to transparent
            assert_color_approx_eq(*red_8, [0.0, 0.0, 0.0, 0.0], 0.01);
        } else {
            panic!("Expected Color variant");
        }

        if let ColorOrGradient::Color(red_upper) = &colors_vec[3] {
            assert_color_approx_eq(*red_upper, [1.0, 0.0, 0.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }
    }

    #[test]
    fn test_css_color_parsing_rgb_formats() {
        let coercer = CssColorCoercer;
        let colors = StringArray::from(vec![
            "rgb(255, 0, 0)",
            "rgba(255, 0, 0, 0.5)",
            "rgb(255,0,0)",      // No spaces
            "rgb(100%, 0%, 0%)", // Percentage values
        ]);
        let result = coercer
            .coerce(&(Arc::new(colors) as ArrayRef), None)
            .unwrap();
        let colors_vec = result.as_vec(4, None);

        if let ColorOrGradient::Color(rgb) = &colors_vec[0] {
            assert_color_approx_eq(*rgb, [1.0, 0.0, 0.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        if let ColorOrGradient::Color(rgba) = &colors_vec[1] {
            assert_color_approx_eq(*rgba, [1.0, 0.0, 0.0, 0.5], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        if let ColorOrGradient::Color(rgb_no_space) = &colors_vec[2] {
            assert_color_approx_eq(*rgb_no_space, [1.0, 0.0, 0.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        if let ColorOrGradient::Color(rgb_percent) = &colors_vec[3] {
            assert_color_approx_eq(*rgb_percent, [1.0, 0.0, 0.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }
    }

    #[test]
    fn test_css_color_parsing_hsl_formats() {
        let coercer = CssColorCoercer;
        let colors = StringArray::from(vec![
            "hsl(0, 100%, 50%)",       // Red
            "hsl(120, 100%, 50%)",     // Green
            "hsl(240, 100%, 50%)",     // Blue
            "hsla(0, 100%, 50%, 0.5)", // Semi-transparent red
        ]);
        let result = coercer
            .coerce(&(Arc::new(colors) as ArrayRef), None)
            .unwrap();
        let colors_vec = result.as_vec(4, None);

        if let ColorOrGradient::Color(hsl_red) = &colors_vec[0] {
            assert_color_approx_eq(*hsl_red, [1.0, 0.0, 0.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        if let ColorOrGradient::Color(hsl_green) = &colors_vec[1] {
            assert_color_approx_eq(*hsl_green, [0.0, 1.0, 0.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        if let ColorOrGradient::Color(hsl_blue) = &colors_vec[2] {
            assert_color_approx_eq(*hsl_blue, [0.0, 0.0, 1.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        if let ColorOrGradient::Color(hsla_red) = &colors_vec[3] {
            assert_color_approx_eq(*hsla_red, [1.0, 0.0, 0.0, 0.5], 0.001);
        } else {
            panic!("Expected Color variant");
        }
    }

    #[test]
    fn test_css_color_parsing_invalid_colors() {
        let coercer = CssColorCoercer;
        let default_color = ColorOrGradient::Color([0.5, 0.5, 0.5, 1.0]); // Gray default

        let colors = StringArray::from(vec![
            "invalid_color",
            "#gggggg",        // Invalid hex
            "rgb(256, 0, 0)", // Out of range
            "",               // Empty string
            "notacolor",
        ]);
        let result = coercer
            .coerce(&(Arc::new(colors) as ArrayRef), Some(default_color.clone()))
            .unwrap();
        let colors_vec = result.as_vec(5, None);

        // Test each invalid color individually - different behaviors based on CSS parser
        // "invalid_color" -> default
        if let ColorOrGradient::Color(c) = &colors_vec[0] {
            assert_color_approx_eq(*c, [0.5, 0.5, 0.5, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        // "#gggggg" -> default
        if let ColorOrGradient::Color(c) = &colors_vec[1] {
            assert_color_approx_eq(*c, [0.5, 0.5, 0.5, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        // "rgb(256, 0, 0)" -> red (CSS parser clamps 256 to 255)
        if let ColorOrGradient::Color(c) = &colors_vec[2] {
            assert_color_approx_eq(*c, [1.0, 0.0, 0.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        // "" -> default
        if let ColorOrGradient::Color(c) = &colors_vec[3] {
            assert_color_approx_eq(*c, [0.5, 0.5, 0.5, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        // "notacolor" -> default
        if let ColorOrGradient::Color(c) = &colors_vec[4] {
            assert_color_approx_eq(*c, [0.5, 0.5, 0.5, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }
    }

    #[test]
    fn test_css_color_parsing_null_values() {
        let coercer = CssColorCoercer;
        let default_color = ColorOrGradient::Color([0.5, 0.5, 0.5, 1.0]);

        let mut builder = arrow::array::builder::StringBuilder::new();
        builder.append_value("red");
        builder.append_null();
        builder.append_value("blue");
        let colors = builder.finish();

        let result = coercer
            .coerce(&(Arc::new(colors) as ArrayRef), Some(default_color.clone()))
            .unwrap();
        let colors_vec = result.as_vec(3, None);

        // First should be red
        if let ColorOrGradient::Color(red) = &colors_vec[0] {
            assert_color_approx_eq(*red, [1.0, 0.0, 0.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        // Second should be default (null value)
        if let ColorOrGradient::Color(default) = &colors_vec[1] {
            assert_color_approx_eq(*default, [0.5, 0.5, 0.5, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        // Third should be blue
        if let ColorOrGradient::Color(blue) = &colors_vec[2] {
            assert_color_approx_eq(*blue, [0.0, 0.0, 1.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }
    }

    #[test]
    fn test_color_coercion_from_numeric_arrays() {
        let coercer = CssColorCoercer;

        // Create a ListArray with 4-element float arrays representing RGBA colors
        let values = vec![
            1.0, 0.0, 0.0, 1.0, // Red
            0.0, 1.0, 0.0, 0.8, // Green with alpha
            0.0, 0.0, 1.0, 1.0, // Blue
        ];
        let list_array = ListArray::new(
            Arc::new(Field::new_list_field(DataType::Float32, true)),
            OffsetBuffer::from_lengths(vec![4, 4, 4]),
            Arc::new(Float32Array::from(values)),
            None,
        );

        let result = coercer
            .coerce(&(Arc::new(list_array) as ArrayRef), None)
            .unwrap();
        let colors_vec = result.as_vec(3, None);

        if let ColorOrGradient::Color(red) = &colors_vec[0] {
            assert_color_approx_eq(*red, [1.0, 0.0, 0.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        if let ColorOrGradient::Color(green) = &colors_vec[1] {
            assert_color_approx_eq(*green, [0.0, 1.0, 0.0, 0.8], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        if let ColorOrGradient::Color(blue) = &colors_vec[2] {
            assert_color_approx_eq(*blue, [0.0, 0.0, 1.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }
    }

    #[test]
    fn test_color_coercion_invalid_numeric_arrays() {
        let coercer = CssColorCoercer;
        let default_color = ColorOrGradient::Color([0.5, 0.5, 0.5, 1.0]);

        // Create arrays with wrong number of elements
        let values = vec![
            1.0, 0.0, // Only 2 elements (should be 4)
            0.0, 1.0, 0.0, // Only 3 elements
            0.0, 0.0, 1.0, 1.0, 0.5, // 5 elements (too many)
        ];
        let list_array = ListArray::new(
            Arc::new(Field::new_list_field(DataType::Float32, true)),
            OffsetBuffer::from_lengths(vec![2, 3, 5]),
            Arc::new(Float32Array::from(values)),
            None,
        );

        let result = coercer
            .coerce(
                &(Arc::new(list_array) as ArrayRef),
                Some(default_color.clone()),
            )
            .unwrap();
        let colors_vec = result.as_vec(3, None);

        // All invalid arrays should return the default color
        for color in colors_vec {
            if let ColorOrGradient::Color(c) = color {
                assert_color_approx_eq(c, [0.5, 0.5, 0.5, 1.0], 0.001);
            } else {
                panic!("Expected Color variant");
            }
        }
    }

    #[test]
    fn test_color_coercion_unsupported_types() {
        let coercer = CssColorCoercer;

        // Try with an unsupported data type (Int32)
        let int_array = arrow::array::Int32Array::from(vec![1, 2, 3]);
        let result = coercer.coerce(&(Arc::new(int_array) as ArrayRef), None);

        assert!(result.is_err());
        if let Err(AvengerScaleError::InternalError(msg)) = result {
            assert!(msg.contains("Unsupported data type for coercing to color"));
        } else {
            panic!("Expected InternalError about unsupported data type");
        }
    }

    #[test]
    fn test_color_coercion_edge_cases() {
        let coercer = CssColorCoercer;

        // Test edge cases with CSS colors
        let colors = StringArray::from(vec![
            "transparent",
            "currentColor",
            "inherit",
            "initial",
            "unset",
        ]);

        let result = coercer
            .coerce(&(Arc::new(colors) as ArrayRef), None)
            .unwrap();
        let colors_vec = result.as_vec(5, None);

        // "transparent" should parse correctly
        if let ColorOrGradient::Color(transparent) = &colors_vec[0] {
            assert_color_approx_eq(*transparent, [0.0, 0.0, 0.0, 0.0], 0.001);
        } else {
            panic!("Expected Color variant for transparent");
        }

        // Other CSS keywords should fall back to default (transparent)
        for i in 1..colors_vec.len() {
            if let ColorOrGradient::Color(c) = &colors_vec[i] {
                assert_color_approx_eq(*c, [0.0, 0.0, 0.0, 0.0], 0.001);
            } else {
                panic!("Expected Color variant");
            }
        }
    }

    #[test]
    fn test_color_coercion_whitespace_handling() {
        let coercer = CssColorCoercer;

        let colors = StringArray::from(vec![
            "  red  ",               // Leading/trailing whitespace
            "\tblue\n",              // Tab and newline
            " rgb( 255 , 0 , 0 ) ",  // Whitespace in rgb
            "hsl( 120, 100%, 50% )", // Whitespace in hsl
        ]);

        let result = coercer
            .coerce(&(Arc::new(colors) as ArrayRef), None)
            .unwrap();
        let colors_vec = result.as_vec(4, None);

        // All should parse correctly despite whitespace
        if let ColorOrGradient::Color(red) = &colors_vec[0] {
            assert_color_approx_eq(*red, [1.0, 0.0, 0.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }

        if let ColorOrGradient::Color(blue) = &colors_vec[1] {
            assert_color_approx_eq(*blue, [0.0, 0.0, 1.0, 1.0], 0.001);
        } else {
            panic!("Expected Color variant");
        }
    }

    #[test]
    fn test_to_path_transform() {
        let ts = Transform::from_str("rotate(-10 50 100)").unwrap();
        println!("{:?}", ts);
    }

    #[test]
    fn test_paths_to_arrow_array() {
        let path = parse_svg_path("M 10 10 L 100 100").unwrap();
        let array = paths_to_arrow_array(&[path]);
        println!("{:?}", array);

        let coerer = Coercer::default();
        let paths = coerer.to_path(&array).unwrap();
        println!("{:?}", paths);
    }

    #[test]
    fn test_number_formatting_default() {
        use crate::formatter::{DefaultFormatter, NumberFormatter};

        let formatter = DefaultFormatter::default();
        let values = vec![Some(1.0), Some(2.5), Some(-3.14), None, Some(0.0)];
        let result = formatter.format(&values, Some("N/A"));

        assert_eq!(result, vec!["1", "2.5", "-3.14", "N/A", "0"]);
    }

    #[test]
    fn test_number_formatting_with_format_string() {
        use crate::formatter::{DefaultFormatter, NumberFormatter};

        let formatter = DefaultFormatter {
            format_str: Some(",.2f".to_string()),
            local_tz: None,
        };
        let values = vec![Some(1234.567), Some(0.123), Some(-987.654), None];
        let result = formatter.format(&values, Some("--"));

        // format_num correctly handles d3-style formatting
        assert_eq!(result, vec!["1,234.57", "0.12", "-987.65", "--"]);
    }

    #[test]
    fn test_number_formatting_percentage() {
        use crate::formatter::{DefaultFormatter, NumberFormatter};

        let formatter = DefaultFormatter {
            format_str: Some(".1%".to_string()),
            local_tz: None,
        };
        let values = vec![Some(0.5), Some(0.123), Some(1.0), None];
        let result = formatter.format(&values, Some("N/A"));

        // The [.1%] format works correctly with numfmt
        assert_eq!(result, vec!["50.0%", "12.3%", "100.0%", "N/A"]);
    }

    #[test]
    fn test_number_formatting_scientific_notation() {
        use crate::formatter::{DefaultFormatter, NumberFormatter};

        let formatter = DefaultFormatter {
            format_str: Some(".2e".to_string()),
            local_tz: None,
        };
        let values = vec![Some(1234.0), Some(0.00123), None, Some(0.0)];
        let result = formatter.format(&values, Some("--"));

        // format_num correctly produces scientific notation
        assert_eq!(result, vec!["1.23e+03", "1.23e-03", "--", "0.00e+00"]);
    }

    #[test]
    fn test_date_formatting_default() {
        use crate::formatter::{DateFormatter, DefaultFormatter};
        use chrono::NaiveDate;

        let formatter = DefaultFormatter::default();
        let values = vec![
            Some(NaiveDate::from_ymd_opt(2023, 12, 25).unwrap()),
            Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
            None,
        ];
        let result = formatter.format(&values, Some("Unknown"));

        assert_eq!(result, vec!["2023-12-25", "2024-01-01", "Unknown"]);
    }

    #[test]
    fn test_date_formatting_with_format_string() {
        use crate::formatter::{DateFormatter, DefaultFormatter};
        use chrono::NaiveDate;

        let formatter = DefaultFormatter {
            format_str: Some("%B %d, %Y".to_string()),
            local_tz: None,
        };
        let values = vec![
            Some(NaiveDate::from_ymd_opt(2023, 12, 25).unwrap()),
            Some(NaiveDate::from_ymd_opt(2024, 7, 4).unwrap()),
            None,
        ];
        let result = formatter.format(&values, Some("N/A"));

        assert_eq!(result, vec!["December 25, 2023", "July 04, 2024", "N/A"]);
    }

    #[test]
    fn test_date_formatting_short_format() {
        use crate::formatter::{DateFormatter, DefaultFormatter};
        use chrono::NaiveDate;

        let formatter = DefaultFormatter {
            format_str: Some("%m/%d/%y".to_string()),
            local_tz: None,
        };
        let values = vec![
            Some(NaiveDate::from_ymd_opt(2023, 12, 25).unwrap()),
            Some(NaiveDate::from_ymd_opt(2024, 1, 5).unwrap()),
            None,
        ];
        let result = formatter.format(&values, Some("--"));

        assert_eq!(result, vec!["12/25/23", "01/05/24", "--"]);
    }

    #[test]
    fn test_timestamp_formatting_default() {
        use crate::formatter::{DefaultFormatter, TimestampFormatter};

        let formatter = DefaultFormatter::default();
        let values = vec![
            Some(
                chrono::NaiveDate::from_ymd_opt(2022, 1, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
            ),
            Some(
                chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
            ),
            None,
        ];
        let result = formatter.format(&values, Some("Unknown"));

        assert_eq!(
            result,
            vec!["2022-01-01 00:00:00", "2024-01-01 00:00:00", "Unknown"]
        );
    }

    #[test]
    fn test_timestamp_formatting_with_format_string() {
        use crate::formatter::{DefaultFormatter, TimestampFormatter};

        let formatter = DefaultFormatter {
            format_str: Some("%Y-%m-%d %H:%M".to_string()),
            local_tz: None,
        };
        let values = vec![
            Some(
                chrono::NaiveDate::from_ymd_opt(2022, 1, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
            ),
            Some(
                chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
            ),
            None,
        ];
        let result = formatter.format(&values, Some("N/A"));

        assert_eq!(result, vec!["2022-01-01 00:00", "2024-01-01 00:00", "N/A"]);
    }

    #[test]
    fn test_timestamptz_formatting_default() {
        use crate::formatter::{DefaultFormatter, TimestamptzFormatter};
        use chrono::DateTime;

        let formatter = DefaultFormatter::default();
        let values = vec![
            Some(DateTime::from_timestamp(1640995200, 0).unwrap()),
            Some(DateTime::from_timestamp(1704067200, 0).unwrap()),
            None,
        ];
        let result = formatter.format(&values, Some("Unknown"));

        assert_eq!(
            result,
            vec![
                "2022-01-01 00:00:00 UTC",
                "2024-01-01 00:00:00 UTC",
                "Unknown"
            ]
        );
    }

    #[test]
    fn test_timestamptz_formatting_with_timezone() {
        use crate::formatter::{DefaultFormatter, TimestamptzFormatter};
        use chrono::DateTime;
        use chrono_tz::Tz;

        let formatter = DefaultFormatter {
            format_str: Some("%Y-%m-%d %H:%M %Z".to_string()),
            local_tz: Some(Tz::America__New_York),
        };
        let values = vec![
            Some(DateTime::from_timestamp(1640995200, 0).unwrap()), // 2022-01-01 00:00:00 UTC
            None,
        ];
        let result = formatter.format(&values, Some("N/A"));

        // UTC midnight becomes 7 PM previous day in New York (EST)
        assert_eq!(result, vec!["2021-12-31 19:00 EST", "N/A"]);
    }

    #[test]
    fn test_formatters_integration_numbers() {
        use arrow::array::Float32Array;

        let coercer = Coercer::default();
        let numbers = Float32Array::from(vec![Some(1234.567), Some(-0.123), None, Some(0.0)]);
        let result = coercer
            .to_string(&(Arc::new(numbers) as ArrayRef), Some("N/A"))
            .unwrap();
        let strings = result.as_vec(4, None);

        assert_eq!(strings, vec!["1234.567", "-0.123", "N/A", "0"]);
    }

    #[test]
    fn test_formatters_integration_dates() {
        use arrow::array::Date32Array;
        use arrow::datatypes::Date32Type;

        let coercer = Coercer::default();
        // Date32 stores days since epoch (1970-01-01)
        let epoch_date =
            Date32Type::from_naive_date(chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
        let test_date =
            Date32Type::from_naive_date(chrono::NaiveDate::from_ymd_opt(2023, 12, 25).unwrap());

        let dates = Date32Array::from(vec![Some(epoch_date), Some(test_date), None]);
        let result = coercer
            .to_string(&(Arc::new(dates) as ArrayRef), Some("Unknown"))
            .unwrap();
        let strings = result.as_vec(3, None);

        assert_eq!(strings, vec!["1970-01-01", "2023-12-25", "1970-01-01"]);
    }

    #[test]
    fn test_formatters_integration_strings() {
        use arrow::array::StringArray;

        let coercer = Coercer::default();
        let strings_array = StringArray::from(vec![Some("hello"), Some("world"), None]);
        let result = coercer
            .to_string(&(Arc::new(strings_array) as ArrayRef), Some("N/A"))
            .unwrap();
        let strings = result.as_vec(3, None);

        assert_eq!(strings, vec!["hello", "world", "N/A"]);
    }

    #[test]
    fn test_formatters_integration_booleans() {
        use arrow::array::BooleanArray;

        let coercer = Coercer::default();
        let bools = BooleanArray::from(vec![Some(true), Some(false), None]);
        let result = coercer
            .to_string(&(Arc::new(bools) as ArrayRef), Some("Unknown"))
            .unwrap();
        let strings = result.as_vec(3, None);

        assert_eq!(strings, vec!["true", "false", "Unknown"]);
    }

    #[test]
    fn test_number_formatting_edge_cases() {
        use crate::formatter::{DefaultFormatter, NumberFormatter};

        let formatter = DefaultFormatter::default();
        let values = vec![
            Some(f32::INFINITY),
            Some(f32::NEG_INFINITY),
            Some(f32::NAN),
            Some(0.0),
            Some(-0.0),
        ];
        let result = formatter.format(&values, Some("N/A"));

        // Check for reasonable string representations
        assert_eq!(result[0], "inf");
        assert_eq!(result[1], "-inf");
        assert_eq!(result[2], "NaN");
        assert_eq!(result[3], "0");
        assert_eq!(result[4], "-0"); // -0.0 displays as -0 in numfmt
    }

    #[test]
    fn test_number_formatting_large_numbers() {
        use crate::formatter::{DefaultFormatter, NumberFormatter};

        let formatter = DefaultFormatter {
            format_str: Some(",.0f".to_string()),
            local_tz: None,
        };
        let values = vec![
            Some(1_000_000.0),
            Some(1_234_567.9),
            Some(-999_999.99),
            None,
        ];
        let result = formatter.format(&values, Some("--"));

        assert_eq!(result, vec!["1,000,000", "1,234,568", "-1,000,000", "--"]);
    }

    #[test]
    fn test_date_formatting_edge_cases() {
        use crate::formatter::{DateFormatter, DefaultFormatter};
        use chrono::NaiveDate;

        let formatter = DefaultFormatter {
            format_str: Some("%Y-%j".to_string()), // Year and day of year
            local_tz: None,
        };
        let values = vec![
            Some(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()), // Leap year start
            Some(NaiveDate::from_ymd_opt(2000, 12, 31).unwrap()), // Leap year end
            Some(NaiveDate::from_ymd_opt(1900, 1, 1).unwrap()), // Non-leap year century
            None,
        ];
        let result = formatter.format(&values, Some("Invalid"));

        assert_eq!(result, vec!["2000-001", "2000-366", "1900-001", "Invalid"]);
    }

    #[test]
    fn test_formatting_empty_arrays() {
        use crate::formatter::{DateFormatter, DefaultFormatter, NumberFormatter};

        let number_formatter = DefaultFormatter::default();
        let date_formatter = DefaultFormatter::default();

        let number_result = NumberFormatter::format(&number_formatter, &[], Some("N/A"));
        let date_result = DateFormatter::format(&date_formatter, &[], Some("N/A"));

        assert_eq!(number_result, Vec::<String>::new());
        assert_eq!(date_result, Vec::<String>::new());
    }
}
