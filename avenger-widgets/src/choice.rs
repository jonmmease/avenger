use crate::{ChoiceItemId, Options, WidgetError, WidgetId, WidgetSpec};
use std::collections::BTreeSet;

/// A stable group item. Labels and ordering can change without changing identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceItem {
    pub id: ChoiceItemId,
    pub label: String,
    pub enabled: bool,
}
impl ChoiceItem {
    pub fn new(id: impl Into<ChoiceItemId>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            enabled: true,
        }
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}
/// Internal row arrangement. Groups do not wrap or run an outer layout solver.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChoiceOrientation {
    #[default]
    Vertical,
    Horizontal,
}

/// Independently checked rows backed by a caller-owned set of item IDs.
#[derive(Clone, Debug)]
pub struct CheckboxGroup {
    pub(crate) options: Options,
    pub(crate) items: Vec<ChoiceItem>,
    pub(crate) checked: BTreeSet<ChoiceItemId>,
    pub(crate) label: String,
    pub(crate) orientation: ChoiceOrientation,
}
impl CheckboxGroup {
    pub fn new(
        id: impl Into<WidgetId>,
        items: impl IntoIterator<Item = ChoiceItem>,
        checked: impl IntoIterator<Item = ChoiceItemId>,
    ) -> Self {
        Self {
            options: Options::new(id),
            items: items.into_iter().collect(),
            checked: checked.into_iter().collect(),
            label: String::new(),
            orientation: ChoiceOrientation::Vertical,
        }
    }
}
/// A single choice with one Tab stop and arrow-key navigation between enabled rows.
#[derive(Clone, Debug)]
pub struct RadioGroup {
    pub(crate) options: Options,
    pub(crate) items: Vec<ChoiceItem>,
    pub(crate) selected: Option<ChoiceItemId>,
    pub(crate) label: String,
    pub(crate) orientation: ChoiceOrientation,
}
impl RadioGroup {
    pub fn new(
        id: impl Into<WidgetId>,
        items: impl IntoIterator<Item = ChoiceItem>,
        selected: Option<ChoiceItemId>,
    ) -> Self {
        Self {
            options: Options::new(id),
            items: items.into_iter().collect(),
            selected,
            label: String::new(),
            orientation: ChoiceOrientation::Vertical,
        }
    }
}
macro_rules! group_builders {
    ($name:ident,$variant:ident) => {
        impl $name {
            pub fn enabled(mut self, enabled: bool) -> Self {
                self.options.enabled = enabled;
                self
            }
            pub fn semantic_name(mut self, name: impl Into<String>) -> Self {
                self.options.semantic_name = Some(name.into());
                self
            }
            pub fn label(mut self, label: impl Into<String>) -> Self {
                self.label = label.into();
                self
            }
            pub fn orientation(mut self, orientation: ChoiceOrientation) -> Self {
                self.orientation = orientation;
                self
            }
        }
        impl From<$name> for WidgetSpec {
            fn from(value: $name) -> Self {
                Self::$variant(value)
            }
        }
    };
}
group_builders!(CheckboxGroup, CheckboxGroup);
group_builders!(RadioGroup, RadioGroup);

pub(crate) fn validate(
    items: &[ChoiceItem],
    selected: impl Iterator<Item = ChoiceItemId>,
) -> Result<(), WidgetError> {
    let mut ids = BTreeSet::new();
    for item in items {
        if !ids.insert(item.id.clone()) {
            return Err(WidgetError::Invalid(format!(
                "duplicate choice item {}",
                item.id.as_str()
            )));
        }
    }
    for id in selected {
        if !ids.contains(&id) {
            return Err(WidgetError::Invalid(format!(
                "selected choice item {} does not exist",
                id.as_str()
            )));
        }
    }
    Ok(())
}
