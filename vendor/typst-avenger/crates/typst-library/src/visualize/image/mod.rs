//! Image handling is stubbed in the avenger-typst math subset.

use std::fmt::{self, Debug, Formatter};

use crate::diag::{SourceResult, bail};
use crate::engine::Engine;
use crate::foundations::{Bytes, Cast, Packed, Smart, StyleChain, Synthesize, elem};
use crate::introspection::{Locatable, Tagged};
use crate::layout::{Length, Rel, Sizing};
use crate::model::Figurable;
use crate::text::Locale;

#[elem(Locatable, Tagged, Synthesize, Figurable)]
pub struct ImageElem {
    pub width: Smart<Rel<Length>>,
    pub height: Sizing,
    #[default(ImageFit::Cover)]
    pub fit: ImageFit,
    #[internal]
    #[synthesized]
    pub locale: Locale,
}

impl Synthesize for Packed<ImageElem> {
    fn synthesize(&mut self, _: &mut Engine, styles: StyleChain) -> SourceResult<()> {
        self.as_mut().locale = Some(Locale::get_in(styles));
        Ok(())
    }
}

impl Packed<ImageElem> {
    pub fn decode(&self, _: &mut Engine, _: StyleChain) -> SourceResult<Image> {
        bail!(self.span(), "images are not available in avenger-typst math fragments")
    }
}

impl Figurable for Packed<ImageElem> {}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Cast)]
pub enum ImageFit {
    Cover,
    Contain,
    Stretch,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Cast)]
pub enum ImageScaling {
    Smooth,
    Pixelated,
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct Image;

impl Image {
    pub const DEFAULT_DPI: f64 = 72.0;
    pub const USVG_DEFAULT_DPI: f64 = 96.0;

    pub fn plain(_: impl Into<ImageKind>) -> Self {
        Self
    }

    pub fn width(&self) -> f64 {
        1.0
    }

    pub fn height(&self) -> f64 {
        1.0
    }

    pub fn dpi(&self) -> Option<f64> {
        Some(Self::DEFAULT_DPI)
    }
}

impl Debug for Image {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.pad("Image(..)")
    }
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub enum ImageKind {
    Raster(RasterImage),
    Svg(SvgImage),
}

impl From<RasterImage> for ImageKind {
    fn from(image: RasterImage) -> Self {
        Self::Raster(image)
    }
}

impl From<SvgImage> for ImageKind {
    fn from(image: SvgImage) -> Self {
        Self::Svg(image)
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Cast)]
pub enum ExchangeFormat {
    Png,
    Jpg,
    Gif,
    Webp,
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct RasterImage;

impl RasterImage {
    pub fn plain(_: Bytes, _: ExchangeFormat) -> crate::diag::StrResult<Self> {
        Ok(Self)
    }
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct SvgImage;

impl SvgImage {
    pub fn new(_: Bytes) -> crate::diag::StrResult<Self> {
        Ok(Self)
    }
}
