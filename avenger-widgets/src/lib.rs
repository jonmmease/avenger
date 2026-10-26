//! Controls, such as buttons, check boxes, radio groups and sliders, that a [`Widgets`] tool draws
//! in its own scene of an [`AvengerApp`](avenger_app::app::AvengerApp).
//!
//! Widgets read their values from app state and report changes through closures that change app
//! state. The app's layout places them: it measures each widget through
//! [`Widgets::measure`], and the tool reads each widget's rectangle back from app state. The
//! tool routes pointer and key events to one widget at a time, holds a widget's press until the
//! release, moves keyboard focus with Tab, and draws a widget again only when what it draws from
//! changes. Other crates add widgets by implementing [`Widget`].

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
