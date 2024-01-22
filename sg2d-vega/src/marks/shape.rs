use crate::error::VegaSceneGraphError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use crate::marks::symbol::parse_svg_path;
use serde::{Deserialize, Serialize};
use sg2d::marks::mark::SceneMark;
use sg2d::marks::path::{PathMark, PathTransform};
use sg2d::marks::value::{EncodingValue, StrokeCap, StrokeJoin};
use std::collections::HashSet;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaShapeItem {
    pub shape: String,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub stroke_cap: Option<StrokeCap>,
    pub stroke_join: Option<StrokeJoin>,
    pub opacity: Option<f32>,
    pub fill: Option<String>,
    pub fill_opacity: Option<f32>,
    pub stroke: Option<String>,
    pub stroke_opacity: Option<f32>,
    pub stroke_width: Option<f32>,
    pub zindex: Option<i32>,
}

impl VegaMarkItem for VegaShapeItem {}

impl VegaMarkContainer<VegaShapeItem> {
    pub fn to_scene_graph(&self, origin: [f32; 2]) -> Result<SceneMark, VegaSceneGraphError> {
        // Get shape of first item and use that for all items for now
        let first = self.items.first();
        let first_has_stroke = first.map(|item| item.stroke.is_some()).unwrap_or(false);
        let stroke_width = if first_has_stroke {
            // Default stroke_width to 1.0 if a stroke is specified
            Some(first.and_then(|item| item.stroke_width).unwrap_or(1.0))
        } else {
            None
        };

        let first_cap = first.and_then(|item| item.stroke_cap).unwrap_or_default();
        let first_join = first.and_then(|item| item.stroke_join).unwrap_or_default();

        // Init mark with scalar defaults
        let mut mark = PathMark {
            clip: self.clip,
            stroke_cap: first_cap,
            stroke_join: first_join,
            stroke_width,
            ..Default::default()
        };

        if let Some(name) = &self.name {
            mark.name = name.clone();
        }

        // Init vector for each encoding channel
        let mut path_str = Vec::<String>::new();
        let mut fill = Vec::<[f32; 4]>::new();
        let mut stroke = Vec::<[f32; 4]>::new();
        let mut transform = Vec::<PathTransform>::new();
        let mut zindex = Vec::<i32>::new();

        for item in &self.items {
            path_str.push(item.shape.clone());

            let base_opacity = item.opacity.unwrap_or(1.0);
            if let Some(c) = &item.fill {
                let c = csscolorparser::parse(c)?;
                let fill_opacity = c.a as f32 * item.fill_opacity.unwrap_or(1.0) * base_opacity;
                fill.push([c.r as f32, c.g as f32, c.b as f32, fill_opacity])
            }
            if let Some(c) = &item.stroke {
                let c = csscolorparser::parse(c)?;
                let stroke_opacity = c.a as f32 * item.stroke_opacity.unwrap_or(1.0) * base_opacity;
                stroke.push([c.r as f32, c.g as f32, c.b as f32, stroke_opacity])
            }

            // Build transform
            if item.x.is_some() || item.y.is_some() {
                transform.push(PathTransform::translation(
                    item.x.unwrap_or(0.0) + origin[0],
                    item.y.unwrap_or(0.0) + origin[1],
                ))
            }

            if let Some(v) = item.zindex {
                zindex.push(v);
            }
        }
        // Override values with vectors
        let len = self.items.len();
        mark.len = len as u32;
        if fill.len() == len {
            mark.fill = EncodingValue::Array { values: fill };
        }
        if stroke.len() == len {
            mark.stroke = EncodingValue::Array { values: stroke };
        }
        if transform.len() == len {
            mark.transform = EncodingValue::Array { values: transform };
        } else {
            mark.transform = EncodingValue::Scalar {
                value: PathTransform::translation(origin[0], origin[1]),
            }
        }
        if zindex.len() == len {
            let mut indices: Vec<usize> = (0..len).collect();
            indices.sort_by_key(|i| zindex[*i]);
            mark.indices = Some(indices);
        }

        // Handle path shape
        let num_unique = HashSet::<&String>::from_iter(path_str.iter()).len();
        if num_unique == 1 {
            // Parse single path and store as a scalar
            let path_str = path_str.first().unwrap();
            mark.path = EncodingValue::Scalar {
                value: parse_svg_path(path_str)?,
            };
        } else {
            // Parse each path individually
            let paths = path_str
                .iter()
                .map(|p| parse_svg_path(p))
                .collect::<Result<Vec<_>, VegaSceneGraphError>>()?;

            mark.path = EncodingValue::Array { values: paths };
        }

        Ok(SceneMark::Path(mark))
    }
}
