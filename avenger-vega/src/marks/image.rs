use crate::error::AvengerVegaError;
use crate::marks::mark::{VegaMarkContainer, VegaMarkItem};

use crate::image::make_image_fetcher;
use avenger::marks::image::{ImageMark, RgbaImage};
use avenger::marks::mark::SceneMark;
use avenger::marks::value::{EncodingValue, ImageAlign, ImageBaseline};
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaImageItem {
    pub url: String,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
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
    pub fn to_scene_graph(&self) -> Result<SceneMark, AvengerVegaError> {
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
            x.push(item.x);
            y.push(item.y);
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
            Some(indices)
        } else {
            None
        };

        Ok(SceneMark::Image(Box::new(ImageMark {
            name,
            clip: self.clip,
            len: self.items.len() as u32,
            aspect,
            smooth,
            align: EncodingValue::Array { values: align },
            baseline: EncodingValue::Array { values: baseline },
            image: EncodingValue::Array { values: images },
            x: EncodingValue::Array { values: x },
            y: EncodingValue::Array { values: y },
            width: EncodingValue::Array { values: width },
            height: EncodingValue::Array { values: height },
            indices,
        })))
    }
}
