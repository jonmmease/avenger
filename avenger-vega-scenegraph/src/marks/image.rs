use std::sync::Arc;

use avenger_common::{
    types::{ImageAlign, ImageBaseline},
    value::ScalarOrArray,
};
use avenger_image::{make_image_fetcher, RgbaImage};
use avenger_scenegraph::marks::{
    image::{SceneImageMark, SceneImageSource},
    mark::SceneMark,
};
use serde::{Deserialize, Serialize};

use crate::{
    error::AvengerVegaError,
    marks::mark::{VegaMarkContainer, VegaMarkItem},
};

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

        let mut fetcher = None;

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

            // Inline images also work when HTTP fetching is disabled.
            if fetcher.is_none() && (url.starts_with("http://") || url.starts_with("https://")) {
                fetcher = Some(make_image_fetcher()?);
            }
            let image = RgbaImage::from_str(&url, fetcher.clone())?;
            let img_width = image.width;
            let img_height = image.height;
            images.push(image);

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
            interactive: self.interactive,
            clip: self.clip || force_clip,
            len: self.items.len() as u32,
            aspect,
            smooth,
            align: ScalarOrArray::new_array(align),
            baseline: ScalarOrArray::new_array(baseline),
            image: ScalarOrArray::new_array(
                images.into_iter().map(SceneImageSource::Inline).collect(),
            ),
            x: ScalarOrArray::new_array(x),
            y: ScalarOrArray::new_array(y),
            width: ScalarOrArray::new_array(width),
            height: ScalarOrArray::new_array(height),
            unavailable_policy: Default::default(),
            indices,
            zindex: self.zindex,
            tile_texture_size: None,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_inline_png_with_intrinsic_and_explicit_dimensions() {
        // Two pixels: opaque red and half-transparent green.
        let url = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAYAAAD0In+KAAAAD0lEQVR4nGP4z8DwHwgbABB5A359Y87XAAAAAElFTkSuQmCC";
        let container = VegaMarkContainer {
            items: vec![
                VegaImageItem {
                    url: url.to_string(),
                    ..Default::default()
                },
                VegaImageItem {
                    url: url.to_string(),
                    width: Some(20.0),
                    height: Some(10.0),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let SceneMark::Image(mark) = container.to_scene_graph(false).unwrap() else {
            panic!("expected an image mark");
        };
        assert_eq!(mark.width.as_vec(2, None), vec![2.0, 20.0]);
        assert_eq!(mark.height.as_vec(2, None), vec![1.0, 10.0]);
        for source in mark.image.as_iter(2, None) {
            let SceneImageSource::Inline(image) = source else {
                panic!("expected decoded inline pixels");
            };
            assert_eq!((image.width, image.height), (2, 1));
            assert_eq!(image.data, [255, 0, 0, 255, 0, 255, 0, 128]);
        }
    }
}
