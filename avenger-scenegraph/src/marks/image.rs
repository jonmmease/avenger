use crate::marks::value::{ScalarOrArray, ImageAlign, ImageBaseline};
use serde::{Deserialize, Serialize};

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
    pub indices: Option<Vec<usize>>,
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
            x: ScalarOrArray::Scalar { value: 0.0 },
            y: ScalarOrArray::Scalar { value: 0.0 },
            width: ScalarOrArray::Scalar { value: 0.0 },
            height: ScalarOrArray::Scalar { value: 0.0 },
            align: ScalarOrArray::Scalar {
                value: Default::default(),
            },
            baseline: ScalarOrArray::Scalar {
                value: Default::default(),
            },
            image: ScalarOrArray::Scalar {
                value: Default::default(),
            },
            zindex: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl RgbaImage {
    pub fn to_image(&self) -> Option<image::RgbaImage> {
        image::RgbaImage::from_raw(self.width, self.height, self.data.clone())
    }

    pub fn from_image(img: &image::RgbaImage) -> Self {
        Self {
            width: img.width(),
            height: img.height(),
            data: img.to_vec(),
        }
    }
}
