//! Scenegraph controls with caller-owned values and layout.
//!
//! Prepare descriptions, allocate rectangles, install their frame, and handle
//! typed events before preparing the next frame. See the crate README for the
//! application and host lifecycle.

#![doc = include_str!("../README.md")]

mod button;
mod checkbox;
mod choice;
mod frame;
mod identity;
mod paint;
mod runtime;
mod slider;
mod style;

pub use avenger_layout::{Edges, Rect, Size};
pub use button::{Button, ButtonVariant};
pub use checkbox::Checkbox;
pub use choice::{CheckboxGroup, ChoiceItem, ChoiceOrientation, RadioGroup};
pub use frame::{PreparedWidgets, WidgetFrame, WidgetMetrics};
pub use identity::{ChoiceItemId, WidgetId, WidgetTarget};
pub use runtime::{
    FocusBoundary, SemanticValue, WidgetAction, WidgetEvent, WidgetRole, WidgetRuntime,
    WidgetSemantic, WidgetUpdate,
};
pub use slider::{Slider, SliderCancelReason, SliderDomain};
pub use style::{
    ButtonStyle, CheckboxStyle, ChoiceGroupStyle, ControlPaint, FocusStyle, PaintStates,
    SliderStyle, TextStyle, WidgetTheme,
};

/// Invalid widget configuration, geometry, identity, or text layout.
#[derive(Debug, thiserror::Error)]
pub enum WidgetError {
    #[error("invalid widget configuration: {0}")]
    Invalid(String),
    #[error("widget frame was prepared from a different or superseded runtime")]
    StaleFrame,
    #[error(transparent)]
    Text(#[from] avenger_text::error::AvengerTextError),
}

/// A declarative control description with its current application value.
#[derive(Clone, Debug)]
pub enum WidgetSpec {
    Button(Button),
    Checkbox(Checkbox),
    CheckboxGroup(CheckboxGroup),
    RadioGroup(RadioGroup),
    Slider(Slider),
}

impl WidgetSpec {
    /// Stable identity supplied by the application.
    pub fn id(&self) -> &WidgetId {
        &self.options().id
    }
    pub(crate) fn options(&self) -> &Options {
        match self {
            Self::Button(v) => &v.options,
            Self::Checkbox(v) => &v.options,
            Self::CheckboxGroup(v) => &v.options,
            Self::RadioGroup(v) => &v.options,
            Self::Slider(v) => &v.options,
        }
    }
    pub(crate) fn label(&self) -> &str {
        match self {
            Self::Button(v) => &v.label,
            Self::Checkbox(v) => &v.label,
            Self::CheckboxGroup(v) => &v.label,
            Self::RadioGroup(v) => &v.label,
            Self::Slider(v) => v.value_label.as_deref().unwrap_or(""),
        }
    }
    pub(crate) fn items(&self) -> Option<&[ChoiceItem]> {
        match self {
            Self::CheckboxGroup(v) => Some(&v.items),
            Self::RadioGroup(v) => Some(&v.items),
            _ => None,
        }
    }
    pub(crate) fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Options {
    pub id: WidgetId,
    pub enabled: bool,
    pub semantic_name: Option<String>,
}
impl Options {
    fn new(id: impl Into<WidgetId>) -> Self {
        Self {
            id: id.into(),
            enabled: true,
            semantic_name: None,
        }
    }
}

/// Common imports for control descriptions, events, styles, and geometry.
pub mod prelude {
    pub use crate::{
        Button, ButtonVariant, Checkbox, CheckboxGroup, ChoiceItem, ChoiceItemId,
        ChoiceOrientation, Edges, FocusBoundary, PreparedWidgets, RadioGroup, Rect, Size, Slider,
        SliderCancelReason, SliderDomain, TextStyle, WidgetAction, WidgetError, WidgetEvent,
        WidgetFrame, WidgetId, WidgetMetrics, WidgetRuntime, WidgetSpec, WidgetTarget, WidgetTheme,
        WidgetUpdate,
    };
}
