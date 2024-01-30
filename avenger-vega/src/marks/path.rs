use crate::error::AvengerVegaError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use crate::marks::symbol::parse_svg_path;
use crate::marks::values::CssColorOrGradient;
use avenger::marks::mark::SceneMark;
use avenger::marks::path::{PathMark, PathTransform};
use avenger::marks::value::{ColorOrGradient, EncodingValue, Gradient, StrokeCap, StrokeJoin};
use lyon_extra::euclid::Vector2D;
use lyon_path::geom::Angle;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaPathItem {
    pub path: Option<String>,
    pub stroke_cap: Option<StrokeCap>,
    pub stroke_join: Option<StrokeJoin>,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub scale_x: Option<f32>,
    pub scale_y: Option<f32>,
    pub opacity: Option<f32>,
    pub fill: Option<CssColorOrGradient>,
    pub fill_opacity: Option<f32>,
    pub stroke: Option<CssColorOrGradient>,
    pub stroke_opacity: Option<f32>,
    pub stroke_width: Option<f32>,
    pub angle: Option<f32>,
    pub zindex: Option<i32>,
}

impl VegaMarkItem for VegaPathItem {}

impl VegaMarkContainer<VegaPathItem> {
    pub fn to_scene_graph(&self) -> Result<SceneMark, AvengerVegaError> {
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
        let mut fill = Vec::<ColorOrGradient>::new();
        let mut stroke = Vec::<ColorOrGradient>::new();
        let mut transform = Vec::<PathTransform>::new();
        let mut zindex = Vec::<i32>::new();
        let mut gradients = Vec::<Gradient>::new();

        for item in &self.items {
            if let Some(v) = &item.path {
                path_str.push(v.clone());
            }

            let base_opacity = item.opacity.unwrap_or(1.0);
            if let Some(v) = &item.fill {
                let fill_opacity = item.fill_opacity.unwrap_or(1.0) * base_opacity;
                fill.push(v.to_color_or_grad(fill_opacity, &mut gradients)?);
            }
            if let Some(v) = &item.stroke {
                let stroke_opacity = item.stroke_opacity.unwrap_or(1.0) * base_opacity;
                stroke.push(v.to_color_or_grad(stroke_opacity, &mut gradients)?);
            }

            // Build transform
            if item.x.is_some()
                || item.y.is_some()
                || item.scale_x.is_some()
                || item.scale_y.is_some()
                || item.angle.is_some()
            {
                transform.push(
                    PathTransform::scale(item.scale_x.unwrap_or(1.0), item.scale_y.unwrap_or(1.0))
                        .then_rotate(Angle::degrees(item.angle.unwrap_or(0.0)))
                        .then_translate(Vector2D::new(
                            item.x.unwrap_or(0.0),
                            item.y.unwrap_or(0.0),
                        )),
                )
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
                value: PathTransform::identity(),
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
                .collect::<Result<Vec<_>, AvengerVegaError>>()?;

            mark.path = EncodingValue::Array { values: paths };
        }

        // Add gradients
        mark.gradients = gradients;

        Ok(SceneMark::Path(mark))
    }
}
