//! Ported from crates/typst-library/src/text/font/mod.rs @ v0.15.1, modified for Avenger.
//!
//! Font handling.
//!
//! avenger: parsed faces live in `self_cell`s that own the font data, in place of upstream's
//! `unsafe` `'static` borrows of it. Instances are cached per world, standing in for comemo's
//! memo of `instantiate`: a world's fonts hold a `Weak` reference to its [`InstanceCache`], so
//! the cache's instances don't keep their fonts in a cycle. Fonts are compared and hashed by
//! identity, since a world creates each font once. Color glyphs (`color.rs`) are not ported.

mod book;
mod exceptions;
mod info;
mod metrics;
mod tag;
mod variant;
mod variations;

pub use self::book::FontBook;
#[allow(
    unused_imports,
    reason = "upstream's re-exports, which the port uses in part"
)]
pub use self::info::{Coverage, FontFlags, FontInfo};
#[allow(
    unused_imports,
    reason = "upstream's re-exports, which the port uses in part"
)]
pub use self::metrics::{
    FontMetrics, LineMetrics, MathConstants, ScriptMetrics, TextEdgeBounds,
    VerticalFontMetric,
};
pub use self::tag::Tag;
pub use self::variant::{FontStretch, FontStyle, FontVariant, FontWeight};
pub use self::variations::{AxisValue, FontAxis, FontVariations, StandardAxes};

use std::cell::OnceCell;
use std::fmt::{self, Debug, Formatter};
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::{Arc, RwLock, Weak};

use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use ttf_parser::{GlyphId, name_id};

use self::exceptions::find_exception;
use self::info::find_name;
use crate::typst_library::layout::{Abs, Em};
use crate::typst_library::text::{BottomEdge, TopEdge};

/// An OpenType font.
///
/// Values of this type are cheap to clone and hash.
#[derive(Clone)]
pub struct Font(Arc<FontInner>);

/// The internal representation of a [`Font`].
struct FontInner {
    /// The font's index in the buffer.
    index: u32,
    /// Metadata about the font.
    info: FontInfo,
    /// The underlying ttf-parser face, together with the raw font data it
    /// borrows from, which is possibly shared with other fonts from the same
    /// collection.
    // avenger: a `self_cell` in place of an `unsafe` `'static` borrow of `data`.
    ttf: OwnedTtf,
    /// The cache of the world that loaded the font, if any.
    instances: Weak<InstanceCache>,
}

self_cell::self_cell!(
    /// A ttf-parser face that owns its data.
    struct OwnedTtf {
        owner: Arc<[u8]>,
        #[covariant]
        dependent: TtfFace,
    }
);

type TtfFace<'a> = ttf_parser::Face<'a>;

self_cell::self_cell!(
    /// A rustybuzz face that owns its data.
    struct OwnedRusty {
        owner: Arc<[u8]>,
        #[covariant]
        dependent: RustyFace,
    }
);

type RustyFace<'a> = rustybuzz::Face<'a>;

impl Font {
    /// Parse a font from data and collection index.
    pub fn new(data: Arc<[u8]>, index: u32) -> Option<Self> {
        Self::new_cached(data, index, Weak::new())
    }

    /// Parse a font whose instances are cached in `instances`.
    // avenger: how a world creates its fonts.
    pub fn new_cached(
        data: Arc<[u8]>,
        index: u32,
        instances: Weak<InstanceCache>,
    ) -> Option<Self> {
        let ttf =
            OwnedTtf::try_new(data, |data| ttf_parser::Face::parse(data, index)).ok()?;
        let info = FontInfo::from_ttf(ttf.borrow_dependent())?;

        Some(Self(Arc::new(FontInner { index, info, ttf, instances })))
    }

    /// Parse all fonts in the given data.
    pub fn iter(data: Arc<[u8]>) -> impl Iterator<Item = Self> {
        let count = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
        (0..count).filter_map(move |index| Self::new(data.clone(), index))
    }

    /// The underlying buffer.
    pub fn data(&self) -> &Arc<[u8]> {
        self.0.ttf.borrow_owner()
    }

    /// The font's index in the buffer.
    pub fn index(&self) -> u32 {
        self.0.index
    }

    /// The font's metadata.
    pub fn info(&self) -> &FontInfo {
        &self.0.info
    }

    /// Determine the font's PostScript name.
    pub fn post_script_name(&self) -> Option<String> {
        find_name(self.0.ttf.borrow_dependent(), name_id::POST_SCRIPT_NAME)
    }

    /// Instantiates the font with specific text properties. The resulting
    /// type allows access to methods that depend on coordinates.
    // avenger: cached in the world's `InstanceCache` instead of memoized.
    pub fn instantiate(
        self,
        variant: FontVariant,
        size: Abs,
        custom: &FontVariations,
    ) -> FontInstance {
        let axes = &self.info().axes;
        let automatic = FontVariations::resolve(axes, variant, size);
        let full = automatic.chain(custom).normalized();
        match self.0.instances.upgrade() {
            Some(cache) => cache.get_or_insert(self, full),
            None => self.instantiate_impl(full),
        }
    }

    /// Instantiates the font with specific variation coordinates. The resulting
    /// type allows access to methods that depend on coordinates.
    fn instantiate_impl(self, variations: FontVariations) -> FontInstance {
        let index = self.index();

        // avenger: the face owns a handle to the data in place of an `unsafe` borrow.
        let rusty = OwnedRusty::new(self.data().clone(), |data| {
            let mut rusty = rustybuzz::Face::from_slice(data, index).unwrap();
            for &(tag, value) in &variations.0 {
                rusty.set_variation(tag.into(), value.0);
            }
            rusty
        });

        let metrics = FontMetrics::from_ttf(rusty.borrow_dependent());

        FontInstance(Arc::new(FontInstanceInner {
            metrics,
            rusty,
            variations,
            font: self,
        }))
    }
}

impl Debug for Font {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "Font({}, {:?})", self.info().family, self.info().variant)
    }
}

// avenger: identity, since a world creates each of its fonts once.
impl Hash for Font {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::ptr::hash(Arc::as_ptr(&self.0), state);
    }
}

impl Eq for Font {}

impl PartialEq for Font {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// The instances of a world's fonts.
// avenger: stands in for comemo's memo of `Font::instantiate`. Instances are kept until the
// cache is full, then the cache starts over.
#[derive(Default)]
pub struct InstanceCache {
    instances: RwLock<FxHashMap<InstanceKey, FontInstance>>,
}

/// A font and its variation coordinates, with the coordinates' bits, which unlike `f32`s are
/// `Eq`.
type InstanceKey = (Font, SmallVec<[(Tag, u32); 1]>);

impl InstanceCache {
    /// The most instances the cache holds. Variable fonts with an optical size axis have an
    /// instance per text size.
    const CAPACITY: usize = 1024;

    /// The instance of `font` with `variations`, built if it isn't cached yet.
    fn get_or_insert(&self, font: Font, variations: FontVariations) -> FontInstance {
        let bits = variations.0.iter().map(|&(tag, value)| (tag, value.0.to_bits()));
        let key = (font, bits.collect());
        if let Some(instance) = self.instances.read().unwrap().get(&key) {
            return instance.clone();
        }
        let instance = key.0.clone().instantiate_impl(variations);
        let mut instances = self.instances.write().unwrap();
        if instances.len() >= Self::CAPACITY {
            instances.clear();
        }
        instances.entry(key).or_insert(instance).clone()
    }
}

/// An OpenType font with fixed variation coordinates.
///
/// Values of this type are cheap to clone and hash.
#[derive(Clone)]
pub struct FontInstance(Arc<FontInstanceInner>);

/// The internal representation of a [`FontInstance`].
struct FontInstanceInner {
    /// The font's metrics.
    metrics: FontMetrics,
    // NOTE: `rusty` references `font`, so it's important for `font` to be
    // dropped after `rusty` or `rusty` will be left dangling while the font is
    // dropped. Fields are dropped in declaration order, so `font` needs to be
    // declared after `rusty`.
    /// The underlying rustybuzz face.
    // avenger: a `self_cell` in place of an `unsafe` `'static` borrow of the font's data.
    rusty: OwnedRusty,
    // The instance's variation coordinates.
    variations: FontVariations,
    /// The underlying font.
    font: Font,
}

impl FontInstance {
    /// The instance's underlying font.
    pub fn font(&self) -> &Font {
        &self.0.font
    }

    /// The instance's variation coordinates.
    pub fn variations(&self) -> &FontVariations {
        &self.0.variations
    }

    /// The font's metrics.
    pub fn metrics(&self) -> &FontMetrics {
        &self.0.metrics
    }

    /// The font's math constants.
    #[inline]
    pub fn math(&self) -> &MathConstants {
        self.0.metrics.math.get_or_init(|| MathConstants::new(self))
    }

    /// The number of font units per one em.
    pub fn units_per_em(&self) -> f64 {
        self.0.metrics.units_per_em
    }

    /// Convert from font units to an em length.
    pub fn to_em(&self, units: impl Into<f64>) -> Em {
        Em::from_units(units, self.units_per_em())
    }

    /// Look up the horizontal advance width of a glyph.
    pub fn x_advance(&self, glyph: u16) -> Option<Em> {
        self.rusty()
            .glyph_hor_advance(GlyphId(glyph))
            .map(|units| self.to_em(units))
    }

    /// Look up the vertical advance width of a glyph.
    pub fn y_advance(&self, glyph: u16) -> Option<Em> {
        self.rusty()
            .glyph_ver_advance(GlyphId(glyph))
            .map(|units| self.to_em(units))
    }

    /// A reference to the underlying `ttf-parser` face.
    pub fn ttf(&self) -> &ttf_parser::Face<'_> {
        self.rusty()
    }

    /// A reference to the underlying `rustybuzz` face.
    pub fn rusty(&self) -> &rustybuzz::Face<'_> {
        self.0.rusty.borrow_dependent()
    }

    /// Resolve the top and bottom edges of text.
    pub fn edges(
        &self,
        top_edge: TopEdge,
        bottom_edge: BottomEdge,
        font_size: Abs,
        bounds: TextEdgeBounds,
    ) -> (Abs, Abs) {
        let cell = OnceCell::new();
        let bbox = |gid, f: fn(ttf_parser::Rect) -> i16| {
            cell.get_or_init(|| self.ttf().glyph_bounding_box(GlyphId(gid)))
                .map(|bbox| self.to_em(f(bbox)).at(font_size))
                .unwrap_or_default()
        };

        let top = match top_edge {
            TopEdge::Metric(metric) => match metric.try_into() {
                Ok(metric) => self.metrics().vertical(metric).at(font_size),
                Err(_) => match bounds {
                    TextEdgeBounds::Zero => Abs::zero(),
                    TextEdgeBounds::Frame(frame) => frame.ascent(),
                    TextEdgeBounds::Glyph(gid) => bbox(gid, |b| b.y_max),
                },
            },
            TopEdge::Length(length) => length.at(font_size),
        };

        let bottom = match bottom_edge {
            BottomEdge::Metric(metric) => match metric.try_into() {
                Ok(metric) => -self.metrics().vertical(metric).at(font_size),
                Err(_) => match bounds {
                    TextEdgeBounds::Zero => Abs::zero(),
                    TextEdgeBounds::Frame(frame) => frame.descent(),
                    TextEdgeBounds::Glyph(gid) => -bbox(gid, |b| b.y_min),
                },
            },
            BottomEdge::Length(length) => -length.at(font_size),
        };

        (top, bottom)
    }
}

impl Deref for FontInstance {
    type Target = Font;

    fn deref(&self) -> &Self::Target {
        self.font()
    }
}

impl Debug for FontInstance {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.debug_struct("FontInstance")
            .field("font", self.font())
            .field("variations", self.variations())
            .finish()
    }
}

impl Hash for FontInstance {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.font.hash(state);
        self.0.variations.hash(state);
    }
}

impl Eq for FontInstance {}

impl PartialEq for FontInstance {
    fn eq(&self, other: &Self) -> bool {
        self.0.font == other.0.font && self.0.variations == other.0.variations
    }
}
