//! Controls drawn by a [`Widgets`] tool in an [`AvengerApp`](avenger_app::app::AvengerApp).
//!
//! Value controls read and update app state through callbacks. Buttons run actions, and group
//! boxes frame other controls. The app measures widgets through [`Widgets::measure`] and stores
//! their rectangles in app state for the tool to read.
//!
//! The tool routes events to at most one widget, retains the owner of a press until release,
//! and moves keyboard focus with Tab. It reuses drawings while each widget's view, rectangle
//! and look are unchanged. Other crates can add widgets by implementing [`Widget`].

mod binding;
mod button;
mod checkbox;
mod draw;
mod group_box;
mod radio;
mod slider;
mod style;
mod tool;
mod widget;
mod widgets;

pub use button::{Button, ButtonState};
pub use checkbox::{Checkbox, CheckboxState, CheckboxView};
pub use group_box::GroupBox;
pub use radio::{RadioDirection, RadioGroup, RadioGroupState, RadioGroupView};
pub use slider::{Slider, SliderState, SliderView};
pub use style::{Paint, Palette, WidgetStyle};
pub use tool::WidgetsState;
pub use widget::{Look, Placed, Widget, WidgetContext, WidgetError, WidgetEvent};
pub use widgets::{WidgetMeasure, Widgets};
