use crate::error::VegaSceneGraphError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use lyon_extra::parser::{ParserOptions, Source};
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
    pub fill_opacity: Option<f32>,
    pub size: Option<f32>,
    pub shape: Option<String>,
}

impl VegaMarkItem for VegaSymbolItem {}

impl VegaMarkContainer<VegaSymbolItem> {
    pub fn to_scene_graph(&self, origin: [f32; 2]) -> Result<SceneMark, VegaSceneGraphError> {
        // Get shape of first item and use that for all items for now
        let first_shape = self
            .items
            .get(0)
            .and_then(|item| item.shape.clone())
            .unwrap_or_else(|| "circle".to_string());

        let first_shape = match first_shape.to_ascii_lowercase().as_str() {
            "circle" => SymbolShape::Circle,
            "square" => SymbolShape::Square,
            "cross" => SymbolShape::Cross,
            "diamond" => SymbolShape::Diamond,
            "triangle-up" => SymbolShape::TriangleUp,
            "triangle-down" => SymbolShape::TriangleDown,
            "triangle-right" => SymbolShape::TriangleRight,
            "triangle-left" => SymbolShape::TriangleLeft,
            "arrow" => SymbolShape::Arrow,
            "wedge" => SymbolShape::Wedge,
            "triangle" => SymbolShape::Triangle,
            _ => {
                let mut source = Source::new(first_shape.chars());
                let mut parser = lyon_extra::parser::PathParser::new();
                let opts = ParserOptions::DEFAULT;
                let mut builder = lyon_path::Path::builder();
                parser.parse(&opts, &mut source, &mut builder)?;
                let path = builder.build();
                SymbolShape::Path(path)
            }
        };

        // Init mark with scalar defaults
        let mut mark = SymbolMark {
            shape: first_shape,
            clip: self.clip,
            ..Default::default()
        };

        if let Some(name) = &self.name {
            mark.name = name.clone();
        }

        // Init vector for each encoding channel
        let mut x = Vec::<f32>::new();
        let mut y = Vec::<f32>::new();
        let mut fill = Vec::<[f32; 3]>::new();
        let mut size = Vec::<f32>::new();

        // For each item, append explicit values to corresponding vector
        for item in &self.items {
            x.push(item.x + origin[0]);
            y.push(item.y + origin[1]);

            if let Some(c) = &item.fill {
                let c = csscolorparser::parse(c)?;
                fill.push([c.r as f32, c.g as f32, c.b as f32])
            }

            if let Some(s) = item.size {
                size.push(s);
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

        Ok(SceneMark::Symbol(mark))
    }
}
