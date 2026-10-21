//! What tooltips look for: one instance's row, or a row for each series at the pointer's x.

use avenger_geometry::data::Datum;
use avenger_geometry::focus::{Focus, FocusStrategy};

use crate::draw::TooltipField;
use crate::error::AvengerTooltipError;

/// Where a tooltip stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TooltipAnchor {
    /// Beside the focus's data, with a callout to it, so it stays still while the pointer moves
    /// within the focus.
    #[default]
    Data,
    /// Below and right of the pointer, as Vega's tooltips stand, so the pointer stays in view.
    /// It follows the pointer within the focus, over an outline of the data that no callout
    /// points at, for marks without one point to point at, such as areas and map regions.
    Pointer,
}

/// What tooltips look for, which they try in order.
#[derive(Debug, Clone, PartialEq)]
pub enum TooltipTarget {
    Instance(InstanceTarget),
    Group(GroupTarget),
}

impl TooltipTarget {
    pub(crate) fn anchor(&self) -> TooltipAnchor {
        match self {
            Self::Instance(target) => target.anchor,
            Self::Group(target) => target.anchor,
        }
    }

    pub(crate) fn key(&self) -> Option<&str> {
        match self {
            Self::Instance(target) => target.key.as_deref(),
            Self::Group(target) => target.key.as_deref(),
        }
    }
}

impl From<InstanceTarget> for TooltipTarget {
    fn from(target: InstanceTarget) -> Self {
        Self::Instance(target)
    }
}

impl From<GroupTarget> for TooltipTarget {
    fn from(target: GroupTarget) -> Self {
        Self::Group(target)
    }
}

/// A target of one instance, such as a point, a bar, or a map region: its tooltip has a header
/// and a row for each field of the instance's row.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceTarget {
    pub(crate) strategy: FocusStrategy,
    pub(crate) marks: Option<Vec<String>>,
    pub(crate) anchor: TooltipAnchor,
    pub(crate) header: Option<TooltipField>,
    pub(crate) fields: Option<Vec<TooltipField>>,
    pub(crate) key: Option<String>,
}

impl Default for InstanceTarget {
    /// The nearest instance within 40 pixels of the pointer, among every mark with rows.
    fn default() -> Self {
        Self::new(FocusStrategy::default())
    }
}

impl InstanceTarget {
    /// A target of the instance that `strategy` finds among every mark with rows, beside its
    /// data, with no header and a row for each of its row's columns.
    ///
    /// # Panics
    ///
    /// When `strategy` is [`FocusStrategy::GroupX`], which finds a group: a [`GroupTarget`]
    /// shows one.
    pub fn new(strategy: FocusStrategy) -> Self {
        assert!(
            !matches!(strategy, FocusStrategy::GroupX { .. }),
            "an instance target finds one instance: a GroupTarget shows a group"
        );
        Self {
            strategy,
            marks: None,
            anchor: TooltipAnchor::default(),
            header: None,
            fields: None,
            key: None,
        }
    }

    /// The target, searching only the marks named `marks`.
    pub fn with_marks(mut self, marks: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.marks = Some(marks.into_iter().map(Into::into).collect());
        self
    }

    pub fn with_anchor(mut self, anchor: TooltipAnchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// The target, headed by `field`'s value, after its title when it has one.
    pub fn with_header(mut self, field: impl Into<TooltipField>) -> Self {
        self.header = Some(field.into());
        self
    }

    /// The target, with a row for each of `fields`, in order, in place of a row for each column
    /// but the header's.
    pub fn with_fields(
        mut self,
        fields: impl IntoIterator<Item = impl Into<TooltipField>>,
    ) -> Self {
        self.fields = Some(fields.into_iter().map(Into::into).collect());
        self
    }

    /// The target, whose pinned instance follows its value in the `key` column, such as an id,
    /// when the rows change. Without a key, a pinned instance hides when its mark's rows
    /// change, since its index may hold another row.
    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The fields of `datum`'s row: the target's, or every column but the header's.
    pub(crate) fn fields_of(&self, datum: &Datum) -> Vec<TooltipField> {
        if let Some(fields) = &self.fields {
            return fields.clone();
        }
        other_columns(datum, self.header.as_ref())
            .map(TooltipField::new)
            .collect()
    }
}

/// The order of a group's rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GroupOrder {
    /// The order of the target's entries, as a legend of the same entries lists them, so each
    /// series' row keeps its place while the pointer moves.
    #[default]
    Entries,
    /// Down the scene, as the series lie under the crosshair: with a y scale that points up, the
    /// highest value first. Two series swap rows where they cross.
    Position,
}

/// A target of a group: each series' instance nearest the pointer's x, such as each line's
/// vertex in a time series. Its tooltip has a header from the first series' row, and a row for
/// each series, with its swatch, its title, and its value. A crosshair stands at the group's x,
/// with a marker on each series.
///
/// Its entries are those of the series' legend: one mark per series, each with its title, in the
/// legend's order, as a line legend's `text` and swatches.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupTarget {
    pub(crate) marks: Vec<String>,
    pub(crate) titles: Vec<String>,
    pub(crate) max_radius: f32,
    pub(crate) limit: Option<usize>,
    pub(crate) order: GroupOrder,
    pub(crate) anchor: TooltipAnchor,
    pub(crate) header: Option<TooltipField>,
    pub(crate) value: Option<TooltipField>,
    pub(crate) key: Option<String>,
}

impl GroupTarget {
    /// A target of the series drawn by the marks named `marks`, titled by `titles`, one for each
    /// mark, within 40 pixels of the pointer along x, beside the crosshair, in the entries' order,
    /// with no header and each series' value in its first column. An error when `marks` and
    /// `titles` differ in length.
    pub fn new(
        marks: impl IntoIterator<Item = impl Into<String>>,
        titles: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, AvengerTooltipError> {
        let marks: Vec<String> = marks.into_iter().map(Into::into).collect();
        let titles: Vec<String> = titles.into_iter().map(Into::into).collect();
        if marks.len() != titles.len() {
            return Err(AvengerTooltipError::Entries {
                marks: marks.len(),
                titles: titles.len(),
            });
        }
        Ok(Self {
            marks,
            titles,
            max_radius: 40.0,
            limit: None,
            order: GroupOrder::default(),
            anchor: TooltipAnchor::default(),
            header: None,
            value: None,
            key: None,
        })
    }

    /// The target, of the series with an instance within `max_radius` pixels of the pointer
    /// along x.
    pub fn with_max_radius(mut self, max_radius: f32) -> Self {
        self.max_radius = max_radius;
        self
    }

    /// The target, of the `limit` series nearest the pointer along y. A shown series keeps its
    /// place until another is nearer by more than a pixel, so rows don't flicker between series
    /// that are about as near.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_order(mut self, order: GroupOrder) -> Self {
        self.order = order;
        self
    }

    pub fn with_anchor(mut self, anchor: TooltipAnchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// The target, headed by `field`'s value in the first series' row, after its title when it
    /// has one.
    pub fn with_header(mut self, field: impl Into<TooltipField>) -> Self {
        self.header = Some(field.into());
        self
    }

    /// The target, with each series' value in `field`, in place of its first column but the
    /// header's.
    pub fn with_value(mut self, field: impl Into<TooltipField>) -> Self {
        self.value = Some(field.into());
        self
    }

    /// The target, whose pinned series' instances follow their values in the `key` column,
    /// such as each series' step, when the rows change. Without a key, a pinned group hides
    /// when a series' rows change, since an index may hold another row.
    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The strategy that finds the group.
    pub(crate) fn strategy(&self) -> FocusStrategy {
        FocusStrategy::GroupX {
            max_radius: self.max_radius,
            limit: self.limit,
        }
    }

    /// The field of `datum`'s row that holds its series' value: the target's, or its first
    /// column but the header's.
    pub(crate) fn value_of(&self, datum: &Datum) -> Option<TooltipField> {
        if let Some(value) = &self.value {
            return Some(value.clone());
        }
        other_columns(datum, self.header.as_ref())
            .next()
            .map(TooltipField::new)
    }

    /// The title of the series that the mark named `mark` draws, or the mark's name when it
    /// isn't one of the target's.
    pub(crate) fn title_of(&self, mark: &str) -> String {
        self.entry_of(mark)
            .map_or_else(|| mark.to_string(), |entry| self.titles[entry].clone())
    }

    /// Put `focus`'s members in the target's order.
    pub(crate) fn order(&self, focus: &mut Focus) {
        if self.order == GroupOrder::Entries {
            focus
                .members
                .sort_by_key(|member| self.entry_of(&member.mark().name));
        }
    }

    fn entry_of(&self, mark: &str) -> Option<usize> {
        self.marks.iter().position(|name| name == mark)
    }
}

/// The names of `datum`'s columns but `header`'s.
fn other_columns<'a>(
    datum: &'a Datum,
    header: Option<&'a TooltipField>,
) -> impl Iterator<Item = &'a str> {
    datum
        .rows
        .schema_ref()
        .fields()
        .iter()
        .map(|field| field.name().as_str())
        .filter(move |name| header.is_none_or(|header| header.column != *name))
}
