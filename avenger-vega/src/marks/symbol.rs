use crate::error::AvengerVegaError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use crate::marks::values::{CssColorOrGradient, StrokeDashSpec};
use avenger::marks::group::{GroupBounds, SceneGroup};
use avenger::marks::line::LineMark;
use avenger::marks::mark::SceneMark;
use avenger::marks::symbol::{SymbolMark, SymbolShape};
use avenger::marks::value::{ColorOrGradient, EncodingValue, Gradient, StrokeCap, StrokeJoin};
use lyon_extra::euclid::Point2D;
use lyon_extra::parser::{ParserOptions, Source};
use lyon_path::geom::{Box2D, Point, Scale};
use lyon_path::{Path, Winding};
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaSymbolItem {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    pub fill: Option<CssColorOrGradient>,
    pub opacity: Option<f32>,
    pub fill_opacity: Option<f32>,
    pub size: Option<f32>,
    pub shape: Option<String>,
    pub stroke: Option<CssColorOrGradient>,
    pub stroke_width: Option<f32>,
    pub stroke_cap: Option<StrokeCap>,
    pub stroke_join: Option<StrokeJoin>,
    pub stroke_dash: Option<StrokeDashSpec>,
    pub stroke_opacity: Option<f32>,
    pub angle: Option<f32>,
    pub zindex: Option<i32>,
}

impl VegaMarkItem for VegaSymbolItem {}

impl VegaMarkContainer<VegaSymbolItem> {
    pub fn to_scene_graph(&self) -> Result<SceneMark, AvengerVegaError> {
        // Get shape of first item and use that for all items for now
        let first = self.items.first();
        let first_shape = first
            .and_then(|item| item.shape.clone())
            .unwrap_or("circle".to_string());

        // Handle special case of sybols with shape == "stroke". This happens when lines are
        // used in legends. We convert these to a group of regular line marks
        if first_shape == "stroke" {
            // stroke symbols are converted to a group of lines
            let mut line_marks: Vec<SceneMark> = Vec::new();
            for item in &self.items {
                let mut gradients = Vec::<Gradient>::new();
                let width = item.size.unwrap_or(100.0).sqrt();
                let stroke = if let Some(c) = &item.stroke {
                    let base_opacity = item.opacity.unwrap_or(1.0);
                    let stroke_opacity = item.stroke_opacity.unwrap_or(1.0) * base_opacity;
                    c.to_color_or_grad(stroke_opacity, &mut gradients)?
                } else {
                    ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])
                };
                let mark = LineMark {
                    name: "".to_string(),
                    clip: false,
                    len: 2,
                    x: EncodingValue::Array {
                        values: vec![item.x - width / 2.0, item.x + width / 2.0],
                    },
                    y: EncodingValue::Scalar { value: item.y },
                    stroke,
                    stroke_width: item.stroke_width.unwrap_or(1.0),
                    stroke_cap: item.stroke_cap.unwrap_or_default(),
                    stroke_join: item.stroke_join.unwrap_or_default(),
                    stroke_dash: item
                        .stroke_dash
                        .clone()
                        .map(|d| Ok::<Vec<f32>, AvengerVegaError>(d.to_array()?.to_vec()))
                        .transpose()?,
                    gradients,
                    ..Default::default()
                };
                line_marks.push(SceneMark::Line(mark));
            }
            return Ok(SceneMark::Group(SceneGroup {
                name: "symbol_line_legend".to_string(),
                bounds: GroupBounds {
                    x: 0.0,
                    y: 0.0,
                    width: None,
                    height: None,
                },
                marks: line_marks,
                gradients: vec![],
                fill: None,
                stroke: None,
                stroke_width: None,
                stroke_offset: None,
                corner_radius: None,
            }));
        }

        let first_has_stroke = first.map(|item| item.stroke.is_some()).unwrap_or(false);

        // Only include stroke_width if there is a stroke color
        let stroke_width = if first_has_stroke {
            first.and_then(|item| item.stroke_width)
        } else {
            None
        };

        // Init mark with scalar defaults
        let mut mark = SymbolMark {
            stroke_width,
            clip: self.clip,
            ..Default::default()
        };

        if let Some(name) = &self.name {
            mark.name = name.clone();
        }

        // Init vector for each encoding channel
        let mut x = Vec::<f32>::new();
        let mut y = Vec::<f32>::new();
        let mut fill = Vec::<ColorOrGradient>::new();
        let mut size = Vec::<f32>::new();
        let mut stroke = Vec::<ColorOrGradient>::new();
        let mut angle = Vec::<f32>::new();
        let mut zindex = Vec::<i32>::new();
        let mut gradients = Vec::<Gradient>::new();

        let mut shape_strings = Vec::<String>::new();
        let mut shape_index = Vec::<usize>::new();

        // For each item, append explicit values to corresponding vector
        for item in &self.items {
            x.push(item.x);
            y.push(item.y);

            let base_opacity = item.opacity.unwrap_or(1.0);
            if let Some(v) = &item.fill {
                let fill_opacity = item.fill_opacity.unwrap_or(1.0) * base_opacity;
                fill.push(v.to_color_or_grad(fill_opacity, &mut gradients)?);
            }

            if let Some(s) = item.size {
                size.push(s);
            }

            if let Some(v) = &item.stroke {
                let stroke_opacity = item.stroke_opacity.unwrap_or(1.0) * base_opacity;
                stroke.push(v.to_color_or_grad(stroke_opacity, &mut gradients)?);
            }
            if let Some(v) = item.angle {
                angle.push(v);
            }
            if let Some(v) = item.zindex {
                zindex.push(v);
            }
            if let Some(shape_string) = &item.shape {
                if let Some(pos) = shape_strings.iter().position(|s| s == shape_string) {
                    // Already have shape
                    shape_index.push(pos);
                } else {
                    // Add shape
                    let pos = shape_strings.len();
                    shape_strings.push(shape_string.clone());
                    shape_index.push(pos);
                }
            }
        }

        // Override values with vectors
        let len = self.items.len();
        mark.len = len as u32;

        if x.len() == len {
            mark.x = EncodingValue::Array { values: x };
        }
        if y.len() == len {
            mark.y = EncodingValue::Array { values: y };
        }
        if fill.len() == len {
            mark.fill = EncodingValue::Array { values: fill };
        }
        if size.len() == len {
            mark.size = EncodingValue::Array { values: size };
        }
        if stroke.len() == len {
            mark.stroke = EncodingValue::Array { values: stroke };
        }
        if angle.len() == len {
            mark.angle = EncodingValue::Array { values: angle };
        }
        if zindex.len() == len {
            let mut indices: Vec<usize> = (0..len).collect();
            indices.sort_by_key(|i| zindex[*i]);
            mark.indices = Some(indices);
        }
        if shape_index.len() == len {
            mark.shape_index = EncodingValue::Array {
                values: shape_index,
            };
            mark.shapes = shape_strings
                .iter()
                .map(|s| shape_to_path(s))
                .collect::<Result<Vec<SymbolShape>, AvengerVegaError>>()?;
        }

        // Add gradients
        mark.gradients = gradients;

        Ok(SceneMark::Symbol(mark))
    }
}

pub fn shape_to_path(shape: &str) -> Result<SymbolShape, AvengerVegaError> {
    let tan30: f32 = (30.0 * std::f32::consts::PI / 180.0).tan();
    let sqrt3: f32 = 3.0f32.sqrt();

    // See https://github.com/vega/vega/blob/main/packages/vega-scenegraph/src/path/symbols.js
    Ok(match shape.to_ascii_lowercase().as_str() {
        "circle" => SymbolShape::Circle,
        "square" => {
            let mut builder = lyon_path::Path::builder();
            builder.add_rectangle(
                &Box2D::new(Point2D::new(-0.5, -0.5), Point2D::new(0.5, 0.5)),
                Winding::Negative,
            );
            let path = builder.build();
            SymbolShape::Path(path)
        }
        "cross" => {
            let r = 0.5;
            let s = r / 2.5;

            let mut builder = lyon_path::Path::builder().with_svg();
            builder.move_to(Point::new(-r, -s));
            builder.line_to(Point::new(-r, s));
            builder.line_to(Point::new(-s, s));
            builder.line_to(Point::new(-s, r));
            builder.line_to(Point::new(s, r));
            builder.line_to(Point::new(s, s));
            builder.line_to(Point::new(r, s));
            builder.line_to(Point::new(r, -s));
            builder.line_to(Point::new(s, -s));
            builder.line_to(Point::new(s, -r));
            builder.line_to(Point::new(-s, -r));
            builder.line_to(Point::new(-s, -s));
            builder.close();
            SymbolShape::Path(builder.build())
        }
        "diamond" => {
            let r = 0.5;
            let mut builder = lyon_path::Path::builder().with_svg();
            builder.move_to(Point::new(-r, 0.0));
            builder.line_to(Point::new(0.0, -r));
            builder.line_to(Point::new(r, 0.0));
            builder.line_to(Point::new(0.0, r));
            builder.close();
            SymbolShape::Path(builder.build())
        }
        "triangle-up" => {
            let r = 0.5;
            let h = r * sqrt3 / 2.0;
            let mut builder = lyon_path::Path::builder().with_svg();
            builder.move_to(Point::new(0.0, -h));
            builder.line_to(Point::new(-r, h));
            builder.line_to(Point::new(r, h));
            builder.close();
            SymbolShape::Path(builder.build())
        }
        "triangle-down" => {
            let r = 0.5;
            let h = r * sqrt3 / 2.0;
            let mut builder = lyon_path::Path::builder().with_svg();
            builder.move_to(Point::new(0.0, h));
            builder.line_to(Point::new(-r, -h));
            builder.line_to(Point::new(r, -h));
            builder.close();
            SymbolShape::Path(builder.build())
        }
        "triangle-right" => {
            let r = 0.5;
            let h = r * sqrt3 / 2.0;
            let mut builder = lyon_path::Path::builder().with_svg();
            builder.move_to(Point::new(h, 0.0));
            builder.line_to(Point::new(-h, -r));
            builder.line_to(Point::new(-h, r));
            builder.close();
            SymbolShape::Path(builder.build())
        }
        "triangle-left" => {
            let r = 0.5;
            let h = r * sqrt3 / 2.0;
            let mut builder = lyon_path::Path::builder().with_svg();
            builder.move_to(Point::new(-h, 0.0));
            builder.line_to(Point::new(h, -r));
            builder.line_to(Point::new(h, r));
            builder.close();
            SymbolShape::Path(builder.build())
        }
        "arrow" => {
            let r = 0.5;
            let s = r / 7.0;
            let t = r / 2.5;
            let v = r / 8.0;

            let mut builder = lyon_path::Path::builder().with_svg();
            builder.move_to(Point::new(-s, r));
            builder.line_to(Point::new(s, r));
            builder.line_to(Point::new(s, -v));
            builder.line_to(Point::new(t, -v));
            builder.line_to(Point::new(0.0, -r));
            builder.line_to(Point::new(-t, -v));
            builder.line_to(Point::new(-s, -v));
            builder.close();
            SymbolShape::Path(builder.build())
        }
        "wedge" => {
            let r = 0.5;
            let h = r * sqrt3 / 2.0;
            let o = h - r * tan30;
            let b = r / 4.0;

            let mut builder = lyon_path::Path::builder().with_svg();
            builder.move_to(Point::new(0.0, -h - o));
            builder.line_to(Point::new(-b, h - o));
            builder.line_to(Point::new(b, h - o));
            builder.close();
            SymbolShape::Path(builder.build())
        }
        "triangle" => {
            let r = 0.5;
            let h = r * sqrt3 / 2.0;
            let o = h - r * tan30;
            let mut builder = lyon_path::Path::builder().with_svg();
            builder.move_to(Point::new(0.0, -h - o));
            builder.line_to(Point::new(-r, h - o));
            builder.line_to(Point::new(r, h - o));
            builder.close();
            SymbolShape::Path(builder.build())
        }
        _ => {
            // General SVG string
            let path = parse_svg_path(shape)?;

            // - Coordinates are divided by 2 to match Vega
            let path = path.transformed(&Scale::new(0.5));

            SymbolShape::Path(path)
        }
    })
}

pub fn parse_svg_path(path: &str) -> Result<Path, AvengerVegaError> {
    let mut source = Source::new(path.chars());
    let mut parser = lyon_extra::parser::PathParser::new();
    let opts = ParserOptions::DEFAULT;
    let mut builder = lyon_path::Path::builder();
    parser.parse(&opts, &mut source, &mut builder)?;
    Ok(builder.build())
}
