use crate::marks::value::{EncodingValue, ImageAlign, ImageBaseline};
use itertools::izip;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ImageMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub aspect: bool,
    pub smooth: bool,
    pub indices: Option<Vec<usize>>,
    pub zindex: Option<i32>,

    // Encodings
    pub image: EncodingValue<RgbaImage>,
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub width: EncodingValue<f32>,
    pub height: EncodingValue<f32>,
    pub align: EncodingValue<ImageAlign>,
    pub baseline: EncodingValue<ImageBaseline>,
}

impl ImageMark {
    pub fn instances(&self) -> Box<dyn Iterator<Item = ImageMarkInstance> + '_> {
        let n = self.len as usize;
        let inds = self.indices.as_ref();
        Box::new(
            izip!(
                self.image.as_iter(n, inds),
                self.x.as_iter(n, inds),
                self.y.as_iter(n, inds),
                self.width.as_iter(n, inds),
                self.height.as_iter(n, inds),
                self.align.as_iter(n, inds),
                self.baseline.as_iter(n, inds)
            )
            .map(
                |(image, x, y, width, height, align, baseline)| ImageMarkInstance {
                    image: image.clone(),
                    x: *x,
                    y: *y,
                    width: *width,
                    height: *height,
                    align: *align,
                    baseline: *baseline,
                },
            ),
        )
    }
}

impl Default for ImageMark {
    fn default() -> Self {
        let default_instance = ImageMarkInstance::default();
        Self {
            name: "image_mark".to_string(),
            clip: true,
            len: 1,
            aspect: true,
            indices: None,
            smooth: true,
            x: EncodingValue::Scalar {
                value: default_instance.x,
            },
            y: EncodingValue::Scalar {
                value: default_instance.y,
            },
            width: EncodingValue::Scalar {
                value: default_instance.width,
            },
            height: EncodingValue::Scalar {
                value: default_instance.height,
            },
            align: EncodingValue::Scalar {
                value: default_instance.align,
            },
            baseline: EncodingValue::Scalar {
                value: default_instance.baseline,
            },
            image: EncodingValue::Scalar {
                value: default_instance.image,
            },
            zindex: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageMarkInstance {
    pub image: RgbaImage,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub align: ImageAlign,
    pub baseline: ImageBaseline,
}

impl Default for ImageMarkInstance {
    fn default() -> Self {
        Self {
            image: Default::default(),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            align: Default::default(),
            baseline: Default::default(),
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
