use crate::marks::util::zindex_to_indices;
use avenger_scenegraph::marks::image::{RgbaImage, SceneImageMark as RsImageMark};
use avenger_scenegraph::marks::value::{ImageAlign, ImageBaseline, ScalarOrArray};
use gloo_utils::format::JsValueSerdeExt;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsError, JsValue};

#[wasm_bindgen]
pub struct ImageMark {
    inner: RsImageMark,
}

impl ImageMark {
    pub fn build(self) -> RsImageMark {
        self.inner
    }
}

#[wasm_bindgen]
impl ImageMark {
    #[wasm_bindgen(constructor)]
    pub fn new(len: u32, clip: bool, name: Option<String>, zindex: Option<i32>) -> Self {
        Self {
            inner: RsImageMark {
                len,
                clip,
                name: name.unwrap_or_default(),
                zindex,
                ..Default::default()
            },
        }
    }

    pub fn set_smooth(&mut self, smooth: bool) {
        self.inner.smooth = smooth;
    }

    pub fn set_aspect(&mut self, aspect: bool) {
        self.inner.aspect = aspect;
    }

    pub fn set_zindex(&mut self, zindex: Vec<i32>) {
        self.inner.indices = Some(zindex_to_indices(zindex));
    }

    pub fn set_xy(&mut self, x: Vec<f32>, y: Vec<f32>) {
        self.inner.x = ScalarOrArray::Array { values: x };
        self.inner.y = ScalarOrArray::Array { values: y };
    }

    pub fn set_width(&mut self, width: Vec<f32>) {
        self.inner.width = ScalarOrArray::Array { values: width };
    }

    pub fn set_height(&mut self, height: Vec<f32>) {
        self.inner.height = ScalarOrArray::Array { values: height };
    }

    /// Set alignment
    ///
    /// @param {("left"|"center"|"right")[]} align_values
    /// @param {Uint32Array} indices
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_align(&mut self, align_values: JsValue, indices: Vec<usize>) -> Result<(), JsError> {
        let align_values: Vec<ImageAlign> = align_values.into_serde()?;
        let values = indices
            .iter()
            .map(|ind| align_values[*ind].clone())
            .collect::<Vec<_>>();
        self.inner.align = ScalarOrArray::Array { values };
        Ok(())
    }

    /// Set alignment
    ///
    /// @param {("alphabetic"|"top"|"middle"|"bottom"|"line-top"|"line-bottom")[]} baseline_values
    /// @param {Uint32Array} indices
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_baseline(
        &mut self,
        baseline_values: JsValue,
        indices: Vec<usize>,
    ) -> Result<(), JsError> {
        let baseline_values: Vec<ImageBaseline> = baseline_values.into_serde()?;
        let values = indices
            .iter()
            .map(|ind| baseline_values[*ind].clone())
            .collect::<Vec<_>>();
        self.inner.baseline = ScalarOrArray::Array { values };
        Ok(())
    }

    /// Set image
    ///
    /// @typedef {Object} RgbaImage
    /// @property {number} width - The width of the image in pixels.
    /// @property {number} height - The height of the image in pixels.
    /// @property {Uint8Array} data - The raw byte data of the image.
    ///
    /// @param {RgbaImage[]} images
    #[wasm_bindgen(skip_jsdoc)]
    pub fn set_image(&mut self, images: JsValue) -> Result<(), JsError> {
        // Use serde_wasm_bindgen instead of gloo_utils to supported
        // nested struct
        let images: Vec<RgbaImage> = serde_wasm_bindgen::from_value(images)?;
        self.inner.image = ScalarOrArray::Array { values: images };
        Ok(())
    }
}
