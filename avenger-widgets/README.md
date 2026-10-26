# avenger-widgets

Controls for an `AvengerApp`: buttons, check boxes, radio groups, sliders and group boxes. A
`Widgets` tool draws a set of widgets in its own scene and routes pointer and key events to
them. Other crates add their own widgets by implementing the `Widget` trait, and their widgets
behave like the built-in ones.

Widgets keep no values. Each widget reads its value from app state through a closure and reports
a change through another closure, which changes app state and returns the scenes to rebuild. The
app's layout places the widgets, so controls and plots share one `avenger-layout` tree.

## Add widgets to an app

Build the widgets, keep a clone for measuring in the app's build, and install the original as a
tool:

```rust
let widgets = Widgets::new("controls", 20)
    .with(
        Slider::new("opacity", "Opacity", 0.0..=1.0)
            .value(|s: &State| s.opacity)
            .on_change(|s, opacity| {
                s.opacity = opacity;
                Rebuild::scene("chart")
            })
            .readout(".0%"),
    )
    .with(
        Checkbox::new("legend", "Show legend")
            .value(|s: &State| s.show_legend)
            .on_change(|s, show| {
                s.show_legend = show;
                Rebuild::tree("chart")
            }),
    )
    .place(|s: &State, id| s.controls.get(id).copied());

let app = AvengerApp::builder(state, Chart { widgets: widgets.clone() }, engine)
    .tool(widgets)
    .build()
    .await?;
```

The app's build lays out the widgets with the rest of the scene. Give each widget a leaf in the
layout, forward the leaf's measurement questions to `widgets.measure(engine)`, and store each
widget's solved rectangle where the `place` closure reads it:

```rust
impl Measure<Id> for ChartMeasure<'_> {
    fn height(&mut self, id: &Id, width: f32) -> Option<Measurement> {
        match id {
            Id::Widget(name) => self.widgets.height(name, width),
            _ => None,
        }
    }
    // `width` and `max_content` forward the same way.
}
```

A widget without a rectangle is absent: the tool doesn't draw it, route events to it, or give it
focus. Widgets draw in the order that they were added, so a later widget is on top for the
pointer, and Tab visits them in that order. The `iris` example puts a column of controls beside
its chart.

## How the tool handles input

The tool routes each event to at most one widget:

- A pointer event goes to the widget that holds the current press, or else to the topmost widget
  under the pointer. A press that begins on a widget belongs to it until the release, so a slider
  drag continues outside the slider. During a press that began elsewhere, such as a pan, no
  widget is hovered.
- A key event goes to the widget with keyboard focus. Tab and Shift+Tab move focus through the
  widgets that are placed, enabled and focusable. Past the last one, focus clears and the Tab goes
  on to the app. Pressing a widget focuses it, and pressing outside every widget clears focus.
  A focus ring shows only for focus from the keyboard.

A widget consumes its pointer events by default, so the scenes below it don't pan or show
tooltips there, and ignores keys outside its keyboard map, so the app's shortcuts still work.
The tool draws a widget again only when its view, rectangle, look or the style changes, so a pan
that rebuilds the chart on every frame redraws no widget. A handler that changes a value that
only a widget shows requests the `Widgets` scene.

## Lay out responsive widgets

A widget's measurements depend only on its configuration, the style and the space it gets, never
on app state, so a value change never moves the layout. A radio group in the automatic direction
answers with its row's height from the row's width up and its column's height below it, so in
an automatic track it becomes a column when the layout narrows it.

A group box draws a border with its title breaking the top edge, around widgets that the layout
places inside it. Build it as a container node with the group box's id and
`widgets.padding(id)` as its padding, and put a title leaf first, which the measurer answers as
the group box. Add the group box to the tool before the widgets inside it. A container nested in
another `Row` or `Column` needs `share(false)` to take padding:

```rust
Column::new([
    Layout::from(Leaf::new().id(Id::Title("points".into()))),
    Leaf::new().id(Id::Widget("opacity".into())).into(),
    Leaf::new().id(Id::Widget("size".into())).into(),
])
.id(Id::Widget("points".into()))
.padding(widgets.padding("points"))
.share(false)
```

## Built-in widgets

| Widget | Pointer | Keys |
| --- | --- | --- |
| `Button` | Activates on a left-button release inside it. | Enter activates on press, and Space on release. Escape cancels a pending press. A held key activates once. |
| `Checkbox` | Toggles on a left-button release inside it. | Space toggles on release. Escape cancels a pending press. |
| `RadioGroup` | Selects the item that a press began and ended on. | Arrow keys move the selection, and wrap. |
| `Slider` | A press on the track sets the value, and the drag follows it. | Arrow keys move one step, Page Up and Page Down ten steps, and Home and End select the ends. |
| `GroupBox` | Consumes presses inside it that miss its widgets. | None. It isn't a Tab stop. |

Each widget takes an optional `enabled` closure. A disabled widget keeps its place, draws dimmed,
receives no events and isn't a Tab stop.

A slider calls `on_change` for each new value and `on_commit` once a drag that changed the value
ends or a key that changed it is released. Escape during a drag, or a canceled drag, restores the
value at the press. With `step`, a slider admits its start, each step after it and its end. Its
optional readout formats the value with the label engine's number provider, as an axis's
`tick_format` does, and `readout_label` wraps it in Typst markup, as `tick_label` does. The readout
reserves the wider of its widths at the slider's start and end.

## Write a widget

Implement `Widget<State>`. The tool passes every widget the same `WidgetContext`, with the label
engine and the style, so a widget from another crate can do what a built-in one does.

- `view` collects everything that `draw` shows from app state and the widget's own state. The
  tool compares views to decide what to draw again, so `draw` must read app state only through
  the view.
- `max_content`, `height` and `width` answer the layout, as `avenger_layout::Measure` does, and
  must not depend on app state.
- `event` receives pointer and key events as `WidgetEvent::Input`, and `Focus`, `Blur` and
  `Cancel` from the tool. A widget that lost its rectangle or became disabled receives `Cancel`
  or `Blur` with the next event.
- `disposition` decides whether an input event is ignored, handled or consumed.
- `draw` can fail, as on invalid markup. The tool draws the other widgets and reports the first
  failure from its update.

## Style

`WidgetStyle` sets the font, colors, sizes and spacing. `WidgetStyle::default()` is light and
`WidgetStyle::dark()` is dark. Each palette holds a `Paint` for the normal, hovered, pressed and
disabled states, and `palette.paint(look)` picks one. The presets dim disabled widgets by blending
the normal colors toward the background, which keeps them opaque, so overlapping parts don't show
through each other.
