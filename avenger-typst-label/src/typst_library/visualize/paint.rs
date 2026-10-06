//! Ported from crates/typst-library/src/visualize/paint.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: labels paint with solid colors only, so there are no gradients or tilings.

use std::fmt::{self, Debug, Formatter};

use ecow::EcoString;

use crate::typst_library::foundations::{Repr, cast};
use crate::typst_library::visualize::Color;

/// How a fill or stroke should be painted.
#[derive(Clone, PartialEq)]
pub enum Paint {
    /// A solid color.
    Solid(Color),
}

impl Paint {
    /// Unwraps a solid color used for text rendering.
    pub fn unwrap_solid(&self) -> Color {
        match self {
            Self::Solid(color) => *color,
        }
    }

    // avenger: no `relative`; only gradients and tilings have a coordinate system.

    /// Turns this paint into a paint for a text decoration.
    ///
    /// If this paint is a gradient, it will be converted to a gradient with
    /// relative set to [`RelativeTo::Parent`].
    pub fn as_decoration(&self) -> Self {
        match self {
            Self::Solid(color) => Self::Solid(*color),
        }
    }
}

impl Debug for Paint {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::Solid(v) => v.fmt(f),
        }
    }
}

impl Repr for Paint {
    fn repr(&self) -> EcoString {
        match self {
            Self::Solid(color) => color.repr(),
        }
    }
}

impl<T: Into<Color>> From<T> for Paint {
    fn from(t: T) -> Self {
        Self::Solid(t.into())
    }
}

cast! {
    Paint,
    self => match self {
        Self::Solid(color) => color.into_value(),
    },
    color: Color => Self::Solid(color),
}
