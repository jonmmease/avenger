use super::mark::SceneMark;
use avenger_common::types::{
    ColorOrGradient, Gradient, LinearScaleAdjustment, PathTransform, SymbolShape,
};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use itertools::izip;
use lyon_extra::euclid::Vector2D;
use lyon_path::geom::Angle;
use lyon_path::Path;
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneSymbolMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub shapes: Vec<SymbolShape>,
    pub stroke_width: Option<f32>,
    pub shape_index: ScalarOrArray<usize>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub fill: ScalarOrArray<ColorOrGradient>,
    pub size: ScalarOrArray<f32>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub angle: ScalarOrArray<f32>,
    pub indices: Option<Arc<Vec<usize>>>,
    pub zindex: Option<i32>,
    pub x_adjustment: Option<LinearScaleAdjustment>,
    pub y_adjustment: Option<LinearScaleAdjustment>,
}

impl Hash for SceneSymbolMark {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.clip.hash(state);
        self.len.hash(state);
        self.gradients.hash(state);
        self.shapes.hash(state);
        if let Some(stroke_width) = self.stroke_width {
            OrderedFloat(stroke_width).hash(state);
        } else {
            OrderedFloat(0.0).hash(state);
        }
        self.x.hash(state);
        self.y.hash(state);
        self.fill.hash(state);
        self.size.hash(state);
        self.stroke.hash(state);
        self.angle.hash(state);
        self.indices.hash(state);
        self.zindex.hash(state);
        self.x_adjustment.hash(state);
        self.y_adjustment.hash(state);
    }
}

impl SceneSymbolMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = f32> + '_> {
        if let Some(adjustment) = self.x_adjustment {
            let scale = adjustment.scale;
            let offset = adjustment.offset;
            Box::new(
                self.x
                    .as_iter(self.len as usize, self.indices.as_ref())
                    .map(move |x| scale * x + offset),
            )
        } else {
            self.x
                .as_iter_owned(self.len as usize, self.indices.as_ref())
        }
    }

    pub fn x_vec(&self) -> Vec<f32> {
        self.x_iter().collect()
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item = f32> + '_> {
        if let Some(adjustment) = self.y_adjustment {
            let scale = adjustment.scale;
            let offset = adjustment.offset;
            Box::new(
                self.y
                    .as_iter(self.len as usize, self.indices.as_ref())
                    .map(move |y| scale * y + offset),
            )
        } else {
            self.y
                .as_iter_owned(self.len as usize, self.indices.as_ref())
        }
    }

    pub fn y_vec(&self) -> Vec<f32> {
        self.y_iter().collect()
    }

    pub fn fill_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.fill.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn fill_vec(&self) -> Vec<ColorOrGradient> {
        self.fill.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn size_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.size.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn size_vec(&self) -> Vec<f32> {
        self.size.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.stroke
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_vec(&self) -> Vec<ColorOrGradient> {
        self.stroke.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn angle_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.angle.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn angle_vec(&self) -> Vec<f32> {
        self.angle.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn shape_index_iter(&self) -> Box<dyn Iterator<Item = &usize> + '_> {
        self.shape_index
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn shape_index_vec(&self) -> Vec<usize> {
        self.shape_index
            .as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn indices_iter(&self) -> Box<dyn Iterator<Item = usize> + '_> {
        if let Some(indices) = self.indices.as_ref() {
            Box::new(indices.iter().cloned())
        } else {
            Box::new(0..self.len as usize)
        }
    }

    pub fn transformed_path_iter(&self, origin: [f32; 2]) -> Box<dyn Iterator<Item = Path> + '_> {
        let paths = self.shapes.iter().map(|s| s.as_path()).collect::<Vec<_>>();
        Box::new(
            izip!(
                self.x_iter(),
                self.y_iter(),
                self.size_iter(),
                self.angle_iter(),
                self.shape_index_iter()
            )
            .map(move |(x, y, size, angle, shape_idx)| {
                let scale = size.sqrt();
                let angle = Angle::degrees(*angle);
                let transform = PathTransform::scale(scale, scale)
                    .then_rotate(angle)
                    .then_translate(Vector2D::new(x + origin[0], y + origin[1]));

                paths[*shape_idx].as_ref().clone().transformed(&transform)
            }),
        )
    }

    pub fn max_size(&self) -> f32 {
        match self.size.value() {
            ScalarOrArrayValue::Scalar(size) => *size,
            ScalarOrArrayValue::Array(values) => values
                .iter()
                .max_by(|a, b| a.partial_cmp(b).unwrap())
                .cloned()
                .unwrap_or(1.0),
        }
    }

    pub fn single_symbol_mark(&self, index: usize) -> SceneSymbolMark {
        let mut mark = self.clone();
        mark.len = 1;
        mark.indices = Some(Arc::new(vec![index]));
        mark
    }
}

impl Default for SceneSymbolMark {
    fn default() -> Self {
        Self {
            name: "".to_string(),
            clip: true,
            shapes: vec![Default::default()],
            stroke_width: None,
            len: 1,
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            shape_index: ScalarOrArray::new_scalar(0),
            fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            size: ScalarOrArray::new_scalar(20.0),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            angle: ScalarOrArray::new_scalar(0.0),
            indices: None,
            gradients: vec![],
            zindex: None,
            x_adjustment: None,
            y_adjustment: None,
        }
    }
}

impl From<SceneSymbolMark> for SceneMark {
    fn from(mark: SceneSymbolMark) -> Self {
        SceneMark::Symbol(mark)
    }
}
