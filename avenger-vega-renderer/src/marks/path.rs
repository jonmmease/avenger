use crate::marks::util::{decode_colors, decode_gradients, zindex_to_indices};
use avenger_scenegraph::error::AvengerError;
use avenger_scenegraph::marks::path::{PathTransform, ScenePathMark as RsPathMark};
use avenger_scenegraph::marks::symbol::parse_svg_path;
use avenger_scenegraph::marks::value::ScalarOrArray;
use gloo_utils::format::JsValueSerdeExt;
use itertools::izip;
use lyon_path::geom::euclid::Vector2D;
use lyon_path::geom::Angle;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsError, JsValue};

#[wasm_bindgen]
pub struct PathMark {
    inner: RsPathMark,
}

impl PathMark {
    pub fn build(self) -> RsPathMark {
        self.inner
    }
}

#[wasm_bindgen]
impl PathMark {
    #[wasm_bindgen(constructor)]
    pub fn new(len: u32, clip: bool, name: Option<String>, zindex: Option<i32>) -> Self {
        Self {
            inner: RsPathMark {
                len,
                clip,
                name: name.unwrap_or_default(),
                zindex,
                ..Default::default()
            },
        }
    }
    pub fn set_zindex(&mut self, zindex: Vec<i32>) {
        self.inner.indices = Some(zindex_to_indices(zindex));
    }

    pub fn set_transform(
        &mut self,
        xs: Vec<f32>,
        ys: Vec<f32>,
        scale_xs: Vec<f32>,
        scale_ys: Vec<f32>,
        angles: Vec<f32>,
    ) {
        let transforms = izip!(xs, ys, scale_xs, scale_ys, angles)
            .map(|(x, y, scale_x, scale_y, angle)| {
                PathTransform::scale(scale_x, scale_y)
                    .then_rotate(Angle::degrees(angle))
                    .then_translate(Vector2D::new(x, y))
            })
            .collect::<Vec<_>>();
        self.inner.transform = ScalarOrArray::Array { values: transforms };
    }

    /// Set path as SVG string
    ///
    /// @param {string[]} path_values
    /// @param {Uint32Array} indices
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_path(&mut self, path_values: JsValue, indices: Vec<usize>) -> Result<(), JsError> {
        let svg_paths: Vec<String> = path_values.into_serde()?;
        let unique_paths = svg_paths
            .iter()
            .map(|s| parse_svg_path(s))
            .collect::<Result<Vec<_>, AvengerError>>()
            .map_err(|_| JsError::new("Failed to parse shapes"))?;

        let paths = indices
            .into_iter()
            .map(|i| unique_paths[i].clone())
            .collect::<Vec<_>>();
        self.inner.path = ScalarOrArray::Array { values: paths };
        Ok(())
    }

    /// Set stroke color.
    ///
    /// @param {string[]} color_values
    /// @param {Uint32Array} indices
    /// @param {Float32Array} opacity
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_stroke(
        &mut self,
        color_values: JsValue,
        indices: Vec<usize>,
        opacity: Vec<f32>,
    ) -> Result<(), JsError> {
        self.inner.stroke = ScalarOrArray::Array {
            values: decode_colors(color_values, indices, opacity)?,
        };
        Ok(())
    }

    /// Set stroke gradient
    ///
    /// @param {(string|object)[]} values
    /// @param {Float32Array} opacity
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_stroke_gradient(
        &mut self,
        values: JsValue,
        opacity: Vec<f32>,
    ) -> Result<(), JsError> {
        self.inner.stroke = ScalarOrArray::Array {
            values: decode_gradients(values, opacity, &mut self.inner.gradients)?,
        };
        Ok(())
    }

    /// Set fill color
    ///
    /// @param {string[]} color_values
    /// @param {Uint32Array} indices
    /// @param {Float32Array} opacity
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_fill(
        &mut self,
        color_values: JsValue,
        indices: Vec<usize>,
        opacity: Vec<f32>,
    ) -> Result<(), JsError> {
        self.inner.fill = ScalarOrArray::Array {
            values: decode_colors(color_values, indices, opacity)?,
        };
        Ok(())
    }

    /// Set fill gradient
    ///
    /// @param {(string|object)[]} values
    /// @param {Float32Array} opacity
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_fill_gradient(&mut self, values: JsValue, opacity: Vec<f32>) -> Result<(), JsError> {
        self.inner.fill = ScalarOrArray::Array {
            values: decode_gradients(values, opacity, &mut self.inner.gradients)?,
        };
        Ok(())
    }

    pub fn set_stroke_width(&mut self, width: Option<f32>) {
        self.inner.stroke_width = width
    }
}
