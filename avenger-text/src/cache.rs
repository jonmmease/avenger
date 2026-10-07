use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{Arc, Mutex},
};

use avenger_typst_label::{LabelParamValue, LabelParams};

use crate::{
    types::{FontStyle, TextConfig, TextLayout, TextSyntaxMode},
    typeset::typst_font_weight,
    DateTimeFormatProvider, NumberFormatProvider, ProviderIdentity,
};

/// What sets a label's layout: labels with equal keys typeset alike.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct LabelKey {
    text: String,
    syntax_mode: TextSyntaxMode,
    font: String,
    font_size: u32,
    font_weight: u16,
    font_style: FontStyle,
    layout: TextLayout,
    params: ParamsKey,
    number_format: Option<ProviderIdentity<dyn NumberFormatProvider>>,
    datetime_format: Option<ProviderIdentity<dyn DateTimeFormatProvider>>,
}

impl LabelKey {
    /// The key of a label, with the engine's formatting providers for labels that bring none.
    pub(crate) fn new(
        config: &TextConfig,
        number_format: Option<&Arc<dyn NumberFormatProvider>>,
        datetime_format: Option<&Arc<dyn DateTimeFormatProvider>>,
    ) -> Self {
        Self {
            text: config.text.to_string(),
            syntax_mode: config.syntax_mode,
            font: config.font.to_string(),
            font_size: config.font_size.to_bits(),
            font_weight: typst_font_weight(config.font_weight).to_number(),
            font_style: config.font_style,
            layout: config.layout,
            params: ParamsKey(config.params.clone()),
            number_format: config
                .number_format
                .or(number_format)
                .map(ProviderIdentity::new),
            datetime_format: config
                .datetime_format
                .or(datetime_format)
                .map(ProviderIdentity::new),
        }
    }
}

/// A label's parameters in a key: equal when their entries are, in order, with floats compared
/// by their bits, as the label crate hashes them.
#[derive(Debug, Clone)]
struct ParamsKey(LabelParams);

impl PartialEq for ParamsKey {
    fn eq(&self, other: &Self) -> bool {
        same_entries(self.0.iter(), other.0.iter())
    }
}

impl Eq for ParamsKey {}

impl Hash for ParamsKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for (key, value) in &self.0 {
            key.hash(state);
            value.hash(state);
        }
    }
}

fn same_entries<'a>(
    a: impl ExactSizeIterator<Item = (&'a String, &'a LabelParamValue)>,
    b: impl ExactSizeIterator<Item = (&'a String, &'a LabelParamValue)>,
) -> bool {
    a.len() == b.len()
        && a.zip(b)
            .all(|((key_a, a), (key_b, b))| key_a == key_b && same_value(a, b))
}

fn same_value(a: &LabelParamValue, b: &LabelParamValue) -> bool {
    match (a, b) {
        (LabelParamValue::Float(a), LabelParamValue::Float(b)) => a.to_bits() == b.to_bits(),
        (LabelParamValue::Array(a), LabelParamValue::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_value(a, b))
        }
        (LabelParamValue::Dict(a), LabelParamValue::Dict(b)) => same_entries(a.iter(), b.iter()),
        _ => a == b,
    }
}

/// Memoized values, shared by the engine's clones. A full memo empties: labels repeat from
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
        Self {
            entries: Default::default(),
            capacity,
        }
    }

    /// The value for a key, made and remembered if the memo has none.
    pub(crate) fn get_or_try_insert<E>(
        &self,
        key: K,
        make: impl FnOnce() -> Result<V, E>,
    ) -> Result<V, E> {
        if let Some(value) = self.lock().get(&key) {
            return Ok(value.clone());
        }
        let value = make()?;
        let mut entries = self.lock();
        if entries.len() >= self.capacity {
            entries.clear();
        }
        entries.insert(key, value.clone());
        Ok(value)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<K, V>> {
        self.entries.lock().expect("text memo lock poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_keys_compare_floats_by_bits_and_entries_in_order() {
        let key = |entries: &[(&str, LabelParamValue)]| {
            ParamsKey(
                entries
                    .iter()
                    .map(|(key, value)| (key.to_string(), value.clone()))
                    .collect(),
            )
        };
        let nan = LabelParamValue::Float(f64::NAN);
        assert_eq!(key(&[("a", nan.clone())]), key(&[("a", nan)]));
        assert_ne!(
            key(&[("a", LabelParamValue::Float(0.0))]),
            key(&[("a", LabelParamValue::Float(-0.0))])
        );
        let (one, two) = (LabelParamValue::Int(1), LabelParamValue::Int(2));
        assert_ne!(
            key(&[("a", one.clone()), ("b", two.clone())]),
            key(&[("b", two), ("a", one)])
        );
    }
}
