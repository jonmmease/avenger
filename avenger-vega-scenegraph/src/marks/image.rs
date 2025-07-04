use std::sync::Arc;

use crate::error::AvengerVegaError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};

use avenger_common::types::{ImageAlign, ImageBaseline};
use avenger_common::value::ScalarOrArray;
use avenger_image::{make_image_fetcher, RgbaImage};
use avenger_scenegraph::marks::image::SceneImageMark;
use avenger_scenegraph::marks::mark::SceneMark;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaImageItem {
    pub url: String,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    #[serde(default = "default_true")]
    pub aspect: bool,
    #[serde(default = "default_true")]
    pub smooth: bool,
    #[serde(default)]
    pub align: ImageAlign,
    #[serde(default)]
    pub baseline: ImageBaseline,
    pub zindex: Option<i32>,
}

fn default_true() -> bool {
    true
}

impl VegaMarkItem for VegaImageItem {}

impl VegaMarkContainer<VegaImageItem> {
    pub fn to_scene_graph(&self, force_clip: bool) -> Result<SceneMark, AvengerVegaError> {
        let name = self
            .name
            .clone()
            .unwrap_or_else(|| "image_mark".to_string());

        let first = self.items.first();
        let aspect = first.map(|f| f.aspect).unwrap_or(true);
        let smooth = first.map(|f| f.smooth).unwrap_or(true);

        let mut x: Vec<f32> = Vec::new();
        let mut y: Vec<f32> = Vec::new();
        let mut width: Vec<f32> = Vec::new();
        let mut height: Vec<f32> = Vec::new();
        let mut align: Vec<ImageAlign> = Vec::new();
        let mut baseline: Vec<ImageBaseline> = Vec::new();
        let mut images: Vec<RgbaImage> = Vec::new();
        let mut zindex = Vec::<i32>::new();

        // let client = Client::new();
        let fetcher = make_image_fetcher()?;

        for item in &self.items {
            x.push(item.x.unwrap_or(0.0));
            y.push(item.y.unwrap_or(0.0));
            align.push(item.align);
            baseline.push(item.baseline);

            // load image
            let url = if item.url.starts_with("data/") {
                // built-in vega dataset
                format!("https://vega.github.io/vega-datasets/{}", &item.url)
            } else {
                item.url.clone()
            };

            let diffuse_image = fetcher.fetch_image(&url)?;
            let rgba_img = diffuse_image.to_rgba8();
            let img_width = rgba_img.width();
            let img_height = rgba_img.height();
            images.push(RgbaImage::from_image(&rgba_img));

            // Push width/height
            width.push(item.width.unwrap_or(img_width as f32));
            height.push(item.height.unwrap_or(img_height as f32));

            if let Some(v) = item.zindex {
                zindex.push(v);
            }
        }

        let len = self.items.len();

        let indices = if zindex.len() == len {
            let mut indices: Vec<usize> = (0..len).collect();
            indices.sort_by_key(|i| zindex[*i]);
            Some(Arc::new(indices))
        } else {
            None
        };

        Ok(SceneMark::Image(Arc::new(SceneImageMark {
            name,
            clip: self.clip || force_clip,
            len: self.items.len() as u32,
            aspect,
            smooth,
            align: ScalarOrArray::new_array(align),
            baseline: ScalarOrArray::new_array(baseline),
            image: ScalarOrArray::new_array(images),
            x: ScalarOrArray::new_array(x),
            y: ScalarOrArray::new_array(y),
            width: ScalarOrArray::new_array(width),
            height: ScalarOrArray::new_array(height),
            indices,
            zindex: self.zindex,
        })))
    }
}
