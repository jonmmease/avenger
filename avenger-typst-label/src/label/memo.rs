//! Memos of labels' boxes and rasters, and the keys that tell labels apart.

use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, MutexGuard};

#[cfg(feature = "raster")]
use avenger_color::AbsoluteColor;

use super::options::{
    Em, Label, LabelLimits, LabelLineHeight, LabelOptions, LabelSource, LabelWidth,
    MathStyle, TextStyle,
};
use super::params::{LabelParamValue, LabelParams};
use crate::typst_library::text::{FontStyle, FontWeight, Lang, Region};

/// Memoized values, shared by an engine's clones. A full memo empties: labels repeat from
/// frame to frame, so the next frame refills it with the labels in use.
#[derive(Debug)]
pub(crate) struct Memo<K, V> {
    entries: Arc<Mutex<HashMap<K, V>>>,
    capacity: usize,
}

impl<K, V> Clone for Memo<K, V> {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            capacity: self.capacity,
        }
    }
}

impl<K: Eq + Hash, V: Clone> Memo<K, V> {
    pub(crate) fn new(capacity: usize) -> Self {
        Self { entries: Default::default(), capacity }
    }

    /// The value for a key, if the memo has one.
    pub(crate) fn get<Q: Eq + Hash + ?Sized>(&self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
    {
        self.lock().get(key).cloned()
    }

    /// The value for a key, made and remembered if the memo has none.
    pub(crate) fn get_or_try_insert<E>(
        &self,
        key: K,
        make: impl FnOnce() -> Result<V, E>,
    ) -> Result<V, E> {
        if let Some(value) = self.get(&key) {
            return Ok(value);
        }
        let value = make()?;
        self.insert(key, value.clone());
        Ok(value)
    }

    pub(crate) fn insert(&self, key: K, value: V) {
        let mut entries = self.lock();
        if entries.len() >= self.capacity {
            entries.clear();
        }
        entries.insert(key, value);
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<K, V>> {
        self.entries.lock().expect("label memo lock poisoned")
    }
}

/// What sets a label's layout: labels with equal keys lay out alike.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct LabelKey {
    source: String,
    markup: bool,
    options: OptionsKey,
    /// The values of the params the source refers to, by name: none for a missing one.
    params: Vec<(String, Option<ParamKey>)>,
}

impl LabelKey {
    /// The key of a label whose markup refers to `referenced` of `params`.
    pub(crate) fn new(
        label: &Label,
        referenced: &[String],
        params: &LabelParams,
    ) -> Self {
        let (source, markup) = match label.source {
            LabelSource::Text(source) => (source, false),
            LabelSource::Markup(source) => (source, true),
        };
        Self {
            source: source.to_owned(),
            markup,
            options: OptionsKey::new(&label.options),
            params: referenced
                .iter()
                .map(|name| (name.clone(), params.get(name).cloned().map(ParamKey)))
                .collect(),
        }
    }
}

/// What a label's raster depends on: what sets its layout, its fills and the scale.
#[cfg(feature = "raster")]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextRasterKey {
    label: LabelKey,
    fill: ColorKey,
    math_fill: Option<ColorKey>,
    scale: u32,
}

#[cfg(feature = "raster")]
impl TextRasterKey {
    pub(crate) fn new(label: LabelKey, options: &LabelOptions, scale: f32) -> Self {
        Self {
            label,
            fill: ColorKey::from(&options.text.fill),
            math_fill: options.math.fill.as_ref().map(ColorKey::from),
            scale: scale.to_bits(),
        }
    }
}

/// A label's options with their distances as bits, but for its fills, which don't change
/// its layout.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OptionsKey {
    font_family: String,
    font_size: u32,
    font_weight: FontWeight,
    font_style: FontStyle,
    lang: Lang,
    region: Option<Region>,
    dir: u8,
    math_family: String,
    math_size: Option<u32>,
    math_weight: Option<FontWeight>,
    width: (u8, u32),
    wrap: bool,
    align: u8,
    line_height: (u8, u32),
    max_lines: Option<NonZeroUsize>,
    hanging_signs: bool,
    newline_breaks: bool,
    ellipsis: bool,
    limits: (usize, usize, usize),
}

impl OptionsKey {
    fn new(options: &LabelOptions) -> Self {
        // Destructured in full, so that a new option can't be left out of the key.
        let LabelOptions {
            text,
            math,
            width,
            wrap,
            align,
            line_height,
            max_lines,
            hanging_signs,
            newline_breaks,
            ellipsis,
            limits,
        } = options;
        let TextStyle {
            font_family,
            font_size,
            fill: _,
            font_weight,
            font_style,
            lang,
            region,
            dir,
        } = text;
        let MathStyle {
            font_family: math_family,
            font_size: math_size,
            fill: _,
            font_weight: math_weight,
        } = math;
        let LabelLimits { max_source_bytes, max_math_spans, max_math_depth } = *limits;
        Self {
            font_family: font_family.clone(),
            font_size: font_size.to_bits(),
            font_weight: *font_weight,
            font_style: *font_style,
            lang: *lang,
            region: *region,
            dir: *dir as u8,
            math_family: math_family.clone(),
            math_size: math_size.map(|Em(size)| size.to_bits()),
            math_weight: *math_weight,
            width: match *width {
                LabelWidth::Auto => (0, 0),
                LabelWidth::Max(width) => (1, width.to_bits()),
                LabelWidth::Fixed(width) => (2, width.to_bits()),
            },
            wrap: *wrap,
            align: *align as u8,
            line_height: match *line_height {
                LabelLineHeight::Auto => (0, 0),
                LabelLineHeight::Fixed(distance) => (1, distance.to_bits()),
                LabelLineHeight::Relative(multiple) => (2, multiple.to_bits()),
            },
            max_lines: *max_lines,
            hanging_signs: *hanging_signs,
            newline_breaks: *newline_breaks,
            ellipsis: *ellipsis,
            limits: (max_source_bytes, max_math_spans, max_math_depth),
        }
    }
}

/// A fill, with its components as bits.
#[cfg(feature = "raster")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ColorKey {
    components: [u32; 3],
    alpha: u32,
    space: u8,
}

#[cfg(feature = "raster")]
impl From<&AbsoluteColor> for ColorKey {
    fn from(color: &AbsoluteColor) -> Self {
        Self {
            components: color.components.map(f32::to_bits),
            alpha: color.alpha.to_bits(),
            space: color.color_space as u8,
        }
    }
}

/// A param's value in a key: equal to another when they are, with floats compared by their
/// bits and dictionaries' entries in order, as values hash.
#[derive(Debug, Clone)]
struct ParamKey(LabelParamValue);

impl PartialEq for ParamKey {
    fn eq(&self, other: &Self) -> bool {
        same_value(&self.0, &other.0)
    }
}

impl Eq for ParamKey {}

impl Hash for ParamKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

fn same_value(a: &LabelParamValue, b: &LabelParamValue) -> bool {
    match (a, b) {
        (LabelParamValue::Float(a), LabelParamValue::Float(b)) => {
            a.to_bits() == b.to_bits()
        }
        (LabelParamValue::Array(a), LabelParamValue::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_value(a, b))
        }
        (LabelParamValue::Dict(a), LabelParamValue::Dict(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|((key_a, a), (key_b, b))| key_a == key_b && same_value(a, b))
        }
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_keys_compare_floats_by_bits_and_entries_in_order() {
        let key = |value| ParamKey(value);
        let nan = LabelParamValue::Float(f64::NAN);
        assert_eq!(key(nan.clone()), key(nan));
        assert_ne!(key(LabelParamValue::Float(0.0)), key(LabelParamValue::Float(-0.0)));
        let dict = |entries: &[(&str, i64)]| {
            LabelParamValue::Dict(
                entries
                    .iter()
                    .map(|(name, value)| (name.to_string(), LabelParamValue::Int(*value)))
                    .collect(),
            )
        };
        assert_ne!(key(dict(&[("a", 1), ("b", 2)])), key(dict(&[("b", 2), ("a", 1)])));
    }
}
