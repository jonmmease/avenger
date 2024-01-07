use crate::error::VegaSceneGraphError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use lyon_extra::euclid::Point2D;
use lyon_extra::parser::{ParserOptions, Source};
use lyon_path::geom::{Box2D, Point, Scale};
use lyon_path::Winding;
use serde::{Deserialize, Serialize};
use sg2d::marks::mark::SceneMark;
use sg2d::marks::symbol::{SymbolMark, SymbolShape};
use sg2d::value::EncodingValue;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaSymbolItem {
    pub x: f32,
    pub y: f32,
    pub fill: Option<String>,
    pub opacity: Option<f32>,
    pub fill_opacity: Option<f32>,
    pub size: Option<f32>,
    pub shape: Option<String>,
    pub stroke: Option<String>,
    pub stroke_width: Option<f32>,
    pub stroke_opacity: Option<f32>,
}

impl VegaMarkItem for VegaSymbolItem {}

impl VegaMarkContainer<VegaSymbolItem> {
    pub fn to_scene_graph(&self, origin: [f32; 2]) -> Result<SceneMark, VegaSceneGraphError> {
        // Get shape of first item and use that for all items for now
        let first = self.items.get(0);

        let first_shape = first
            .and_then(|item| item.shape.clone())
            .unwrap_or_else(|| "circle".to_string());

        let first_has_stroke = first.map(|item| item.stroke.is_some()).unwrap_or(false);

        // Only include stroke_width if there is a stroke color
        let stroke_width = if first_has_stroke {
            first.and_then(|item| item.stroke_width)
        } else {
            None
        };

        let first_shape = shape_to_path(&first_shape)?;

        // Init mark with scalar defaults
        let mut mark = SymbolMark {
            shape: first_shape,
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
        let mut fill = Vec::<[f32; 4]>::new();
        let mut size = Vec::<f32>::new();
        let mut stroke = Vec::<[f32; 4]>::new();
        let mut stroke_width = Vec::<f32>::new();

        // For each item, append explicit values to corresponding vector
        for item in &self.items {
            x.push(item.x + origin[0]);
            y.push(item.y + origin[1]);

            if let Some(c) = &item.fill {
                let c = csscolorparser::parse(c)?;
                let fill_opacity = item
                    .fill_opacity
                    .unwrap_or_else(|| item.opacity.unwrap_or(1.0));
                fill.push([c.r as f32, c.g as f32, c.b as f32, fill_opacity])
            }

            if let Some(s) = item.size {
                size.push(s);
            }

            if let Some(c) = &item.stroke {
                let c = csscolorparser::parse(c)?;
                let stroke_opacity = item
                    .fill_opacity
                    .unwrap_or_else(|| item.opacity.unwrap_or(1.0));
                stroke.push([c.r as f32, c.g as f32, c.b as f32, stroke_opacity])
            }

            if let Some(s) = item.stroke_width {
                stroke_width.push(s);
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

        Ok(SceneMark::Symbol(mark))
    }
}

pub fn shape_to_path(shape: &str) -> Result<SymbolShape, VegaSceneGraphError> {
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
            let mut source = Source::new(shape.chars());
            let mut parser = lyon_extra::parser::PathParser::new();
            let opts = ParserOptions::DEFAULT;
            let mut builder = lyon_path::Path::builder();
            parser.parse(&opts, &mut source, &mut builder)?;
            let path = builder.build();

            // - Coordinates are divided by 2 to match Vega
            let path = path.transformed(&Scale::new(0.5));

            SymbolShape::Path(path)
        }
    })
}
