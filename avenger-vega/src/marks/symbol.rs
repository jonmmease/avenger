use crate::error::AvengerVegaError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};
use crate::marks::values::{CssColorOrGradient, StrokeDashSpec};
use avenger_common::types::{ColorOrGradient, Gradient, StrokeCap, StrokeJoin, SymbolShape};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::line::SceneLineMark;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use lyon_extra::parser::ParseError;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaSymbolItem {
    pub x: Option<f32>,
    pub y: Option<f32>,
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
    pub fn to_scene_graph(&self, force_clip: bool) -> Result<SceneMark, AvengerVegaError> {
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
                let x = item.x.unwrap_or(0.0);
                let y = item.y.unwrap_or(0.0);
                let mark = SceneLineMark {
                    name: "".to_string(),
                    clip: false,
                    zindex: self.zindex,
                    len: 2,
                    x: ScalarOrArray::new_array(vec![x - width / 2.0, x + width / 2.0]),
                    y: ScalarOrArray::new_scalar(y),
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
                origin: [0.0, 0.0],
                clip: Clip::None,
                marks: line_marks,
                gradients: vec![],
                fill: None,
                stroke: None,
                stroke_width: None,
                stroke_offset: None,
                zindex: None,
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
        let mut mark = SceneSymbolMark {
            stroke_width,
            clip: self.clip || force_clip,
            zindex: self.zindex,
            ..Default::default()
        };

        if let Some(name) = &self.name {
            mark.name = name.clone();
        }

        // Override values with vectors
        let len = self.items.len();
        mark.len = len as u32;

        // Init vector for each encoding channel
        let mut x = Vec::<f32>::with_capacity(len);
        let mut y = Vec::<f32>::with_capacity(len);
        let mut fill = Vec::<ColorOrGradient>::with_capacity(len);
        let mut size = Vec::<f32>::with_capacity(len);
        let mut stroke = Vec::<ColorOrGradient>::with_capacity(len);
        let mut angle = Vec::<f32>::with_capacity(len);
        let mut zindex = Vec::<i32>::with_capacity(len);
        let mut gradients = Vec::<Gradient>::with_capacity(len);

        let mut shape_strings = Vec::<String>::with_capacity(len);
        let mut shape_index = Vec::<usize>::with_capacity(len);

        // For each item, append explicit values to corresponding vector
        for item in &self.items {
            x.push(item.x.unwrap_or(0.0));
            y.push(item.y.unwrap_or(0.0));

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

        if x.len() == len {
            mark.x = ScalarOrArray::new_array(x);
        }
        if y.len() == len {
            mark.y = ScalarOrArray::new_array(y);
        }
        if fill.len() == len {
            mark.fill = ScalarOrArray::new_array(fill);
        }
        if size.len() == len {
            mark.size = ScalarOrArray::new_array(size);
        }
        if stroke.len() == len {
            mark.stroke = ScalarOrArray::new_array(stroke);
        }
        if angle.len() == len {
            mark.angle = ScalarOrArray::new_array(angle);
        }
        if zindex.len() == len {
            let mut indices: Vec<usize> = (0..len).collect();
            indices.sort_by_key(|i| zindex[*i]);
            mark.indices = Some(Arc::new(indices));
        }
        if shape_index.len() == len {
            mark.shape_index = ScalarOrArray::new_array(shape_index);
            mark.shapes = shape_strings
                .iter()
                .map(|s| SymbolShape::from_vega_str(s))
                .collect::<Result<Vec<SymbolShape>, ParseError>>()?;
        }

        // Add gradients
        mark.gradients = gradients;

        Ok(SceneMark::Symbol(mark))
    }
}
