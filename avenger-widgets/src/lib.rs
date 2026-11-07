//! Controls drawn by a [`Widgets`] tool in an avenger app, or on their own, such as in a static
//! image. The `app` feature, on by default, adds the tool.
//!
//! Value controls read and update app state through callbacks. Buttons run actions, and group
//! boxes frame other controls. The app measures widgets through [`Widgets::measure`] and stores
//! their rectangles in app state for the tool to read.
//!
//! The tool routes events to at most one widget, retains the owner of a press until release,
//! and moves keyboard focus with Tab. It reuses drawings while each widget's view, rectangle
//! and look are unchanged. Other crates can add widgets by implementing [`Widget`].

// Without the tool, parts of the widget collection that only the tool reads are unused.
#![cfg_attr(not(feature = "app"), allow(dead_code))]

mod binding;
mod button;
mod checkbox;
mod checkboxes;
mod draw;
mod group_box;
mod radio;
mod range_slider;
mod slider;
mod style;
mod text_input;
#[cfg(feature = "app")]
mod tool;
mod widget;
mod widgets;

pub use button::{Button, ButtonState};
pub use checkbox::{Checkbox, CheckboxState, CheckboxView};
pub use checkboxes::{CheckboxList, CheckboxListState, CheckboxListView};
pub use group_box::GroupBox;
pub use radio::{RadioDirection, RadioGroup, RadioGroupState, RadioGroupView};
pub use range_slider::{RangeSlider, RangeSliderState, RangeSliderView};
pub use slider::{Slider, SliderState, SliderView};
pub use style::{Paint, Palette, WidgetStyle};
pub use text_input::{TextInput, TextInputState, TextInputView};
#[cfg(feature = "app")]
pub use tool::WidgetsState;
pub use widget::{Look, Placed, Widget, WidgetContext, WidgetError, WidgetEvent};
pub use widgets::{WidgetMeasure, Widgets};
