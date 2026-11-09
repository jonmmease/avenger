//! The categories that a chart styles apart from their scale: the Other group of a lump, which
//! the `other` style nodes style, and the NULL category of a categorical scale, which the `null`
//! nodes style.

use arrow::{
    array::{Array, ArrayRef, BooleanArray},
    datatypes::DataType,
};
use avenger_chart::lump;
use avenger_style::NodeId;

/// A category that a style node styles apart from its scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Variant {
    /// The Other group of a lump.
    Other,
    /// The NULL category of a categorical scale.
    Null,
}

impl Variant {
    /// The variants in the order in which they apply, so a row that is NULL in one encoding and
    /// in the Other group in another takes the `null` node.
    pub(crate) const ALL: [Variant; 2] = [Variant::Other, Variant::Null];

    /// Whether each of `categories`, the values of a categorical scale or the keys of lumps, is
    /// in the variant: the Other group of lumps, or the NULL category, a NULL value or the NULL
    /// category of lumps. A NULL struct, such as a NULL path of nested bands or a NULL lump, is no
    /// category. `None` for categories of which none can be.
    pub(crate) fn mask(self, categories: &ArrayRef) -> Option<Vec<bool>> {
        let bools = |mask: BooleanArray| mask.iter().map(|is| is == Some(true)).collect();
        match self {
            Variant::Other => lump::others(categories).map(bools),
            Variant::Null => match lump::nulls(categories) {
                Some(nulls) => Some(bools(nulls)),
                None if matches!(categories.data_type(), DataType::Struct(_)) => None,
                None => Some(
                    (0..categories.len())
                        .map(|row| categories.is_null(row))
                        .collect(),
                ),
            },
        }
    }

    /// Whether some of `categories` are in the variant.
    pub(crate) fn holds(self, categories: &ArrayRef) -> bool {
        self.mask(categories)
            .is_some_and(|mask| mask.contains(&true))
    }
}

/// The variant nodes of a node: its `other`, and its `null`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Variants {
    pub(crate) other: Option<NodeId>,
    pub(crate) null: Option<NodeId>,
}

impl Variants {
    /// The node of `variant`.
    pub(crate) fn node(self, variant: Variant) -> Option<NodeId> {
        match variant {
            Variant::Other => self.other,
            Variant::Null => self.null,
        }
    }

    /// The nodes that the variants have, in the order in which they apply.
    pub(crate) fn nodes(self) -> impl Iterator<Item = (Variant, NodeId)> {
        (Variant::ALL.into_iter()).filter_map(move |variant| Some((variant, self.node(variant)?)))
    }
}
