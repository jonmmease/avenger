use std::sync::Arc;

use avenger_common::types::{ImageAlign, ImageBaseline};
use avenger_common::value::ScalarOrArray;
use avenger_image::RgbaImage;
use itertools::izip;
use lyon_path::Path;
use serde::{Deserialize, Serialize};

use super::mark::SceneMark;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneImageMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub aspect: bool,
    pub smooth: bool,
    pub image: ScalarOrArray<RgbaImage>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub width: ScalarOrArray<f32>,
    pub height: ScalarOrArray<f32>,
    pub align: ScalarOrArray<ImageAlign>,
    pub baseline: ScalarOrArray<ImageBaseline>,
    pub indices: Option<Arc<Vec<usize>>>,
    pub zindex: Option<i32>,
}

impl SceneImageMark {
    pub fn image_iter(&self) -> Box<dyn Iterator<Item = &RgbaImage> + '_> {
        self.image.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn width_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.width.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn height_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.height
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn align_iter(&self) -> Box<dyn Iterator<Item = &ImageAlign> + '_> {
        self.align.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn baseline_iter(&self) -> Box<dyn Iterator<Item = &ImageBaseline> + '_> {
        self.baseline
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn indices_iter(&self) -> Box<dyn Iterator<Item = usize> + '_> {
        if let Some(indices) = self.indices.as_ref() {
            Box::new(indices.iter().cloned())
        } else {
            Box::new((0..self.len as usize).into_iter())
        }
    }

    pub fn transformed_path_iter(&self, origin: [f32; 2]) -> Box<dyn Iterator<Item = Path> + '_> {
        Box::new(
            izip!(
                self.image_iter(),
                self.x_iter(),
                self.y_iter(),
                self.width_iter(),
                self.height_iter(),
                self.baseline_iter(),
                self.align_iter(),
            )
            .map(move |(img, x, y, width, height, baseline, align)| {
                let x = *x + origin[0];
                let y = *y + origin[1];

                // Compute image left
                let left = match *align {
                    ImageAlign::Left => x,
                    ImageAlign::Center => x - *width / 2.0,
                    ImageAlign::Right => x - *width,
                };

                // Compute image top
                let top = match *baseline {
                    ImageBaseline::Top => y,
                    ImageBaseline::Middle => y - *height / 2.0,
                    ImageBaseline::Bottom => y - *height,
                };

                // Adjust position and dimensions if aspect ratio should be preserved
                let (left, top, width, height) = if self.aspect {
                    let img_aspect = img.width as f32 / img.height as f32;
                    let outline_aspect = *width / *height;
                    if img_aspect > outline_aspect {
                        // image is wider than the box, so we scale
                        // image to box width and center vertically
                        let aspect_height = *width / img_aspect;
                        let aspect_top = top + (*height - aspect_height) / 2.0;
                        (left, aspect_top, *width, aspect_height)
                    } else if img_aspect < outline_aspect {
                        // image is taller than the box, so we scale
                        // image to box height an center horizontally
                        let aspect_width = *height * img_aspect;
                        let aspect_left = left + (*width - aspect_width) / 2.0;
                        (aspect_left, top, aspect_width, *height)
                    } else {
                        (left, top, *width, *height)
                    }
                } else {
                    (left, top, *width, *height)
                };

                // Create rect path
                let mut path_builder = Path::builder();
                path_builder.begin(lyon_path::math::point(left, top));
                path_builder.line_to(lyon_path::math::point(left + width, top));
                path_builder.line_to(lyon_path::math::point(left + width, top + height));
                path_builder.line_to(lyon_path::math::point(left, top + height));
                path_builder.close();
                path_builder.build()
            }),
        )
    }
}

impl Default for SceneImageMark {
    fn default() -> Self {
        Self {
            name: "image_mark".to_string(),
            clip: true,
            len: 1,
            aspect: true,
            indices: None,
            smooth: true,
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            width: ScalarOrArray::new_scalar(0.0),
            height: ScalarOrArray::new_scalar(0.0),
            align: ScalarOrArray::new_scalar(Default::default()),
            baseline: ScalarOrArray::new_scalar(Default::default()),
            image: ScalarOrArray::new_scalar(Default::default()),
            zindex: None,
        }
    }
}

impl From<SceneImageMark> for SceneMark {
    fn from(mark: SceneImageMark) -> Self {
        SceneMark::Image(Arc::new(mark))
    }
}
